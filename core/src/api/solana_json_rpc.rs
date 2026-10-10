//! The Solana JSON-RPC adapter: balances, SPL token accounts, signatures
//! history, blockhashes, fees and raw transaction broadcast.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::HttpClient;

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
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

#[derive(Debug, Clone)]
pub(crate) struct SolanaStakeAccount {
    pub address: String,
    pub lamports: u64,
    pub rent_reserve: u64,
    pub staker: String,
    pub withdrawer: String,
    pub vote: Option<String>,
    pub delegated: u64,
    pub activation_epoch: Option<u64>,
    pub deactivation_epoch: Option<u64>,
    pub lockup_epoch: u64,
    pub lockup_time: i64,
}

fn parse_stake_account(address: &str, account: &Value) -> Result<SolanaStakeAccount, ApiError> {
    if account["owner"].as_str() != Some(crate::registry::Chain::Solana.solana_stake_program()?)
        || !matches!(
            account["data"]["parsed"]["type"].as_str(),
            Some("initialized" | "delegated")
        )
    {
        return Err(ApiError::decode(
            "Solana position is not an initialized stake account",
        ));
    }
    let info = &account["data"]["parsed"]["info"];
    let meta = &info["meta"];
    let unit = |v: &Value| {
        v.as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| v.as_u64())
            .or_decode("Solana stake: invalid integer")
    };
    let delegation = &info["stake"]["delegation"];
    let delegated = account["data"]["parsed"]["type"] == "delegated";
    Ok(SolanaStakeAccount {
        address: address.into(),
        lamports: unit(&account["lamports"])?,
        rent_reserve: unit(&meta["rentExemptReserve"])?,
        staker: meta["authorized"]["staker"]
            .as_str()
            .or_decode("Solana stake: missing staker")?
            .into(),
        withdrawer: meta["authorized"]["withdrawer"]
            .as_str()
            .or_decode("Solana stake: missing withdrawer")?
            .into(),
        vote: if delegated {
            Some(
                delegation["voter"]
                    .as_str()
                    .or_decode("Solana stake: missing voter")?
                    .into(),
            )
        } else {
            None
        },
        delegated: if delegated {
            unit(&delegation["stake"])?
        } else {
            0
        },
        activation_epoch: if delegated {
            Some(unit(&delegation["activationEpoch"])?)
        } else {
            None
        },
        deactivation_epoch: if delegated {
            Some(unit(&delegation["deactivationEpoch"])?)
        } else {
            None
        },
        lockup_epoch: unit(&meta["lockup"]["epoch"])?,
        lockup_time: meta["lockup"]["unixTimestamp"]
            .as_i64()
            .or_decode("Solana stake: missing lockup time")?,
    })
}

impl SolanaClient {
    pub(crate) async fn fetch_stake_accounts(
        &self,
        owner: &str,
    ) -> Result<Vec<SolanaStakeAccount>, ApiError> {
        let mut accounts = std::collections::BTreeMap::new();
        // Bincode enum(4), rent reserve(8), then the two authority pubkeys.
        for offset in [12, 44] {
            let result=self.call("getProgramAccounts",json!([crate::registry::Chain::Solana.solana_stake_program()?,{"encoding":"jsonParsed","commitment":"confirmed","filters":[{"dataSize":200},{"memcmp":{"offset":offset,"bytes":owner}}]}])).await?;
            for row in result
                .as_array()
                .or_decode("Solana stake: missing accounts")?
            {
                let address = row["pubkey"]
                    .as_str()
                    .or_decode("Solana stake: missing account address")?;
                let account = parse_stake_account(address, &row["account"])?;
                if account.staker != owner && account.withdrawer != owner {
                    return Err(ApiError::decode("Solana stake: account authority mismatch"));
                }
                accounts.insert(address.to_string(), account);
            }
        }
        Ok(accounts.into_values().collect())
    }

    pub(crate) async fn fetch_stake_account(
        &self,
        address: &str,
    ) -> Result<SolanaStakeAccount, ApiError> {
        let result = self
            .call(
                "getAccountInfo",
                json!([address,{"encoding":"jsonParsed","commitment":"confirmed"}]),
            )
            .await?;
        parse_stake_account(address, &result["value"])
    }

    pub(crate) async fn fetch_staking_epoch(&self) -> Result<u64, ApiError> {
        let result = self
            .call("getEpochInfo", json!([{"commitment":"confirmed"}]))
            .await?;
        result["epoch"]
            .as_u64()
            .or_decode("Solana stake: missing current epoch")
    }

    pub(crate) async fn fetch_stake_minimum(&self) -> Result<u64, ApiError> {
        let result = self
            .call(
                "getStakeMinimumDelegation",
                json!([{"commitment":"confirmed"}]),
            )
            .await?;
        result["value"]
            .as_u64()
            .or_decode("Solana stake: missing minimum delegation")
    }

    pub(crate) async fn fetch_stake_rent(&self) -> Result<u64, ApiError> {
        let result = self
            .call(
                "getMinimumBalanceForRentExemption",
                json!([200,{"commitment":"confirmed"}]),
            )
            .await?;
        result
            .as_u64()
            .or_decode("Solana stake: missing account rent")
    }

    pub(crate) async fn fetch_staking_message_fee(&self, message: &[u8]) -> Result<u64, ApiError> {
        use base64::Engine;
        let result=self.call("getFeeForMessage",json!([base64::engine::general_purpose::STANDARD.encode(message),{"commitment":"confirmed"}])).await?;
        result["value"]
            .as_u64()
            .or_decode("Solana stake: missing transaction fee or expired blockhash")
    }

    pub(crate) async fn simulate_staking_message(&self, message: &[u8]) -> Result<bool, ApiError> {
        use base64::Engine;
        let mut bytes = vec![1];
        bytes.extend([0; 64]);
        bytes.extend(message);
        let result=self.call("simulateTransaction",json!([base64::engine::general_purpose::STANDARD.encode(bytes),{"encoding":"base64","sigVerify":false,"commitment":"confirmed"}])).await?;
        let error = result["value"]
            .get("err")
            .or_decode("Solana staking simulation: missing result")?;
        if error.is_null() {
            return Ok(true);
        }
        if error.get("InstructionError").is_some() {
            return Ok(false);
        }
        // Fee-payer exhaustion, a stale blockhash or a node problem cannot
        // establish that an owned stake is locked. Preserve the read failure.
        Err(ApiError::invalid(format!(
            "Unable to determine Solana staking execution: {error}"
        )))
    }
}

/// One token account, as a closure checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SolanaTokenAccount {
    pub address: String,
    pub program: String,
    pub mint: String,
    pub owner: String,
    pub amount: u128,
    /// The rent it holds, which closing returns.
    pub lamports: u64,
    pub close_authority: Option<String>,
    pub frozen: bool,
    /// Token-2022 transfer fees withheld in it, which block closing.
    pub withheld: u128,
}

// Solana fetch paths: native balance, SPL balances, recent blockhash,
// unified history, account existence.

impl SolanaClient {
    pub(crate) async fn verify_network(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), ApiError> {
        let result = self.call("getGenesisHash", json!([])).await?;
        if result.as_str() != Some(chain.solana_genesis_hash()?) {
            return Err(ApiError::decode("Solana endpoint is on the wrong network"));
        }
        Ok(())
    }

    pub(crate) async fn fetch_transaction_status(
        &self,
        signature: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        crate::api::transaction_status::validate_base58_hash(signature, 64)?;
        let result = self
            .call(
                "getSignatureStatuses",
                json!([[signature], {"searchTransactionHistory": true}]),
            )
            .await?;
        solana_transaction_status(&result)
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<SolanaBalance, ApiError> {
        let result = self
            .call("getBalance", json!([address, {"commitment": "confirmed"}]))
            .await?;
        let lamports = result
            .get("value")
            .and_then(|v| v.as_u64())
            .or_decode("getBalance: missing value")?;
        Ok(SolanaBalance { lamports })
    }

    /// Every token account the owner holds under either token program, as
    /// one confirmed read each: what closing them depends on.
    pub(crate) async fn fetch_token_accounts(
        &self,
        owner: &str,
    ) -> Result<Vec<SolanaTokenAccount>, ApiError> {
        let mut out = Vec::new();
        for program in crate::send::solana::TOKEN_PROGRAMS {
            let value = self
                .call(
                    "getTokenAccountsByOwner",
                    json!([owner, {"programId": program}, {"encoding": "jsonParsed", "commitment": "confirmed"}]),
                )
                .await?;
            for entry in value["value"]
                .as_array()
                .or_decode("getTokenAccountsByOwner: missing account list")?
            {
                let info = entry
                    .pointer("/account/data/parsed/info")
                    .or_decode("token account: not parsed")?;
                let text = |field: &str| info[field].as_str().map(str::to_string);
                let withheld = info["extensions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|extension| extension["extension"] == "transferFeeAmount")
                    .map(|extension| {
                        extension["state"]["withheldAmount"]
                            .as_u64()
                            .map(u128::from)
                            .or_else(|| {
                                extension["state"]["withheldAmount"]
                                    .as_str()
                                    .and_then(|amount| amount.parse().ok())
                            })
                            .or_decode("token account: invalid withheld amount")
                    })
                    .sum::<Result<u128, ApiError>>()?;
                out.push(SolanaTokenAccount {
                    address: entry["pubkey"]
                        .as_str()
                        .or_decode("token account: missing address")?
                        .to_string(),
                    program: program.to_string(),
                    mint: text("mint").or_decode("token account: missing mint")?,
                    owner: text("owner").or_decode("token account: missing owner")?,
                    amount: info
                        .pointer("/tokenAmount/amount")
                        .and_then(Value::as_str)
                        .and_then(|amount| amount.parse().ok())
                        .or_decode("token account: missing amount")?,
                    lamports: entry
                        .pointer("/account/lamports")
                        .and_then(Value::as_u64)
                        .or_decode("token account: missing lamports")?,
                    close_authority: text("closeAuthority"),
                    frozen: info["state"] == "frozen",
                    withheld,
                });
            }
        }
        Ok(out)
    }

    /// Every SPL token account the owner holds, in one call.
    ///
    /// `getTokenAccountsByOwner` filtered by `programId` rather than by mint
    /// returns the lot, and the parsed account carries the mint's own
    /// `decimals` — so discovery answers "what does this address hold" and
    /// "how is it denominated" together, without a catalog and without an
    /// indexer.
    pub async fn fetch_all_spl_balances(&self, owner: &str) -> Result<Vec<SplBalance>, ApiError> {
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
                    .or_decode("SPL token account has no amount")?
                    .parse()
                    .map_err(|_| {
                        ApiError::Decode("SPL token account amount is not an integer".into())
                    })?;
                // A closed or emptied account is not a holding.
                if raw == 0 {
                    continue;
                }
                let decimals = token_amount
                    .get("decimals")
                    .and_then(|v| v.as_u64())
                    .and_then(|d| u8::try_from(d).ok())
                    .or_decode("SPL token account has no decimals")?;
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
                            .or_decode("SPL balance overflow")?;
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
    ) -> Result<Vec<SplBalance>, ApiError> {
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
                        .or_decode("getTokenAccountsByOwner: missing account list")?;
                    if accounts.is_empty() {
                        return Ok::<_, ApiError>(None);
                    }
                    let mut raw = 0u128;
                    let mut own_decimals = None;
                    for account in accounts {
                        let amount = account
                            .pointer("/account/data/parsed/info/tokenAmount")
                            .or_decode("SPL account: missing tokenAmount")?;
                        let value: u64 = amount
                            .get("amount")
                            .and_then(|v| v.as_str())
                            .or_decode("SPL account: missing amount")?
                            .parse()
                            .map_err(|_| ApiError::Decode("SPL account: invalid amount".into()))?;
                        let decimals = crate::api::checked_token_decimals(u128::from(
                            amount
                                .get("decimals")
                                .and_then(|v| v.as_u64())
                                .or_decode("SPL account: missing decimals")?,
                        ))?;
                        if own_decimals.is_some_and(|previous| previous != decimals) {
                            return Err(ApiError::Decode(
                                "SPL accounts disagree on mint decimals".into(),
                            ));
                        }
                        own_decimals = Some(decimals);
                        raw = raw
                            .checked_add(u128::from(value))
                            .or_decode("SPL balance overflow")?;
                    }
                    let decimals = own_decimals.or_decode("SPL mint decimals unavailable")?;
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
            .collect::<Result<Vec<_>, ApiError>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    pub async fn fetch_recent_blockhash(&self) -> Result<String, ApiError> {
        let result = self
            .call("getLatestBlockhash", json!([{"commitment": "confirmed"}]))
            .await?;
        result
            .get("value")
            .and_then(|v| v.get("blockhash"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_decode("getLatestBlockhash: missing blockhash")
    }

    /// Fetch up to `limit` recent transfers as unified entries covering both
    /// native SOL and SPL token transfers.
    pub async fn fetch_unified_history_page(
        &self,
        address: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<SolanaTransfer>, ApiError> {
        let limit = limit.clamp(1, 50);
        let mut options = json!({"limit": limit, "commitment": "confirmed"});
        if let Some(cursor) = cursor {
            options["before"] = json!(cursor);
        }
        // 1. Get signatures.
        let sigs_result = self
            .call("getSignaturesForAddress", json!([address, options]))
            .await?;
        let sig_array = sigs_result
            .as_array()
            .or_decode("getSignaturesForAddress: expected array")?;

        if sig_array.len() > limit {
            return Err(ApiError::Decode(
                "Solana history exceeds requested page size".into(),
            ));
        }
        let signatures: Vec<String> = sig_array
            .iter()
            .map(|row| {
                row["signature"]
                    .as_str()
                    .filter(|signature| !signature.is_empty())
                    .map(str::to_string)
                    .or_decode("Solana history: missing signature")
            })
            .collect::<Result<_, _>>()?;

        if signatures.is_empty() {
            return Ok(crate::api::HistoryPage {
                items: vec![],
                next_cursor: None,
            });
        }

        // 2. Fetch each transaction and build unified entries.
        let mut result: Vec<SolanaTransfer> = Vec::new();
        for sig in &signatures {
            let tx = self
                .call(
                    "getTransaction",
                    json!([sig, {"encoding": "json", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}]),
                )
                .await?;
            if tx.is_null() {
                return Err(ApiError::Rejected(format!(
                    "Solana transaction {sig} is unavailable; history page cannot be consumed"
                )));
            }
            result.extend(solana_transfers_in_transaction(&tx, sig, address));
        }
        let next_cursor = (sig_array.len() == limit)
            .then(|| signatures.last().cloned())
            .flatten();
        Ok(crate::api::HistoryPage {
            items: result,
            next_cursor,
        })
    }
}

fn solana_transaction_status(
    result: &Value,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    let rows = result["value"]
        .as_array()
        .filter(|rows| rows.len() == 1)
        .or_decode("Solana status: expected one signature result")?;
    let row = &rows[0];
    if row.is_null() {
        return Ok(TransactionStatus::Pending);
    }
    match row["confirmationStatus"].as_str() {
        Some("processed" | "confirmed") => return Ok(TransactionStatus::Pending),
        Some("finalized") => {}
        _ => return Err(ApiError::decode("Solana status: invalid finality")),
    }
    let error = row
        .get("err")
        .or_decode("Solana status: missing execution result")?;
    Ok(TransactionStatus::Confirmed {
        succeeded: error.is_null(),
        block: Some(
            row["slot"]
                .as_u64()
                .or_decode("Solana status: missing slot")?,
        ),
    })
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
    pub async fn broadcast_raw(
        &self,
        signed_tx_base64: &str,
    ) -> Result<SolanaSendResult, ApiError> {
        let result = self
            .call(
                "sendTransaction",
                json!([signed_tx_base64, {"encoding": "base64", "preflightCommitment": "confirmed"}]),
            )
            .await?;
        let signature = result
            .as_str()
            .or_decode("sendTransaction: expected string")?
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
    /// Token-2022's transfer fee, when the mint has the extension: the
    /// transfer then states the fee it expects, and the program fails it on
    /// any other.
    pub transfer_fee: Option<TransferFeeConfig>,
    /// The program a Token-2022 transfer hook runs on every transfer.
    pub transfer_hook: Option<[u8; 32]>,
    /// New token accounts start frozen, so a recipient without one cannot
    /// receive until the issuer thaws the account the transfer would create.
    pub default_frozen: bool,
}

/// One Token-2022 transfer fee setting: from `epoch` on, `basis_points` of
/// the amount, rounded up and capped at `maximum_fee`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TransferFee {
    pub epoch: u64,
    pub maximum_fee: u64,
    pub basis_points: u16,
}

impl TransferFee {
    /// spl-token-2022's `TransferFee::calculate_fee`, which the program
    /// recomputes and compares with the fee a transfer states.
    pub fn fee(&self, amount: u64) -> u64 {
        if self.basis_points == 0 || amount == 0 {
            return 0;
        }
        let raw = (u128::from(amount) * u128::from(self.basis_points)).div_ceil(10_000);
        // Never more than the amount: the basis points are at most 10 000.
        u64::try_from(raw).map_or(self.maximum_fee, |raw| raw.min(self.maximum_fee))
    }
}

/// A mint's transfer fee: the older setting, until the epoch the newer one
/// takes over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TransferFeeConfig {
    pub older: TransferFee,
    pub newer: TransferFee,
}

impl TransferFeeConfig {
    /// The setting a transaction executing in `epoch` pays.
    pub fn at(&self, epoch: u64) -> TransferFee {
        if epoch >= self.newer.epoch {
            self.newer
        } else {
            self.older
        }
    }
}

/// Where the cluster is in its epoch: a Token-2022 fee is the one of the
/// epoch the transaction executes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EpochInfo {
    pub epoch: u64,
    pub slot_index: u64,
    pub slots_in_epoch: u64,
}

/// A token account a transfer reads from or credits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TransferTokenAccount {
    /// The token program that owns it.
    pub program: [u8; 32],
    pub mint: String,
    pub owner: String,
    pub amount: u64,
    pub frozen: bool,
    /// Token-2022 `MemoTransfer`: a credit without a memo before it fails.
    pub requires_memo: bool,
    /// Token-2022 confidential transfers with non-confidential credits
    /// switched off: a public transfer into it fails.
    pub refuses_public_credits: bool,
}

impl SolanaClient {
    /// The mint as a transfer of it must treat it, or a refusal.
    pub(crate) async fn fetch_transfer_mint(&self, mint: &str) -> Result<TransferMint, ApiError> {
        crate::derivation::solana::decode_b58_32(mint).map_err(ApiError::invalid)?;
        let result = self
            .call(
                "getAccountInfo",
                json!([mint, {"encoding":"jsonParsed", "commitment":"confirmed"}]),
            )
            .await?;
        validate_transfer_mint(&result["value"])
    }

    pub(crate) async fn fetch_epoch_info(&self) -> Result<EpochInfo, ApiError> {
        let result = self
            .call("getEpochInfo", json!([{"commitment": "confirmed"}]))
            .await?;
        let field = |name: &str| {
            result[name]
                .as_u64()
                .or_decode("getEpochInfo: missing epoch position")
        };
        let info = EpochInfo {
            epoch: field("epoch")?,
            slot_index: field("slotIndex")?,
            slots_in_epoch: field("slotsInEpoch")?,
        };
        if info.slot_index >= info.slots_in_epoch {
            return Err(ApiError::decode("getEpochInfo: slot outside its epoch"));
        }
        Ok(info)
    }

    /// A token account, or `None` when nothing is at the address yet.
    pub(crate) async fn fetch_transfer_token_account(
        &self,
        address: &str,
    ) -> Result<Option<TransferTokenAccount>, ApiError> {
        let result = self
            .call(
                "getAccountInfo",
                json!([address, {"encoding":"jsonParsed", "commitment":"confirmed"}]),
            )
            .await?;
        let account = &result["value"];
        if account.is_null() {
            return Ok(None);
        }
        parse_transfer_token_account(account).map(Some)
    }

    /// An account's owner program and raw data, or `None` when it does not
    /// exist: what a transfer hook's extra accounts are resolved from.
    pub(crate) async fn fetch_account_data(
        &self,
        address: &str,
    ) -> Result<Option<([u8; 32], Vec<u8>)>, ApiError> {
        use base64::Engine;
        let result = self
            .call(
                "getAccountInfo",
                json!([address, {"encoding":"base64", "commitment":"confirmed"}]),
            )
            .await?;
        let account = &result["value"];
        if account.is_null() {
            return Ok(None);
        }
        let owner = account["owner"]
            .as_str()
            .or_decode("getAccountInfo: missing owner")?;
        let data = account["data"]
            .as_array()
            .filter(|data| data.get(1).and_then(Value::as_str) == Some("base64"))
            .and_then(|data| data.first())
            .and_then(Value::as_str)
            .or_decode("getAccountInfo: missing base64 data")?;
        Ok(Some((
            crate::derivation::solana::decode_b58_32(owner)?,
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| ApiError::decode("getAccountInfo: invalid base64 data"))?,
        )))
    }

    /// Run an unsigned message without signature checks. `None` when it
    /// executes; the program's error when an instruction fails. A node that
    /// cannot run it at all is a read failure, not a verdict.
    pub(crate) async fn simulate_message(
        &self,
        message: &[u8],
    ) -> Result<Option<String>, ApiError> {
        use base64::Engine;
        let mut bytes = vec![1];
        bytes.extend([0; 64]);
        bytes.extend(message);
        let result = self
            .call(
                "simulateTransaction",
                json!([
                    base64::engine::general_purpose::STANDARD.encode(bytes),
                    {"encoding":"base64","sigVerify":false,"commitment":"confirmed"}
                ]),
            )
            .await?;
        let error = result["value"]
            .get("err")
            .or_decode("simulateTransaction: missing result")?;
        if error.is_null() {
            return Ok(None);
        }
        if error.get("InstructionError").is_some() {
            return Ok(Some(error.to_string()));
        }
        Err(ApiError::rejected(format!(
            "Solana simulation did not run: {error}"
        )))
    }
}

fn parse_transfer_token_account(account: &Value) -> Result<TransferTokenAccount, ApiError> {
    let owner = account["owner"]
        .as_str()
        .or_decode("token account: missing owner program")?;
    let parsed = &account["data"]["parsed"];
    if parsed["type"] != "account" {
        return Err(ApiError::invalid("Not a token account"));
    }
    let info = &parsed["info"];
    let text = |field: &str| {
        info[field]
            .as_str()
            .map(str::to_string)
            .or_decode("token account: missing mint or owner")
    };
    let mut requires_memo = false;
    let mut refuses_public_credits = false;
    for extension in info["extensions"].as_array().into_iter().flatten() {
        let state = &extension["state"];
        match extension["extension"].as_str() {
            Some("memoTransfer") => {
                requires_memo = state["requireIncomingTransferMemos"]
                    .as_bool()
                    .or_decode("token account: invalid memo requirement")?;
            }
            Some("confidentialTransferAccount") => {
                refuses_public_credits = !state["allowNonConfidentialCredits"]
                    .as_bool()
                    .or_decode("token account: invalid confidential transfer state")?;
            }
            _ => {}
        }
    }
    Ok(TransferTokenAccount {
        program: crate::derivation::solana::decode_b58_32(owner)?,
        mint: text("mint")?,
        owner: text("owner")?,
        amount: info
            .pointer("/tokenAmount/amount")
            .and_then(Value::as_str)
            .and_then(|amount| amount.parse().ok())
            .or_decode("token account: missing amount")?,
        frozen: match info["state"].as_str() {
            Some("initialized") => false,
            Some("frozen") => true,
            _ => return Err(ApiError::decode("token account: not initialized")),
        },
        requires_memo,
        refuses_public_credits,
    })
}

/// Refuse unknown programs and extensions before signing.
///
/// Token-2022 extensions are taken by name, each for what it does to a
/// `TransferChecked` from the owner:
///
/// - metadata, group and member pointers and records only describe the mint;
/// - a mint close authority acts only on a mint with no supply, and
///   confidential and permissioned mint and burn settings act on minting and
///   burning;
/// - a permanent delegate can move the owner's tokens, but whether the owner
///   sends does not change that;
/// - confidential transfer configs govern encrypted balances, not this one;
/// - a transfer fee is withheld from the amount on its way: the transfer
///   states it, and the recipient receives the rest;
/// - a transfer hook runs its program, which the transfer gives the extra
///   accounts its validation account lists;
/// - a default frozen state freezes the account the transfer would create
///   for a recipient who has none;
/// - a pause stops every transfer while it lasts.
///
/// Anything else — non-transferable tokens, interest or scaled amounts that
/// show a balance other than the one moved, or an extension this list has
/// never heard of — is refused.
fn validate_transfer_mint(account: &serde_json::Value) -> Result<TransferMint, ApiError> {
    let owner = account["owner"]
        .as_str()
        .or_decode("SPL mint: missing owner")?;
    if !crate::send::solana::TOKEN_PROGRAMS.contains(&owner) {
        return Err(ApiError::InvalidInput(
            "SPL mint: unsupported owner program".into(),
        ));
    }
    let parsed = &account["data"]["parsed"];
    let info = &parsed["info"];
    if parsed["type"] != "mint" || info["isInitialized"] != true {
        return Err(ApiError::Decode(
            "SPL mint: expected initialized mint".into(),
        ));
    }
    let mut mint = TransferMint {
        program: crate::derivation::solana::decode_b58_32(owner)?,
        decimals: info["decimals"]
            .as_u64()
            .and_then(|d| u8::try_from(d).ok())
            .or_decode("SPL mint: invalid decimals")?,
        transfer_fee: None,
        transfer_hook: None,
        default_frozen: false,
    };
    let extensions = match info.get("extensions") {
        None => &Vec::new(),
        Some(extensions) => extensions
            .as_array()
            .or_decode("SPL mint: invalid extensions")?,
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
                | "confidentialTransferFeeConfig"
                | "confidentialMintBurn"
                | "permissionedBurnConfig",
            ) => {}
            Some("transferHook") => {
                mint.transfer_hook = match &state["programId"] {
                    Value::Null => None,
                    Value::String(program) => {
                        Some(crate::derivation::solana::decode_b58_32(program)?)
                    }
                    _ => return Err(ApiError::decode("SPL mint: invalid transfer hook")),
                };
            }
            Some("transferFeeConfig") => {
                let fee = |config: &Value| {
                    Some(TransferFee {
                        epoch: config["epoch"].as_u64()?,
                        maximum_fee: config["maximumFee"].as_u64()?,
                        basis_points: config["transferFeeBasisPoints"]
                            .as_u64()
                            .filter(|points| *points <= 10_000)
                            .and_then(|points| u16::try_from(points).ok())?,
                    })
                };
                mint.transfer_fee = Some(TransferFeeConfig {
                    older: fee(&state["olderTransferFee"])
                        .or_decode("SPL mint: invalid transfer fee")?,
                    newer: fee(&state["newerTransferFee"])
                        .or_decode("SPL mint: invalid transfer fee")?,
                });
            }
            Some("defaultAccountState") => {
                mint.default_frozen = match state["accountState"].as_str() {
                    Some("frozen") => true,
                    Some("initialized") => false,
                    _ => return Err(ApiError::decode("SPL mint: invalid default account state")),
                };
            }
            Some("pausableConfig") => match state["paused"].as_bool() {
                Some(false) => {}
                Some(true) => {
                    return Err(ApiError::invalid(
                        "The token's issuer has paused its transfers",
                    ));
                }
                None => return Err(ApiError::decode("SPL mint: invalid pause state")),
            },
            Some("nonTransferable") => {
                return Err(ApiError::invalid("This token cannot be transferred"));
            }
            Some("interestBearingConfig" | "scaledUiAmountConfig") => {
                return Err(ApiError::invalid(
                    "Sending a token whose displayed amount is scaled from the amount moved is not supported",
                ));
            }
            Some(other) => {
                return Err(ApiError::invalid(format!(
                    "SPL mint: Token-2022 extension {other} is not supported for sending"
                )));
            }
            None => return Err(ApiError::InvalidInput("SPL mint: unnamed extension".into())),
        }
    }
    Ok(mint)
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
        assert!(a.transfer_fee.is_none() && b.transfer_fee.is_none());
        assert!(a.transfer_hook.is_none() && !a.default_frozen);
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
                .to_string()
                .contains("cannot be transferred")
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

    /// Every extension PYUSD carries leaves the transfer meaning what it
    /// says: a fee of nothing, and a hook with no program to run.
    #[test]
    fn pyusd_is_sendable_with_a_zero_fee_and_no_hook() {
        let mint = validate_transfer_mint(&pyusd_mint()).unwrap();
        assert_eq!(mint.decimals, 6);
        let fee = mint.transfer_fee.unwrap();
        assert_eq!(fee.at(644).fee(u64::MAX), 0);
        assert!(mint.transfer_hook.is_none() && !mint.default_frozen);
    }

    /// spl-token-2022's fee: basis points of the amount rounded up, capped,
    /// and the newer setting from its own epoch on.
    #[test]
    fn a_transfer_fee_is_read_and_charged_per_epoch() {
        let fee = |epoch: u64, bps: u64, max: u64| json!({"epoch": epoch, "transferFeeBasisPoints": bps, "maximumFee": max});
        let mint = validate_transfer_mint(&with_extension(
            pyusd_mint(),
            "transferFeeConfig",
            json!({"olderTransferFee": fee(10, 50, 5_000), "newerTransferFee": fee(12, 100, 1_000_000)}),
        ))
        .unwrap();
        let config = mint.transfer_fee.unwrap();
        // 0.5%, rounded up: 1 unit of fee on anything from 1 to 200 units.
        assert_eq!(config.at(11).fee(1), 1);
        assert_eq!(config.at(11).fee(200), 1);
        assert_eq!(config.at(11).fee(201), 2);
        assert_eq!(config.at(11).fee(0), 0);
        // Capped at the maximum.
        assert_eq!(config.at(11).fee(10_000_000), 5_000);
        // The newer setting from its epoch on.
        assert_eq!(config.at(12).fee(10_000), 100);
        assert_eq!(config.at(13).fee(u64::MAX), 1_000_000);
        // A fee cannot be more than the amount.
        let all = TransferFee {
            epoch: 0,
            maximum_fee: u64::MAX,
            basis_points: 10_000,
        };
        assert_eq!(all.fee(u64::MAX), u64::MAX);
        for bad in [
            json!({"olderTransferFee": fee(1, 10_001, 1), "newerTransferFee": fee(1, 0, 0)}),
            json!({"olderTransferFee": {}, "newerTransferFee": fee(1, 0, 0)}),
        ] {
            assert!(
                validate_transfer_mint(&with_extension(pyusd_mint(), "transferFeeConfig", bad))
                    .is_err()
            );
        }
    }

    /// A hook names the program the transfer must give its extra accounts.
    #[test]
    fn a_transfer_hook_program_is_read() {
        let hook = "HooK111111111111111111111111111111111111111";
        let mint = validate_transfer_mint(&with_extension(
            pyusd_mint(),
            "transferHook",
            json!({"programId": hook}),
        ))
        .unwrap();
        assert_eq!(
            mint.transfer_hook,
            Some(crate::derivation::solana::decode_b58_32(hook).unwrap())
        );
        let malformed = with_extension(pyusd_mint(), "transferHook", json!({"programId": 7}));
        assert!(validate_transfer_mint(&malformed).is_err());
    }

    /// Frozen default accounts are reported, and a pause refuses only while
    /// it lasts.
    #[test]
    fn default_state_and_pause_are_read() {
        let frozen = with_extension(
            pyusd_mint(),
            "defaultAccountState",
            json!({"accountState": "frozen"}),
        );
        assert!(validate_transfer_mint(&frozen).unwrap().default_frozen);
        let thawed = with_extension(
            pyusd_mint(),
            "defaultAccountState",
            json!({"accountState": "initialized"}),
        );
        assert!(!validate_transfer_mint(&thawed).unwrap().default_frozen);
        let running = with_extension(
            pyusd_mint(),
            "pausableConfig",
            json!({"authority": "x", "paused": false}),
        );
        assert!(validate_transfer_mint(&running).is_ok());
        let paused = with_extension(
            pyusd_mint(),
            "pausableConfig",
            json!({"authority": "x", "paused": true}),
        );
        assert!(
            validate_transfer_mint(&paused)
                .unwrap_err()
                .to_string()
                .contains("paused")
        );
    }

    /// Extensions that change what a transfer means, and ones nobody listed,
    /// are refused, each with its reason.
    #[test]
    fn remaining_extensions_are_refused() {
        for (name, reason) in [
            ("nonTransferable", "cannot be transferred"),
            ("interestBearingConfig", "scaled"),
            ("scaledUiAmountConfig", "scaled"),
            ("unparseableExtension", "unparseableExtension"),
            ("somethingNew", "somethingNew"),
        ] {
            let mint = with_extension(pyusd_mint(), name, json!({}));
            let error = validate_transfer_mint(&mint).unwrap_err();
            assert!(
                matches!(error, ApiError::InvalidInput(_)) && error.to_string().contains(reason),
                "{name}: {error}"
            );
        }
        let mut unnamed = pyusd_mint();
        unnamed["data"]["parsed"]["info"]["extensions"] = json!([{}]);
        assert!(validate_transfer_mint(&unnamed).is_err());
        unnamed["data"]["parsed"]["info"]["extensions"] = json!("none");
        assert!(validate_transfer_mint(&unnamed).is_err());
    }

    /// A recipient account's memo requirement and confidential-only credits
    /// are read from its extensions.
    #[test]
    fn token_account_transfer_rules_are_read() {
        let account = |extensions: Value, state: &str| {
            json!({"owner":"TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb","data":{"parsed":{"type":"account","info":{
                "mint":"M","owner":"O","state":state,"tokenAmount":{"amount":"42","decimals":6},
                "extensions":extensions}}}})
        };
        let plain = parse_transfer_token_account(&account(json!([]), "initialized")).unwrap();
        assert_eq!(
            (
                plain.amount,
                plain.frozen,
                plain.requires_memo,
                plain.refuses_public_credits
            ),
            (42, false, false, false)
        );
        let strict = parse_transfer_token_account(&account(
            json!([
                {"extension":"memoTransfer","state":{"requireIncomingTransferMemos":true}},
                {"extension":"confidentialTransferAccount","state":{"allowNonConfidentialCredits":false}},
                {"extension":"immutableOwner"}
            ]),
            "frozen",
        ))
        .unwrap();
        assert!(strict.frozen && strict.requires_memo && strict.refuses_public_credits);
        assert!(parse_transfer_token_account(&account(json!([]), "uninitialized")).is_err());
        let mut mint = account(json!([]), "initialized");
        mint["data"]["parsed"]["type"] = json!("mint");
        assert!(parse_transfer_token_account(&mint).is_err());
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

// Validator-directory response data; staking owns the projection.
#[derive(Deserialize)]
pub struct VoteAccountsResult {
    pub current: Vec<VoteAccount>,
    pub delinquent: Vec<VoteAccount>,
}
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VoteAccount {
    pub vote_pubkey: String,
    pub activated_stake: u64,
    pub commission: u8,
}

impl SolanaClient {
    pub async fn fetch_staking_validators(&self) -> Result<VoteAccountsResult, ApiError> {
        let value = self
            .call(
                "getVoteAccounts",
                serde_json::json!([{"commitment":"confirmed","keepUnstakedDelinquents":false}]),
            )
            .await?;
        serde_json::from_value(value).map_err(ApiError::from)
    }
}

#[cfg(test)]
mod transaction_status_tests {
    use super::*;
    use crate::api::transaction_status::TransactionStatus;

    /// A signature settles only once finalized, by its error; confirmed but
    /// not finalized, or not yet known, it is still pending. One signature
    /// asked is one row answered.
    #[test]
    fn a_signature_settles_only_when_finalized() {
        let read = |rows: Vec<Value>| solana_transaction_status(&json!({"value": rows}));
        let row = |error: Value, finality: &str| json!({"slot": 42, "err": error, "confirmationStatus": finality});
        let failure = json!({"InstructionError": [0, "Custom"]});
        assert_eq!(
            read(vec![row(Value::Null, "finalized")]),
            Ok(TransactionStatus::Confirmed {
                succeeded: true,
                block: Some(42)
            })
        );
        assert_eq!(
            read(vec![row(failure.clone(), "finalized")]),
            Ok(TransactionStatus::Confirmed {
                succeeded: false,
                block: Some(42)
            })
        );
        assert_eq!(
            read(vec![row(failure, "confirmed")]),
            Ok(TransactionStatus::Pending)
        );
        assert_eq!(read(vec![Value::Null]), Ok(TransactionStatus::Pending));
        let finalized = row(Value::Null, "finalized");
        assert_eq!(
            read(vec![finalized.clone(), finalized]),
            Err(ApiError::decode(
                "Solana status: expected one signature result"
            ))
        );
    }
}
