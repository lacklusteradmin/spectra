//! NEAR Protocol chain client.
//!
//! Uses the NEAR JSON-RPC API for balance, nonce, block hash, history,
//! and transaction broadcast.
//! Transactions are BORSH-serialized and signed with Ed25519.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// JSON-RPC helper
// ----------------------------------------------------------------

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": "1", "method": method, "params": params })
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearBalance {
    /// yoctoNEAR (1 NEAR = 10^24 yoctoNEAR).
    pub yocto_near: String,
    pub near_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearHistoryEntry {
    pub txid: String,
    pub timestamp_ns: u64,
    pub signer_id: String,
    pub receiver_id: String,
    pub amount_yocto: String,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearSendResult {
    pub txid: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct NearClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl NearClient {
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
                    return Err(format!("near rpc error: {err}"));
                }
                resp.get("result")
                    .cloned()
                    .ok_or_else(|| "missing result".to_string())
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, account_id: &str) -> Result<NearBalance, String> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_account",
                    "finality": "final",
                    "account_id": account_id
                }),
            )
            .await?;
        let yocto = result
            .get("amount")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        let display = format_near(&yocto);
        Ok(NearBalance {
            yocto_near: yocto,
            near_display: display,
        })
    }

    pub async fn fetch_access_key_nonce(&self, account_id: &str, public_key_b58: &str) -> Result<u64, String> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_access_key",
                    "finality": "final",
                    "account_id": account_id,
                    "public_key": format!("ed25519:{public_key_b58}")
                }),
            )
            .await?;
        result
            .get("nonce")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "view_access_key: missing nonce".to_string())
    }

    pub async fn fetch_latest_block_hash(&self) -> Result<String, String> {
        let result = self
            .call("block", json!({"finality": "final"}))
            .await?;
        result
            .pointer("/header/hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "block: missing hash".to_string())
    }

    /// Fetch transaction history via NEAR Explorer API (indexer).
    pub async fn fetch_history(
        &self,
        account_id: &str,
        indexer_base: &str,
    ) -> Result<Vec<NearHistoryEntry>, String> {
        let url = format!(
            "{}/accounts/{}/activity?limit=50",
            indexer_base.trim_end_matches('/'),
            account_id
        );
        let items: Vec<Value> = self
            .client
            .get_json(&url, RetryProfile::ChainRead)
            .await
            .unwrap_or_default();

        Ok(items
            .into_iter()
            .map(|item| {
                let txid = item.get("transaction_hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let timestamp_ns: u64 = item
                    .get("block_timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let signer_id = item.get("signer_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let receiver_id = item.get("receiver_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let amount_yocto = item
                    .pointer("/args/deposit")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                let is_incoming = receiver_id == account_id;
                NearHistoryEntry {
                    txid,
                    timestamp_ns,
                    signer_id,
                    receiver_id,
                    amount_yocto,
                    is_incoming,
                }
            })
            .collect())
    }

    /// Sign and broadcast a NEAR Transfer transaction.
    pub async fn sign_and_broadcast(
        &self,
        from_account_id: &str,
        to_account_id: &str,
        yocto_near: u128,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<NearSendResult, String> {
        let public_key_b58 = bs58::encode(public_key_bytes).into_string();
        let nonce = self
            .fetch_access_key_nonce(from_account_id, &public_key_b58)
            .await?
            + 1;
        let block_hash = self.fetch_latest_block_hash().await?;
        let block_hash_bytes = bs58::decode(&block_hash)
            .into_vec()
            .map_err(|e| format!("block hash decode: {e}"))?;
        if block_hash_bytes.len() != 32 {
            return Err("block hash wrong length".to_string());
        }
        let block_hash_arr: [u8; 32] = block_hash_bytes.try_into().unwrap();

        let tx_bytes = build_near_transfer_tx(
            from_account_id,
            public_key_bytes,
            nonce,
            to_account_id,
            yocto_near,
            &block_hash_arr,
            private_key_bytes,
        )?;

        use base64::Engine;
        let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

        let result = self
            .call("broadcast_tx_commit", json!([tx_b64]))
            .await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .ok_or("broadcast: missing hash")?
            .to_string();
        Ok(NearSendResult { txid })
    }
}

// ----------------------------------------------------------------
// NEAR transaction builder (BORSH)
// ----------------------------------------------------------------

/// Build a signed NEAR Transfer transaction.
pub fn build_near_transfer_tx(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
    private_key: &[u8; 64],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    // BORSH-encode the transaction.
    let tx = borsh_encode_transfer(signer_id, public_key, nonce, receiver_id, yocto_amount, block_hash);

    // Hash the transaction for signing.
    let tx_hash: [u8; 32] = Sha256::digest(&tx).into();

    let signing_key = SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature
    // Signature in NEAR is: [key_type(4)] + [sig(64)]
    let mut signed = tx;
    signed.extend_from_slice(&0u32.to_le_bytes()); // key type = ED25519
    signed.extend_from_slice(signature.to_bytes().as_ref());

    Ok(signed)
}

/// BORSH-encode a NEAR Transfer transaction.
fn borsh_encode_transfer(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::new();

    // signer_id: string (u32 len + bytes)
    borsh_string(&mut out, signer_id);
    // public_key: key_type(u32) + bytes(32)
    out.extend_from_slice(&0u32.to_le_bytes()); // ED25519
    out.extend_from_slice(public_key);
    // nonce: u64
    out.extend_from_slice(&nonce.to_le_bytes());
    // receiver_id: string
    borsh_string(&mut out, receiver_id);
    // block_hash: [u8; 32]
    out.extend_from_slice(block_hash);
    // actions: array (u32 len)
    out.extend_from_slice(&1u32.to_le_bytes()); // 1 action
    // Action::Transfer = variant 3
    out.push(3u8);
    // Transfer.deposit: u128
    out.extend_from_slice(&yocto_amount.to_le_bytes());

    out
}

fn borsh_string(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_near(yocto: &str) -> String {
    // yocto is a 25-digit decimal; divide by 10^24 for NEAR.
    let n: u128 = yocto.parse().unwrap_or(0);
    let divisor: u128 = 1_000_000_000_000_000_000_000_000; // 10^24
    let whole = n / divisor;
    let frac = n % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:024}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

pub fn validate_near_address(address: &str) -> bool {
    // NEAR accounts: named (alice.near, sub.alice.near) or implicit (64 hex chars).
    if address.len() == 64 && address.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    // Named account: 2-64 chars, alphanumeric, hyphen, underscore, dot.
    !address.is_empty()
        && address.len() <= 64
        && address
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}
