//! Solana fetch paths: native balance, SPL balances, recent blockhash,
//! unified history, account existence, mint decimals.

use base64::Engine as _;
use serde_json::{json, Value};

use super::{
    SolanaBalance, SolanaClient, SolanaHistoryEntry, SolanaTransfer, SplBalance,
};

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
                        .await;
                    let val = result.ok()?;
                    let accounts = val.get("value")?.as_array()?;
                    let account = accounts.first()?;
                    let info = account.pointer("/account/data/parsed/info")?;
                    let token_amount = info.get("tokenAmount")?;
                    let balance_raw = token_amount
                        .get("amount")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0")
                        .to_string();
                    let decimals = token_amount
                        .get("decimals")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u8;
                    let balance_display = token_amount
                        .get("uiAmountString")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0")
                        .to_string();
                    Some(SplBalance {
                        mint,
                        owner,
                        balance_raw,
                        balance_display,
                        decimals,
                        symbol: String::new(),
                    })
                }
            })
            .collect();

        let results = join_all(futs).await;
        Ok(results.into_iter().flatten().collect())
    }

    pub async fn fetch_recent_blockhash(&self) -> Result<String, String> {
        let result = self
            .call(
                "getLatestBlockhash",
                json!([{"commitment": "confirmed"}]),
            )
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
            .filter_map(|s| s.get("signature").and_then(|v| v.as_str()).map(str::to_string))
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
            .filter_map(|s| s.get("signature").and_then(|v| v.as_str()).map(str::to_string))
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
                let owner = post_entry.get("owner").and_then(|v| v.as_str()).unwrap_or("");
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
                        e.get("accountIndex").and_then(|v| v.as_u64()).unwrap_or(u64::MAX)
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

    /// Fetch the SPL token balance for `owner` holding `mint`.
    pub async fn fetch_spl_balance(&self, mint: &str, owner: &str) -> Result<SplBalance, String> {
        let result = self
            .call(
                "getTokenAccountsByOwner",
                json!([
                    owner,
                    { "mint": mint },
                    { "encoding": "jsonParsed", "commitment": "confirmed" }
                ]),
            )
            .await?;

        // Sum balances across all accounts the owner has for this mint.
        let accounts = result
            .get("value")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut total: u128 = 0;
        let mut decimals: u8 = 0;
        for acct in &accounts {
            let info = acct.pointer("/account/data/parsed/info/tokenAmount");
            if let Some(info) = info {
                let amt_str = info.get("amount").and_then(|v| v.as_str()).unwrap_or("0");
                let amt: u128 = amt_str.parse().unwrap_or(0);
                total = total.saturating_add(amt);
                if let Some(d) = info.get("decimals").and_then(|v| v.as_u64()) {
                    decimals = d as u8;
                }
            }
        }

        // If the owner has no token account, decimals are unknown. Fetch the
        // mint account directly to determine decimals so the display is sane.
        if accounts.is_empty() {
            decimals = self.fetch_spl_mint_decimals(mint).await.unwrap_or(0);
        }

        Ok(SplBalance {
            mint: mint.to_string(),
            owner: owner.to_string(),
            balance_raw: total.to_string(),
            balance_display: format_ft_amount(total, decimals),
            decimals,
            symbol: String::new(),
        })
    }

    /// Fetch a mint's decimals via `getAccountInfo` + base64 parse.
    pub async fn fetch_spl_mint_decimals(&self, mint: &str) -> Result<u8, String> {
        let result = self
            .call(
                "getAccountInfo",
                json!([mint, {"encoding": "base64", "commitment": "confirmed"}]),
            )
            .await?;
        let data = result
            .pointer("/value/data/0")
            .and_then(|v| v.as_str())
            .ok_or("getAccountInfo: missing data")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| format!("mint data b64: {e}"))?;
        // Mint layout: first 82 bytes.
        // [0..36]: mint authority (COption<Pubkey>)
        // [36..44]: supply (u64 le)
        // [44]: decimals (u8)
        bytes.get(44).copied().ok_or_else(|| "mint data short".to_string())
    }

    /// Check whether an account exists on-chain.
    pub async fn account_exists(&self, address: &str) -> Result<bool, String> {
        let result = self
            .call(
                "getAccountInfo",
                json!([address, {"encoding": "base64", "commitment": "confirmed"}]),
            )
            .await?;
        Ok(result.pointer("/value").map(|v| !v.is_null()).unwrap_or(false))
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
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

/// Format a raw SPL token amount using its `decimals`.
fn format_ft_amount(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let divisor: u128 = 10u128.pow(decimals as u32);
    let whole = raw / divisor;
    let frac = raw % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}
