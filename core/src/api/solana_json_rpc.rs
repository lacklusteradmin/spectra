//! The Solana JSON-RPC adapter: balances, SPL token accounts, signatures
//! history, blockhashes, fees and raw transaction broadcast.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::HttpClient;

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct SolanaBalance {
    /// Lamports (1 SOL = 1_000_000_000 lamports).
    pub lamports: u64,
}

/// Unified history entry covering both native SOL and SPL token transfers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaTransfer {
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub fee_lamports: u64,
    pub is_incoming: bool,
    /// Human-readable amount ("1.5", "0.001", …).
    pub amount_display: String,
    /// "SOL" for native, mint address for SPL token transfers.
    pub symbol: String,
    /// Empty string for native SOL; mint address for SPL.
    pub mint: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaSendResult {
    pub signature: String,
    #[serde(default)]
    pub signed_tx_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplBalance {
    pub mint: String,
    pub owner: String,
    pub balance_raw: String,
    pub balance_display: String,
    pub decimals: u8,
    /// Best-effort symbol. Solana token symbols live in Metaplex metadata PDAs
    /// which we don't resolve yet; this is an empty string for now.
    pub symbol: String,
}

// ── Solana client

pub struct SolanaClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl SolanaClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        crate::api::json_rpc::call(
            crate::EndpointApi::SolanaJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }
}

// Solana fetch paths: native balance, SPL balances, recent blockhash,
// unified history, account existence.

impl SolanaClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<SolanaBalance, String> {
        let result = self
            .call("getBalance", json!([address, {"commitment": "confirmed"}]))
            .await?;
        let lamports = result
            .get("value")
            .and_then(|v| v.as_u64())
            .ok_or("getBalance: missing value")?;
        Ok(SolanaBalance { lamports })
    }

    /// Fetch SPL token balances for a list of mint addresses.
    /// Every SPL token account the owner holds, in one call.
    ///
    /// `getTokenAccountsByOwner` filtered by `programId` rather than by mint
    /// returns the lot, and the parsed account carries the mint's own
    /// `decimals` — so discovery answers "what does this address hold" and
    /// "how is it denominated" together, without a catalog and without an
    /// indexer.
    pub(crate) async fn fetch_transfer_mint(&self, mint: &str) -> Result<TransferMint, String> {
        crate::derivation::solana::decode_b58_32(mint)?;
        let result = self
            .call(
                "getAccountInfo",
                json!([mint, {"encoding":"jsonParsed", "commitment":"confirmed"}]),
            )
            .await?;
        validate_transfer_mint(&result["value"])
    }

    pub async fn fetch_all_spl_balances(&self, owner: &str) -> Result<Vec<SplBalance>, String> {
        const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
        const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
        let mut out: Vec<SplBalance> = Vec::new();
        for program in [TOKEN_PROGRAM, TOKEN_2022_PROGRAM] {
            // A program that will not answer is not a program the owner holds
            // nothing under, and skipping it would report the difference as an
            // empty wallet.
            let val = self
                .call(
                    "getTokenAccountsByOwner",
                    json!([
                        owner,
                        {"programId": program},
                        {"encoding": "jsonParsed", "commitment": "confirmed"}
                    ]),
                )
                .await?;
            let Some(accounts) = val.get("value").and_then(|v| v.as_array()) else {
                continue;
            };
            for account in accounts {
                let Some(info) = account.pointer("/account/data/parsed/info") else {
                    continue;
                };
                let Some(mint) = info.get("mint").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(token_amount) = info.get("tokenAmount") else {
                    continue;
                };
                let raw: u128 = token_amount
                    .get("amount")
                    .and_then(|v| v.as_str())
                    .ok_or("SPL token account has no amount")?
                    .parse()
                    .map_err(|_| "SPL token account amount is not an integer")?;
                // A closed or emptied account is not a holding.
                if raw == 0 {
                    continue;
                }
                let decimals = token_amount
                    .get("decimals")
                    .and_then(|v| v.as_u64())
                    .and_then(|d| u8::try_from(d).ok())
                    .ok_or("SPL token account has no decimals")?;
                // One mint can have several accounts; sum them.
                let balance = |raw: u128| SplBalance {
                    mint: mint.to_string(),
                    owner: owner.to_string(),
                    balance_raw: raw.to_string(),
                    balance_display: crate::decimal::from_units(raw, u32::from(decimals)),
                    decimals,
                    symbol: String::new(),
                };
                match out.iter_mut().find(|b| b.mint == mint) {
                    Some(existing) => {
                        let sum = existing
                            .balance_raw
                            .parse::<u128>()
                            .ok()
                            .and_then(|a| a.checked_add(raw))
                            .ok_or("SPL balance overflow")?;
                        *existing = balance(sum);
                    }
                    None => out.push(balance(raw)),
                }
            }
        }
        Ok(out)
    }

    pub async fn fetch_spl_balances(
        &self,
        owner: &str,
        mints: &[String],
    ) -> Result<Vec<SplBalance>, String> {
        use futures::future::join_all;
        let futs: Vec<_> = mints
            .iter()
            .map(|mint| {
                let owner = owner.to_string();
                let mint = mint.clone();
                let client = Self {
                    endpoints: self.endpoints.clone(),
                    client: self.client.clone(),
                };
                async move {
                    let result = client
                        .call(
                            "getTokenAccountsByOwner",
                            json!([
                                owner,
                                {"mint": mint},
                                {"encoding": "jsonParsed", "commitment": "confirmed"}
                            ]),
                        )
                        .await?;
                    let accounts = result
                        .get("value")
                        .and_then(|v| v.as_array())
                        .ok_or("getTokenAccountsByOwner: missing account list")?;
                    if accounts.is_empty() {
                        return Ok::<_, String>(None);
                    }
                    let mut raw = 0u128;
                    let mut own_decimals = None;
                    for account in accounts {
                        let amount = account
                            .pointer("/account/data/parsed/info/tokenAmount")
                            .ok_or("SPL account: missing tokenAmount")?;
                        let value: u64 = amount
                            .get("amount")
                            .and_then(|v| v.as_str())
                            .ok_or("SPL account: missing amount")?
                            .parse()
                            .map_err(|_| "SPL account: invalid amount")?;
                        let decimals = crate::api::checked_token_decimals(u128::from(
                            amount
                                .get("decimals")
                                .and_then(|v| v.as_u64())
                                .ok_or("SPL account: missing decimals")?,
                        ))?;
                        if own_decimals.is_some_and(|previous| previous != decimals) {
                            return Err("SPL accounts disagree on mint decimals".into());
                        }
                        own_decimals = Some(decimals);
                        raw = raw
                            .checked_add(u128::from(value))
                            .ok_or("SPL balance overflow")?;
                    }
                    let decimals = own_decimals.ok_or("SPL mint decimals unavailable")?;
                    Ok(Some(SplBalance {
                        mint,
                        owner,
                        balance_raw: raw.to_string(),
                        balance_display: crate::decimal::from_units(raw, u32::from(decimals)),
                        decimals,
                        symbol: String::new(),
                    }))
                }
            })
            .collect();

        let results = join_all(futs).await;
        Ok(results
            .into_iter()
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    pub async fn fetch_recent_blockhash(&self) -> Result<String, String> {
        let result = self
            .call("getLatestBlockhash", json!([{"commitment": "confirmed"}]))
            .await?;
        result
            .get("value")
            .and_then(|v| v.get("blockhash"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "getLatestBlockhash: missing blockhash".to_string())
    }

    /// Fetch up to `limit` recent transfers as unified entries covering both
    /// native SOL and SPL token transfers.
    pub async fn fetch_unified_history(
        &self,
        address: &str,
        limit: usize,
    ) -> Result<Vec<SolanaTransfer>, String> {
        // 1. Get signatures.
        let sigs_result = self
            .call(
                "getSignaturesForAddress",
                json!([address, {"limit": limit, "commitment": "confirmed"}]),
            )
            .await?;
        let sig_array = sigs_result
            .as_array()
            .ok_or("getSignaturesForAddress: expected array")?;

        let signatures: Vec<String> = sig_array
            .iter()
            .filter_map(|s| {
                s.get("signature")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();

        if signatures.is_empty() {
            return Ok(vec![]);
        }

        // 2. Fetch each transaction and build unified entries.
        let mut result: Vec<SolanaTransfer> = Vec::new();
        for sig in &signatures {
            let tx = self
                .call(
                    "getTransaction",
                    json!([sig, {"encoding": "json", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}]),
                )
                .await
                .unwrap_or(Value::Null);
            if !tx.is_null() {
                result.extend(solana_transfers_in_transaction(&tx, sig, address));
            }
        }
        Ok(result)
    }
}

/// What one transaction moved into or out of `address`: an entry per SPL
/// mint whose balance changed and one for SOL if it did, fee excluded.
///
/// Balances are indexed by the static account keys followed by the addresses
/// a version-0 transaction loads from lookup tables, writable then read-only.
/// Reading the static keys alone missed an address loaded from a table and
/// reported its transfer as 0 SOL. A transaction that changed nothing for the
/// address but its fee — a program interaction, a memo — yields no entry.
fn solana_transfers_in_transaction(tx: &Value, sig: &str, address: &str) -> Vec<SolanaTransfer> {
    let slot = tx.get("slot").and_then(Value::as_u64).unwrap_or(0);
    let timestamp = tx.get("blockTime").and_then(Value::as_i64);
    let fee = tx.pointer("/meta/fee").and_then(Value::as_u64).unwrap_or(0);
    let keys = |pointer: &str| -> Vec<String> {
        tx.pointer(pointer)
            .and_then(Value::as_array)
            .map(|keys| {
                keys.iter()
                    .filter_map(|key| key.as_str().or_else(|| key.get("pubkey")?.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let accounts: Vec<String> = [
        "/transaction/message/accountKeys",
        "/meta/loadedAddresses/writable",
        "/meta/loadedAddresses/readonly",
    ]
    .iter()
    .flat_map(|pointer| keys(pointer))
    .collect();
    let from = accounts.first().cloned().unwrap_or_default();
    let to = accounts.get(1).cloned().unwrap_or_default();
    let transfer = |is_incoming: bool, amount_display: String, mint: String| SolanaTransfer {
        signature: sig.to_string(),
        slot,
        timestamp,
        fee_lamports: fee,
        is_incoming,
        amount_display,
        symbol: if mint.is_empty() {
            "SOL".to_string()
        } else {
            mint.clone()
        },
        mint,
        from: from.clone(),
        to: to.clone(),
    };

    let mut result = Vec::new();

    // SPL: every token account the address owns, before or after — an
    // account closed by the transaction appears only before.
    // accountIndex -> (mint, decimals, pre, post)
    let mut tokens: std::collections::BTreeMap<u64, (String, u32, u128, u128)> =
        std::collections::BTreeMap::new();
    for (pointer, is_post) in [
        ("/meta/preTokenBalances", false),
        ("/meta/postTokenBalances", true),
    ] {
        for entry in tx
            .pointer(pointer)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if entry.get("owner").and_then(Value::as_str) != Some(address) {
                continue;
            }
            // The mint is the token's identity. A row without one names no
            // asset, and is left out rather than filed under a guess.
            let Some(mint) = entry
                .get("mint")
                .and_then(Value::as_str)
                .filter(|mint| !mint.is_empty())
            else {
                continue;
            };
            let Some(index) = entry.get("accountIndex").and_then(Value::as_u64) else {
                continue;
            };
            let raw: u128 = entry
                .pointer("/uiTokenAmount/amount")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let decimals = entry
                .pointer("/uiTokenAmount/decimals")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let slot = tokens
                .entry(index)
                .or_insert_with(|| (mint.to_string(), decimals, 0, 0));
            if is_post {
                slot.3 = raw;
            } else {
                slot.2 = raw;
            }
        }
    }
    for (mint, decimals, pre, post) in tokens.into_values() {
        if pre == post {
            continue;
        }
        let delta = post.abs_diff(pre);
        result.push(transfer(
            post > pre,
            crate::decimal::from_units(delta, decimals),
            mint,
        ));
    }

    // SOL. The fee payer is the first account; its fee is not a transfer.
    if let Some(index) = accounts.iter().position(|a| a == address) {
        let balance = |pointer: &str| {
            tx.pointer(pointer)
                .and_then(|balances| balances.get(index))
                .and_then(Value::as_u64)
        };
        if let (Some(pre), Some(post)) =
            (balance("/meta/preBalances"), balance("/meta/postBalances"))
        {
            let fee_paid = if index == 0 { fee } else { 0 };
            let delta = i128::from(post) - i128::from(pre) + i128::from(fee_paid);
            if delta != 0 {
                result.push(transfer(
                    delta > 0,
                    crate::decimal::from_units(delta.unsigned_abs(), 9),
                    String::new(),
                ));
            }
        }
    }
    result
}

impl SolanaClient {
    /// Broadcast an already-signed transaction given as a base64 string.
    pub async fn broadcast_raw(&self, signed_tx_base64: &str) -> Result<SolanaSendResult, String> {
        let result = self
            .call(
                "sendTransaction",
                json!([signed_tx_base64, {"encoding": "base64", "preflightCommitment": "confirmed"}]),
            )
            .await?;
        let signature = result
            .as_str()
            .ok_or("sendTransaction: expected string")?
            .to_string();
        Ok(SolanaSendResult {
            signature,
            signed_tx_base64: signed_tx_base64.to_string(),
        })
    }
}

#[cfg(test)]
mod balance_read_tests {
    use super::*;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

    #[tokio::test]
    async fn spl_empty_accounts_are_zero_but_malformed_accounts_are_errors() {
        fn account(raw: &str, decimals: u64) -> Value {
            json!({"account":{"data":{"parsed":{"info":{"tokenAmount":{"amount":raw,"decimals":decimals}}}}}})
        }
        for (accounts, expected) in [
            (json!([]), Some(None)),
            (
                json!([account("10", 6), account("20", 6)]),
                Some(Some("30")),
            ),
            (json!([{}]), None),
            (json!([account("bad", 6)]), None),
            (json!([account("1", 6), account("1", 9)]), None),
            (json!([account("1", 39)]), None),
        ] {
            let server = MockServer::start().await;
            Mock::given(any())
                .respond_with(move |req: &Request| {
                    let body: Value = req.body_json().unwrap();
                    ResponseTemplate::new(200).set_body_json(
                        json!({"jsonrpc":"2.0","id":body["id"],"result":{"value":accounts}}),
                    )
                })
                .mount(&server)
                .await;
            let client = SolanaClient::new(std::sync::Arc::new(vec![server.uri()]));
            let result = client.fetch_spl_balances("owner", &["mint".into()]).await;
            match expected {
                None => assert!(result.is_err()),
                Some(None) => assert!(result.unwrap().is_empty()),
                Some(Some(raw)) => assert_eq!(result.unwrap()[0].balance_raw, raw),
            }
        }
    }
}

/// What a transfer needs to know about its mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransferMint {
    pub program: [u8; 32],
    pub decimals: u8,
    /// The mint has a transfer-fee extension, so the transfer states the fee
    /// it expects (always zero; see [`validate_transfer_mint`]).
    pub transfer_fee_extension: bool,
}

/// Refuse unknown programs and extensions before signing.
///
/// Token-2022 extensions are allowed by name, each for a reason it leaves a
/// plain `TransferChecked` from the owner meaning what it says:
///
/// - metadata, group and member pointers and records only describe the mint;
/// - a mint close authority acts only on a mint with no supply;
/// - a permanent delegate can move the owner's tokens, but whether the owner
///   sends does not change that;
/// - confidential transfer configs govern encrypted balances, not this one;
/// - a transfer hook with no program runs nothing;
/// - a transfer fee that charges nothing in either its current or its
///   scheduled config withholds nothing. The transfer then asserts a zero
///   fee on chain, so a fee raised between review and landing fails the
///   transaction instead of withholding from the recipient.
///
/// Anything else — a hook program, a fee, non-transferable tokens, frozen
/// default accounts, interest or scaled amounts, pausing, or an extension
/// this list has never heard of — changes what arrives or needs accounts the
/// transfer does not supply, so it is refused.
fn validate_transfer_mint(account: &serde_json::Value) -> Result<TransferMint, String> {
    let owner = account["owner"].as_str().ok_or("SPL mint: missing owner")?;
    if ![
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
    ]
    .contains(&owner)
    {
        return Err("SPL mint: unsupported owner program".into());
    }
    let parsed = &account["data"]["parsed"];
    let info = &parsed["info"];
    if parsed["type"] != "mint" || info["isInitialized"] != true {
        return Err("SPL mint: expected initialized mint".into());
    }
    let mut transfer_fee_extension = false;
    let extensions = match info.get("extensions") {
        None => &Vec::new(),
        Some(extensions) => extensions
            .as_array()
            .ok_or("SPL mint: invalid extensions")?,
    };
    for extension in extensions {
        let state = &extension["state"];
        match extension["extension"].as_str() {
            Some(
                "metadataPointer"
                | "tokenMetadata"
                | "groupPointer"
                | "groupMemberPointer"
                | "tokenGroup"
                | "tokenGroupMember"
                | "mintCloseAuthority"
                | "permanentDelegate"
                | "confidentialTransferMint"
                | "confidentialTransferFeeConfig",
            ) => {}
            Some("transferHook") => {
                if !state["programId"].is_null() {
                    return Err("SPL mint: transfer hook programs are not supported".into());
                }
            }
            Some("transferFeeConfig") => {
                // A fee is zero when either factor is: no basis points, or a
                // cap of nothing.
                let charges_nothing = |config: &serde_json::Value| {
                    config["transferFeeBasisPoints"].as_u64() == Some(0)
                        || config["maximumFee"].as_u64() == Some(0)
                };
                if !charges_nothing(&state["olderTransferFee"])
                    || !charges_nothing(&state["newerTransferFee"])
                {
                    return Err(
                        "SPL mint: tokens that charge a transfer fee are not supported".into(),
                    );
                }
                transfer_fee_extension = true;
            }
            Some(other) => {
                return Err(format!(
                    "SPL mint: Token-2022 extension {other} is not supported for sending"
                ));
            }
            None => return Err("SPL mint: unnamed extension".into()),
        }
    }
    let decimals = info["decimals"]
        .as_u64()
        .and_then(|d| u8::try_from(d).ok())
        .ok_or("SPL mint: invalid decimals")?;
    Ok(TransferMint {
        program: crate::derivation::solana::decode_b58_32(owner)?,
        decimals,
        transfer_fee_extension,
    })
}

#[cfg(test)]
mod audit_fix5_mint_tests {
    use super::*;
    #[test]
    fn mint_program_precision_and_extensions_are_validated() {
        let account = |owner: &str| json!({"owner":owner,"data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":9,"extensions":[]}}}});
        let legacy = account("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
        let mut token2022 = account("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
        let a = validate_transfer_mint(&legacy).unwrap();
        let b = validate_transfer_mint(&token2022).unwrap();
        assert_ne!(a.program, b.program);
        assert_eq!(a.decimals, 9);
        assert!(!a.transfer_fee_extension && !b.transfer_fee_extension);
        let program = |id: &str| crate::derivation::solana::decode_b58_32(id).unwrap();
        assert_eq!(
            a.program,
            program("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
        );
        assert_eq!(
            b.program,
            program("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")
        );
        token2022["data"]["parsed"]["info"]["extensions"] =
            json!([{"extension":"nonTransferable"}]);
        assert!(
            validate_transfer_mint(&token2022)
                .unwrap_err()
                .contains("nonTransferable")
        );
        assert!(validate_transfer_mint(&account("11111111111111111111111111111111")).is_err());
        assert!(validate_transfer_mint(&serde_json::Value::Null).is_err());
    }

    /// PYUSD's mint as Solana Devnet reported it on 2026-09-30, authorities
    /// elided: eight extensions, a zero fee and a hook with no program.
    fn pyusd_mint() -> Value {
        let fee = json!({"epoch": 644, "maximumFee": 0, "transferFeeBasisPoints": 0});
        json!({"owner":"TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb","data":{"parsed":{"type":"mint","info":{
            "isInitialized": true, "decimals": 6,
            "extensions": [
                {"extension":"mintCloseAuthority","state":{"closeAuthority":"x"}},
                {"extension":"permanentDelegate","state":{"delegate":"x"}},
                {"extension":"transferFeeConfig","state":{"newerTransferFee":fee,"olderTransferFee":fee,
                    "transferFeeConfigAuthority":"x","withdrawWithheldAuthority":"x","withheldAmount":0}},
                {"extension":"confidentialTransferMint","state":{"auditorElgamalPubkey":null,"authority":"x","autoApproveNewAccounts":false}},
                {"extension":"confidentialTransferFeeConfig","state":{"authority":"x","harvestToMintEnabled":true}},
                {"extension":"transferHook","state":{"authority":"x","programId":null}},
                {"extension":"metadataPointer","state":{"authority":"x","metadataAddress":"x"}},
                {"extension":"tokenMetadata","state":{"symbol":"PYUSD"}}
            ]
        }}}})
    }

    fn with_extension(mut mint: Value, name: &str, state: Value) -> Value {
        let extensions = mint
            .pointer_mut("/data/parsed/info/extensions")
            .and_then(Value::as_array_mut)
            .unwrap();
        extensions.retain(|e| e["extension"] != name);
        extensions.push(json!({"extension": name, "state": state}));
        mint
    }

    /// Every extension PYUSD carries leaves the transfer meaning what it says,
    /// and the fee extension is reported so the transfer can assert zero.
    #[test]
    fn pyusd_is_sendable_and_states_its_fee_extension() {
        let mint = validate_transfer_mint(&pyusd_mint()).unwrap();
        assert_eq!(mint.decimals, 6);
        assert!(mint.transfer_fee_extension);
    }

    /// A fee charges nothing when either factor is zero, in both the current
    /// and the scheduled config; anything else is refused.
    #[test]
    fn a_fee_that_can_withhold_is_refused() {
        let config = |older: Value, newer: Value| {
            with_extension(
                pyusd_mint(),
                "transferFeeConfig",
                json!({"olderTransferFee": older, "newerTransferFee": newer}),
            )
        };
        let fee = |bps: u64, max: u64| json!({"transferFeeBasisPoints": bps, "maximumFee": max});
        for (older, newer, sendable) in [
            (fee(0, 5), fee(0, 5), true),
            (fee(50, 0), fee(50, 0), true),
            (fee(50, 5), fee(0, 0), false),
            // Scheduled for a later epoch still counts: review cannot know
            // which epoch the transaction lands in.
            (fee(0, 0), fee(1, 1), false),
            (json!({}), fee(0, 0), false),
        ] {
            let result = validate_transfer_mint(&config(older, newer));
            assert_eq!(result.is_ok(), sendable, "{result:?}");
            if !sendable {
                assert!(result.unwrap_err().contains("transfer fee"));
            }
        }
    }

    /// A hook with a program needs extra accounts this transfer does not
    /// resolve.
    #[test]
    fn a_transfer_hook_program_is_refused() {
        let mint = with_extension(
            pyusd_mint(),
            "transferHook",
            json!({"programId": "HooK111111111111111111111111111111111111111"}),
        );
        assert!(validate_transfer_mint(&mint).unwrap_err().contains("hook"));
    }

    /// Extensions that change what arrives, and ones nobody listed, are refused.
    #[test]
    fn unlisted_extensions_are_refused() {
        for name in [
            "nonTransferable",
            "defaultAccountState",
            "interestBearingConfig",
            "scaledUiAmountConfig",
            "pausableConfig",
            "somethingNew",
        ] {
            let mint = with_extension(pyusd_mint(), name, json!({}));
            assert!(
                validate_transfer_mint(&mint).unwrap_err().contains(name),
                "{name}"
            );
        }
        let mut unnamed = pyusd_mint();
        unnamed["data"]["parsed"]["info"]["extensions"] = json!([{}]);
        assert!(validate_transfer_mint(&unnamed).is_err());
        unnamed["data"]["parsed"]["info"]["extensions"] = json!("none");
        assert!(validate_transfer_mint(&unnamed).is_err());
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "Me11111111111111111111111111111111111111111";
    const PAYER: &str = "Payer111111111111111111111111111111111111111";
    const MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    /// A version-0 transaction that loads the address from a lookup table and
    /// credits it a single lamport — spam dust, but a real transfer.
    #[test]
    fn an_address_loaded_from_a_lookup_table_is_read_exactly() {
        let tx = json!({
            "slot": 1, "blockTime": 1_790_000_000,
            "transaction": {"message": {"accountKeys": [PAYER, "Program1111111111111111111111111111111111111"]}},
            "meta": {
                "fee": 5000,
                "loadedAddresses": {"writable": [ME], "readonly": []},
                "preBalances": [10_000_000u64, 1, 2_000_000],
                "postBalances": [9_994_999u64, 1, 2_000_001]
            }
        });
        let entries = solana_transfers_in_transaction(&tx, "sig", ME);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert!(entries[0].is_incoming);
        assert_eq!(entries[0].amount_display, "0.000000001");
    }

    /// Paying only a fee is not a transfer; a token account closed by the
    /// transaction still reports what left it.
    #[test]
    fn fee_only_changes_are_nothing_and_closed_token_accounts_count() {
        let tx = json!({
            "slot": 1, "blockTime": 1_790_000_000,
            "transaction": {"message": {"accountKeys": [ME, "Token1111111111111111111111111111111111111"]}},
            "meta": {
                "fee": 5000,
                "preBalances": [10_000_000u64, 2_039_280],
                "postBalances": [9_995_000u64, 2_039_280],
                "preTokenBalances": [{
                    "accountIndex": 1, "mint": MINT, "owner": ME,
                    "uiTokenAmount": {"amount": "2500000", "decimals": 6}
                }],
                "postTokenBalances": []
            }
        });
        let entries = solana_transfers_in_transaction(&tx, "sig", ME);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].mint, MINT);
        assert!(!entries[0].is_incoming);
        assert_eq!(entries[0].amount_display, "2.5");
    }
}
