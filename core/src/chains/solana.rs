//! Solana chain client.
//!
//! Uses the Solana JSON-RPC API for balance, history, and broadcast.
//! Transaction serialization follows the compact (v0) wire format:
//!   [signatures] [message header] [accounts] [recent_blockhash] [instructions]
//!
//! Ed25519 signing is performed using the `ed25519-dalek` crate.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use base64::Engine as _;

use crate::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// JSON-RPC helper
// ----------------------------------------------------------------

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

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

// ----------------------------------------------------------------
// Solana client
// ----------------------------------------------------------------

pub struct SolanaClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl SolanaClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = rpc(method, params);
        with_fallback(&self.endpoints, |url| {
            let client = self.client.clone();
            let body = body.clone();
            async move {
                let resp: Value = client
                    .post_json(&url, &body, RetryProfile::ChainRead)
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

    // ----------------------------------------------------------------
    // Fetch
    // ----------------------------------------------------------------

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
    /// For each mint, calls `getTokenAccountsByOwner` filtered by that mint.
    /// Returns one entry per mint that has a token account (mints with no
    /// account are silently omitted).
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
            let timestamp = tx
                .get("blockTime")
                .and_then(|v| v.as_i64());
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
    /// native SOL and SPL token transfers. For each transaction, if SPL token
    /// balance deltas are found for `address`, those are emitted as separate
    /// entries (one per mint). Native SOL entries with a zero delta are
    /// suppressed when SPL entries are present for the same signature.
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
                    .find(|e| e.get("accountIndex").and_then(|v| v.as_u64()).unwrap_or(u64::MAX) == acct_idx)
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
                    symbol: mint.clone(), // Swift resolves mint → symbol from token registry
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

    // ----------------------------------------------------------------
    // Send
    // ----------------------------------------------------------------

    /// Sign and broadcast a native SOL transfer.
    pub async fn sign_and_broadcast(
        &self,
        from_pubkey_bytes: &[u8; 32],
        to_address: &str,
        lamports: u64,
        private_key_bytes: &[u8; 64],
    ) -> Result<SolanaSendResult, String> {
        let blockhash = self.fetch_recent_blockhash().await?;
        let to_pubkey = bs58::decode(to_address)
            .into_vec()
            .map_err(|e| format!("invalid to address: {e}"))?;
        if to_pubkey.len() != 32 {
            return Err(format!("invalid to pubkey length: {}", to_pubkey.len()));
        }
        let to_pubkey: [u8; 32] = to_pubkey.try_into().unwrap();

        let raw_tx = build_sol_transfer(
            from_pubkey_bytes,
            &to_pubkey,
            lamports,
            &blockhash,
            private_key_bytes,
        )?;

        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw_tx);
        let result = self
            .call(
                "sendTransaction",
                json!([encoded.clone(), {"encoding": "base64", "preflightCommitment": "confirmed"}]),
            )
            .await?;
        let signature = result
            .as_str()
            .ok_or("sendTransaction: expected string")?
            .to_string();
        Ok(SolanaSendResult { signature, signed_tx_base64: encoded })
    }

    // ----------------------------------------------------------------
    // SPL Token (fungible token) support
    // ----------------------------------------------------------------

    /// Fetch the SPL token balance for `owner` holding `mint`. Uses
    /// `getTokenAccountsByOwner` (parsed) so we can read both the raw
    /// amount and the decimals directly from the parsed account data.
    pub async fn fetch_spl_balance(
        &self,
        mint: &str,
        owner: &str,
    ) -> Result<SplBalance, String> {
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
        // In practice there's usually exactly one (the ATA).
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

    /// Fetch a mint's decimals via `getAccountInfo` + base64 parse. The
    /// SPL mint layout places the `decimals` byte at offset 44.
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

    /// Sign and broadcast an SPL token transfer. Derives the source and
    /// destination associated token accounts; if the destination ATA does
    /// not exist yet the transaction prepends a Create-Idempotent
    /// instruction so it is materialized in the same atomic tx.
    #[allow(clippy::too_many_arguments)]
    pub async fn sign_and_broadcast_spl(
        &self,
        from_owner_pubkey: &[u8; 32],
        to_owner_b58: &str,
        mint_b58: &str,
        amount_raw: u64,
        decimals: u8,
        private_key_bytes: &[u8; 64],
    ) -> Result<SolanaSendResult, String> {
        let to_owner = decode_b58_32(to_owner_b58)?;
        let mint = decode_b58_32(mint_b58)?;

        let source_ata = derive_associated_token_account(from_owner_pubkey, &mint)?;
        let dest_ata = derive_associated_token_account(&to_owner, &mint)?;

        let source_ata_b58 = bs58::encode(&source_ata).into_string();
        let dest_ata_b58 = bs58::encode(&dest_ata).into_string();

        // Destination ATA may not exist yet; we always emit the idempotent
        // create instruction so it's a no-op if the account already exists.
        let _dest_exists = self
            .account_exists(&dest_ata_b58)
            .await
            .unwrap_or(false);

        let blockhash = self.fetch_recent_blockhash().await?;
        let raw_tx = build_spl_transfer_checked(
            from_owner_pubkey,
            &to_owner,
            &mint,
            &source_ata,
            &dest_ata,
            amount_raw,
            decimals,
            &blockhash,
            private_key_bytes,
        )?;

        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw_tx);
        let result = self
            .call(
                "sendTransaction",
                json!([encoded.clone(), {"encoding": "base64", "preflightCommitment": "confirmed"}]),
            )
            .await?;
        let signature = result
            .as_str()
            .ok_or("sendTransaction: expected string")?
            .to_string();
        // Reference the source_ata for potential logging in future.
        let _ = source_ata_b58;
        Ok(SolanaSendResult { signature, signed_tx_base64: encoded })
    }

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
        Ok(SolanaSendResult { signature, signed_tx_base64: signed_tx_base64.to_string() })
    }
}

// ----------------------------------------------------------------
// Transaction builder
// ----------------------------------------------------------------

/// Build a signed Solana legacy transaction for a native SOL transfer.
///
/// Wire format (legacy):
///   compact_u16(num_sigs) || sig[0..64] || message_bytes
///
/// Message:
///   [header: 3 bytes] [compact_u16(num_accounts)] [accounts..] [blockhash: 32]
///   [compact_u16(num_instructions)] [instruction: program_id_idx | compact_u16(accounts) | compact_u16(data)]
pub fn build_sol_transfer(
    from: &[u8; 32],
    to: &[u8; 32],
    lamports: u64,
    recent_blockhash_b58: &str,
    private_key: &[u8; 64],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};

    let blockhash_bytes = bs58::decode(recent_blockhash_b58)
        .into_vec()
        .map_err(|e| format!("invalid blockhash: {e}"))?;
    if blockhash_bytes.len() != 32 {
        return Err("blockhash must be 32 bytes".to_string());
    }
    let blockhash: [u8; 32] = blockhash_bytes.try_into().unwrap();

    // System program ID (all zeros except last byte = 0).
    let system_program: [u8; 32] = [0u8; 32];

    // Accounts: [from (signer+writable), to (writable), system_program]
    // Header: [num_required_signatures=1, num_readonly_signed=0, num_readonly_unsigned=1]
    let header = [1u8, 0u8, 1u8];

    // Build message.
    let mut msg = Vec::new();
    msg.extend_from_slice(&header);
    // Account list (3 accounts).
    msg.extend_from_slice(&compact_u16(3));
    msg.extend_from_slice(from);
    msg.extend_from_slice(to);
    msg.extend_from_slice(&system_program);
    // Recent blockhash.
    msg.extend_from_slice(&blockhash);
    // Instructions (1).
    msg.extend_from_slice(&compact_u16(1));
    // Instruction: program id index = 2 (system program).
    msg.push(2u8);
    // Account indices: [0 (from), 1 (to)].
    msg.extend_from_slice(&compact_u16(2));
    msg.push(0u8); // from index
    msg.push(1u8); // to index
    // Data: SystemInstruction::Transfer = [2,0,0,0] + lamports as le u64
    let mut data = Vec::new();
    data.extend_from_slice(&2u32.to_le_bytes()); // Transfer instruction
    data.extend_from_slice(&lamports.to_le_bytes());
    msg.extend_from_slice(&compact_u16(data.len()));
    msg.extend_from_slice(&data);

    // Sign.
    let signing_key = SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&msg);

    // Serialize: compact_u16(1) || sig || message
    let mut tx = Vec::new();
    tx.extend_from_slice(&compact_u16(1)); // 1 signature
    tx.extend_from_slice(signature.to_bytes().as_ref());
    tx.extend_from_slice(&msg);

    Ok(tx)
}

// ----------------------------------------------------------------
// SPL helpers: ATA derivation and SPL Transfer transaction builder
// ----------------------------------------------------------------

/// SPL Token program id (decoded base58).
pub const SPL_TOKEN_PROGRAM_ID: [u8; 32] = [
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133,
    237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
];

/// Associated Token Account program id (decoded base58).
pub const ASSOCIATED_TOKEN_PROGRAM_ID: [u8; 32] = [
    140, 151, 37, 143, 78, 36, 137, 241, 187, 61, 16, 41, 20, 142, 13, 131, 11, 90, 19, 153, 218,
    255, 16, 132, 4, 142, 123, 216, 219, 233, 248, 89,
];

/// Decode a base58 Solana address into a 32-byte pubkey.
fn decode_b58_32(b58: &str) -> Result<[u8; 32], String> {
    let bytes = bs58::decode(b58)
        .into_vec()
        .map_err(|e| format!("b58 decode {b58}: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("b58 {b58} not 32 bytes: {}", v.len()))
}

/// Derive the Associated Token Account for a (wallet, mint) pair.
///
/// PDA seeds = [wallet, TOKEN_PROGRAM_ID, mint], program = ASSOCIATED_TOKEN_PROGRAM_ID.
pub fn derive_associated_token_account(
    wallet: &[u8; 32],
    mint: &[u8; 32],
) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    let seeds: [&[u8]; 3] = [wallet, &SPL_TOKEN_PROGRAM_ID, mint];
    // Brute-force the bump seed from 255 down until we find an off-curve point.
    for bump in (0u8..=255u8).rev() {
        let mut h = Sha256::new();
        for s in seeds.iter() {
            h.update(s);
        }
        h.update([bump]);
        h.update(ASSOCIATED_TOKEN_PROGRAM_ID);
        h.update(b"ProgramDerivedAddress");
        let digest: [u8; 32] = h.finalize().into();
        if is_off_curve(&digest) {
            return Ok(digest);
        }
    }
    Err("failed to find PDA bump".to_string())
}

/// An ed25519 point is "off-curve" if CompressedEdwardsY::decompress returns None.
/// PDAs are valid only when the resulting point is off-curve (so they cannot
/// coincide with a real pubkey).
fn is_off_curve(bytes: &[u8; 32]) -> bool {
    use curve25519_dalek::edwards::CompressedEdwardsY;
    CompressedEdwardsY::from_slice(bytes)
        .ok()
        .and_then(|p| p.decompress())
        .is_none()
}

/// Build a signed Solana legacy transaction that
///   1. Issues an Associated Token Account Create-Idempotent instruction
///      so the destination ATA is materialized if needed,
///   2. Issues an SPL Token `TransferChecked` instruction for the transfer.
///
/// All instructions are authorized by the source wallet (single signer).
#[allow(clippy::too_many_arguments)]
pub fn build_spl_transfer_checked(
    from_owner: &[u8; 32],
    to_owner: &[u8; 32],
    mint: &[u8; 32],
    source_ata: &[u8; 32],
    dest_ata: &[u8; 32],
    amount_raw: u64,
    decimals: u8,
    recent_blockhash_b58: &str,
    private_key: &[u8; 64],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};

    let blockhash_bytes = bs58::decode(recent_blockhash_b58)
        .into_vec()
        .map_err(|e| format!("invalid blockhash: {e}"))?;
    if blockhash_bytes.len() != 32 {
        return Err("blockhash must be 32 bytes".to_string());
    }
    let blockhash: [u8; 32] = blockhash_bytes.try_into().unwrap();

    // Account list order (all distinct pubkeys used anywhere):
    //   index 0: from_owner       (signer + writable, pays fees & funds ATA)
    //   index 1: dest_ata         (writable)
    //   index 2: source_ata       (writable)
    //   index 3: to_owner         (readonly)  — ATA create needs the wallet
    //   index 4: mint             (readonly)
    //   index 5: system_program   (readonly)
    //   index 6: spl_token        (readonly)
    //   index 7: ata_program      (readonly)
    //
    // Header:
    //   num_required_signatures = 1
    //   num_readonly_signed     = 0
    //   num_readonly_unsigned   = 5 (to_owner, mint, system, spl, ata)

    let system_program: [u8; 32] = [0u8; 32];
    let accounts: [&[u8; 32]; 8] = [
        from_owner,
        dest_ata,
        source_ata,
        to_owner,
        mint,
        &system_program,
        &SPL_TOKEN_PROGRAM_ID,
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    ];

    let header = [1u8, 0u8, 5u8];

    let mut msg = Vec::new();
    msg.extend_from_slice(&header);
    msg.extend_from_slice(&compact_u16(accounts.len()));
    for a in &accounts {
        msg.extend_from_slice(a.as_ref());
    }
    msg.extend_from_slice(&blockhash);

    // 2 instructions.
    msg.extend_from_slice(&compact_u16(2));

    // -- Instruction 1: CreateIdempotent on ATA program --------------------
    // Program id: ata_program (index 7)
    // Accounts (per associated-token-account spec):
    //   0: payer (signer+writable)   = from_owner (idx 0)
    //   1: ata (writable)            = dest_ata    (idx 1)
    //   2: wallet (readonly)         = to_owner    (idx 3)
    //   3: mint (readonly)           = mint        (idx 4)
    //   4: system program (readonly) = system      (idx 5)
    //   5: spl token (readonly)      = spl_token   (idx 6)
    // Data: single byte tag 1 = CreateIdempotent
    msg.push(7u8); // program id index (ata program)
    let ata_accts: [u8; 6] = [0, 1, 3, 4, 5, 6];
    msg.extend_from_slice(&compact_u16(ata_accts.len()));
    msg.extend_from_slice(&ata_accts);
    let ata_data: [u8; 1] = [1u8];
    msg.extend_from_slice(&compact_u16(ata_data.len()));
    msg.extend_from_slice(&ata_data);

    // -- Instruction 2: SPL TransferChecked ------------------------------
    // Program id: spl token (index 6)
    // Accounts:
    //   0: source ata (writable)   = source_ata (idx 2)
    //   1: mint (readonly)         = mint       (idx 4)
    //   2: dest ata (writable)     = dest_ata   (idx 1)
    //   3: authority (signer)      = from_owner (idx 0)
    // Data: [12, amount: u64 LE, decimals: u8]
    msg.push(6u8); // program id index (spl token)
    let xfer_accts: [u8; 4] = [2, 4, 1, 0];
    msg.extend_from_slice(&compact_u16(xfer_accts.len()));
    msg.extend_from_slice(&xfer_accts);
    let mut xfer_data = Vec::with_capacity(1 + 8 + 1);
    xfer_data.push(12u8); // TransferChecked
    xfer_data.extend_from_slice(&amount_raw.to_le_bytes());
    xfer_data.push(decimals);
    msg.extend_from_slice(&compact_u16(xfer_data.len()));
    msg.extend_from_slice(&xfer_data);

    // Sign the message bytes.
    let signing_key = SigningKey::from_bytes(
        &private_key[..32].try_into().map_err(|_| "privkey too short")?,
    );
    let signature = signing_key.sign(&msg);

    let mut tx = Vec::new();
    tx.extend_from_slice(&compact_u16(1)); // 1 signature
    tx.extend_from_slice(signature.to_bytes().as_ref());
    tx.extend_from_slice(&msg);
    Ok(tx)
}

/// Solana compact-u16 encoding.
fn compact_u16(val: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut v = val as u16;
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
    out
}

// ----------------------------------------------------------------
// Formatting
// ----------------------------------------------------------------

fn format_lamports(lamports: u64) -> String { format_sol(lamports) }

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

// ----------------------------------------------------------------
// Address validation
// ----------------------------------------------------------------

/// Solana addresses are base58-encoded 32-byte Ed25519 public keys.
pub fn validate_solana_address(address: &str) -> bool {
    match bs58::decode(address).into_vec() {
        Ok(bytes) => bytes.len() == 32,
        Err(_) => false,
    }
}
