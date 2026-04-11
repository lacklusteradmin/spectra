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

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// JSON-RPC helper
// ----------------------------------------------------------------

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaSendResult {
    pub signature: String,
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
                json!([encoded, {"encoding": "base64", "preflightCommitment": "confirmed"}]),
            )
            .await?;
        let signature = result
            .as_str()
            .ok_or("sendTransaction: expected string")?
            .to_string();
        Ok(SolanaSendResult { signature })
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
