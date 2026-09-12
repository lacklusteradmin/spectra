//! Solana chain client.
//!
//! Uses the Solana JSON-RPC API for balance, history, and broadcast.
//! Transaction serialization follows the compact (v0) wire format:
//!   [signatures] [message header] [accounts] [recent_blockhash] [instructions]
//!
//! Ed25519 signing is performed using the `ed25519-dalek` crate.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct SolanaBalance {
    /// Lamports (1 SOL = 1_000_000_000 lamports).
    pub lamports: u64,
    pub sol_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaHistoryEntry {
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub fee_lamports: u64,
    pub is_incoming: bool,
    pub amount_lamports: u64,
    pub from: String,
    pub to: String,
}

/// Unified history entry covering both native SOL and SPL token transfers.
/// Swift decodes this instead of `SolanaHistoryEntry` for the history tab.
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

impl super::SignedSubmission for SolanaSendResult {
    fn submission_id(&self) -> &str {
        &self.signature
    }
    fn signed_payload(&self) -> &str {
        &self.signed_tx_base64
    }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat {
        super::SignedPayloadFormat::Base64
    }
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
        let body = std::sync::Arc::new(rpc(method, params));
        with_fallback(&self.endpoints, |url| {
            let client = self.client.clone();
            let body = std::sync::Arc::clone(&body);
            async move {
                let resp: Value = client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
                    .await?;
                if let Some(err) = resp.get("error") {
                    return Err(format!("rpc error: {err}"));
                }
                resp.get("result")
                    .cloned()
                    .ok_or_else(|| "missing result".to_string())
            }
        })
        .await
    }
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
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
        Ok(SolanaBalance {
            lamports,
            sol_display: format_sol(lamports),
        })
    }

    /// Fetch SPL token balances for a list of mint addresses.
    /// Every SPL token account the owner holds, in one call.
    ///
    /// `getTokenAccountsByOwner` filtered by `programId` rather than by mint
    /// returns the lot, and the parsed account carries the mint's own
    /// `decimals` — so discovery answers "what does this address hold" and
    /// "how is it denominated" together, without a catalog and without an
    /// indexer.
    pub(crate) async fn fetch_transfer_mint(&self, mint: &str) -> Result<([u8; 32], u8), String> {
        crate::derivation::chains::solana::decode_b58_32(mint)?;
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
                let balance_raw = token_amount
                    .get("amount")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                // A closed or emptied account is not a holding.
                if balance_raw == "0" {
                    continue;
                }
                let decimals = token_amount
                    .get("decimals")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u8;
                let balance_display = token_amount
                    .get("uiAmountString")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                // One mint can have several accounts; sum them.
                if let Some(existing) = out.iter_mut().find(|b| b.mint == mint) {
                    let a: u128 = existing.balance_raw.parse().unwrap_or(0);
                    let b: u128 = balance_raw.parse().unwrap_or(0);
                    existing.balance_raw = (a + b).to_string();
                    existing.balance_display =
                        crate::fetch::chains::evm::format_token_amount(a + b, decimals);
                } else {
                    out.push(SplBalance {
                        mint: mint.to_string(),
                        owner: owner.to_string(),
                        balance_raw,
                        balance_display,
                        decimals,
                        symbol: String::new(),
                    });
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
                        let decimals = super::checked_token_decimals(u128::from(
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
                        balance_display: crate::fetch::chains::evm::format_token_amount(
                            raw, decimals,
                        ),
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

    pub async fn fetch_history(
        &self,
        address: &str,
        limit: usize,
    ) -> Result<Vec<SolanaHistoryEntry>, String> {
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

        // 2. Fetch transactions.
        let mut entries = Vec::new();
        for sig in &signatures {
            let tx = self
                .call(
                    "getTransaction",
                    json!([sig, {"encoding": "json", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}]),
                )
                .await
                .unwrap_or(Value::Null);

            if tx.is_null() {
                continue;
            }

            let slot = tx.get("slot").and_then(|v| v.as_u64()).unwrap_or(0);
            let timestamp = tx.get("blockTime").and_then(|v| v.as_i64());
            let fee = tx
                .pointer("/meta/fee")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            // Determine direction from pre/post balances for address index 0.
            let pre_balances = tx
                .pointer("/meta/preBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let post_balances = tx
                .pointer("/meta/postBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let accounts: Vec<String> = tx
                .pointer("/transaction/message/accountKeys")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();

            // Find this address's index.
            let idx = accounts.iter().position(|a| a == address);
            let (pre, post) = idx
                .and_then(|i| {
                    Some((
                        pre_balances.get(i)?.as_u64()?,
                        post_balances.get(i)?.as_u64()?,
                    ))
                })
                .unwrap_or((0, 0));

            let is_incoming = post > pre;
            let amount_lamports = if is_incoming {
                post.saturating_sub(pre)
            } else {
                pre.saturating_sub(post).saturating_sub(fee)
            };

            let from = accounts.first().cloned().unwrap_or_default();
            let to = accounts.get(1).cloned().unwrap_or_default();

            entries.push(SolanaHistoryEntry {
                signature: sig.clone(),
                slot,
                timestamp,
                fee_lamports: fee,
                is_incoming,
                amount_lamports,
                from,
                to,
            });
        }
        Ok(entries)
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

            if tx.is_null() {
                continue;
            }

            let slot = tx.get("slot").and_then(|v| v.as_u64()).unwrap_or(0);
            let timestamp = tx.get("blockTime").and_then(|v| v.as_i64());
            let fee = tx
                .pointer("/meta/fee")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let pre_balances = tx
                .pointer("/meta/preBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let post_balances = tx
                .pointer("/meta/postBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let accounts: Vec<String> = tx
                .pointer("/transaction/message/accountKeys")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();

            let from = accounts.first().cloned().unwrap_or_default();
            let to = accounts.get(1).cloned().unwrap_or_default();

            // Check for SPL token balance deltas.
            let pre_tok = tx
                .pointer("/meta/preTokenBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let post_tok = tx
                .pointer("/meta/postTokenBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut spl_entries: Vec<SolanaTransfer> = Vec::new();

            // Find post-token entries owned by this address.
            for post_entry in &post_tok {
                let owner = post_entry
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if owner != address {
                    continue;
                }
                let mint = post_entry
                    .get("mint")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let acct_idx = post_entry
                    .get("accountIndex")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(u64::MAX);

                let _post_ui = post_entry
                    .pointer("/uiTokenAmount/uiAmountString")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0");
                let post_raw: u128 = post_entry
                    .pointer("/uiTokenAmount/amount")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                // Find matching pre entry by accountIndex.
                let pre_raw: u128 = pre_tok
                    .iter()
                    .find(|e| {
                        e.get("accountIndex")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(u64::MAX)
                            == acct_idx
                    })
                    .and_then(|e| e.pointer("/uiTokenAmount/amount").and_then(|v| v.as_str()))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                if post_raw == pre_raw {
                    continue; // No change for this token account.
                }

                let is_incoming = post_raw > pre_raw;
                let decimals = post_entry
                    .pointer("/uiTokenAmount/decimals")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(6);
                let delta_raw = if is_incoming {
                    post_raw.saturating_sub(pre_raw)
                } else {
                    pre_raw.saturating_sub(post_raw)
                };
                let divisor = 10u128.pow(decimals as u32);
                let whole = delta_raw / divisor;
                let frac = delta_raw % divisor;
                let amount_display = if frac == 0 || decimals == 0 {
                    whole.to_string()
                } else {
                    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
                    let trimmed = frac_str.trim_end_matches('0');
                    format!("{}.{}", whole, trimmed)
                };

                spl_entries.push(SolanaTransfer {
                    signature: sig.clone(),
                    slot,
                    timestamp,
                    fee_lamports: fee,
                    is_incoming,
                    amount_display,
                    symbol: mint.clone(),
                    mint,
                    from: from.clone(),
                    to: to.clone(),
                });
            }

            if !spl_entries.is_empty() {
                result.extend(spl_entries);
                // Still emit a native SOL entry if the native balance changed
                // (fee + send amount visible separately).
                let idx = accounts.iter().position(|a| a == address);
                let (pre, post) = idx
                    .and_then(|i| {
                        Some((
                            pre_balances.get(i)?.as_u64()?,
                            post_balances.get(i)?.as_u64()?,
                        ))
                    })
                    .unwrap_or((0, 0));
                let sol_delta = if post > pre {
                    post.saturating_sub(pre)
                } else {
                    pre.saturating_sub(post).saturating_sub(fee)
                };
                if sol_delta > 0 {
                    let is_incoming = post > pre;
                    let sol_display = format_lamports(sol_delta);
                    result.push(SolanaTransfer {
                        signature: sig.clone(),
                        slot,
                        timestamp,
                        fee_lamports: fee,
                        is_incoming,
                        amount_display: sol_display,
                        symbol: "SOL".to_string(),
                        mint: String::new(),
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
                continue;
            }

            // No SPL transfers — emit as a native SOL entry.
            let idx = accounts.iter().position(|a| a == address);
            let (pre, post) = idx
                .and_then(|i| {
                    Some((
                        pre_balances.get(i)?.as_u64()?,
                        post_balances.get(i)?.as_u64()?,
                    ))
                })
                .unwrap_or((0, 0));

            let is_incoming = post > pre;
            let amount_lamports = if is_incoming {
                post.saturating_sub(pre)
            } else {
                pre.saturating_sub(post).saturating_sub(fee)
            };

            result.push(SolanaTransfer {
                signature: sig.clone(),
                slot,
                timestamp,
                fee_lamports: fee,
                is_incoming,
                amount_display: format_lamports(amount_lamports),
                symbol: "SOL".to_string(),
                mint: String::new(),
                from,
                to,
            });
        }

        Ok(result)
    }

    /// Check whether an account exists on-chain.
    pub async fn account_exists(&self, address: &str) -> Result<bool, String> {
        let result = self
            .call(
                "getAccountInfo",
                json!([address, {"encoding": "base64", "commitment": "confirmed"}]),
            )
            .await?;
        Ok(result
            .pointer("/value")
            .map(|v| !v.is_null())
            .unwrap_or(false))
    }
}

fn format_lamports(lamports: u64) -> String {
    format_sol(lamports)
}

fn format_sol(lamports: u64) -> String {
    let whole = lamports / 1_000_000_000;
    let frac = lamports % 1_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:09}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 {
        &trimmed[..6]
    } else {
        trimmed
    };
    format!("{}.{}", whole, capped)
}

#[cfg(test)]
mod balance_read_tests {
    use super::*;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

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

/// Refuse unknown programs and extensions before signing. Extensions may alter
/// transfer semantics or require accounts that our TransferChecked does not supply.
fn validate_transfer_mint(account: &serde_json::Value) -> Result<([u8; 32], u8), String> {
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
    if let Some(extensions) = info.get("extensions") {
        if !extensions.as_array().is_some_and(|e| e.is_empty()) {
            return Err("SPL mint: Token-2022 extensions are not supported for sending".into());
        }
    }
    let decimals = info["decimals"]
        .as_u64()
        .and_then(|d| u8::try_from(d).ok())
        .ok_or("SPL mint: invalid decimals")?;
    Ok((
        crate::derivation::chains::solana::decode_b58_32(owner)?,
        decimals,
    ))
}

#[cfg(test)]
mod audit_fix5_mint_tests {
    use super::*;
    #[test]
    fn audit_fix5_mint_program_precision_and_extensions_are_validated() {
        let account = |owner: &str| json!({"owner":owner,"data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":9,"extensions":[]}}}});
        let legacy = account("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
        let mut token2022 = account("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
        let (a, decimals) = validate_transfer_mint(&legacy).unwrap();
        let (b, _) = validate_transfer_mint(&token2022).unwrap();
        assert_ne!(a, b);
        assert_eq!(decimals, 9);
        let owner = crate::derivation::chains::solana::decode_b58_32(
            "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk",
        )
        .unwrap();
        let mint = [0x44; 32];
        // Independent @solana/spl-token 0.4.14 vectors.
        let ata = crate::send::chains::solana::derive_associated_token_account;
        assert_eq!(
            bs58::encode(ata(&owner, &mint, &a).unwrap()).into_string(),
            "FF2BjgeRK2LgK8Lj4wY2CTJrmJAKV5ZPCdHqfq1tJLGi"
        );
        assert_eq!(
            bs58::encode(ata(&owner, &mint, &b).unwrap()).into_string(),
            "Hzvpgx8hB4wZewvsYXSedgrgSb4yNycQRhufYeMaKuRM"
        );
        token2022["data"]["parsed"]["info"]["extensions"] = json!([{"extension":"transferHook"}]);
        assert!(validate_transfer_mint(&token2022)
            .unwrap_err()
            .contains("extensions"));
        assert!(validate_transfer_mint(&account("11111111111111111111111111111111")).is_err());
        assert!(validate_transfer_mint(&serde_json::Value::Null).is_err());
    }
}
