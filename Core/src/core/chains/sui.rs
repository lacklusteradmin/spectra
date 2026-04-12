//! Sui chain client.
//!
//! Uses the Sui JSON-RPC API (sui_getBalance, sui_getCoins,
//! sui_queryTransactionBlocks, unsafe_transferSui / sui_executeTransactionBlock).
//! Signing uses Ed25519 via ed25519-dalek.
//! Sui addresses are 32-byte Blake2b-256 hashes of the public key,
//! prefixed with a flag byte (0x00 for Ed25519).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
pub struct SuiBalance {
    /// MIST (1 SUI = 1_000_000_000 MIST).
    pub mist: u64,
    pub sui_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiHistoryEntry {
    pub digest: String,
    pub timestamp_ms: u64,
    pub is_incoming: bool,
    pub amount_mist: u64,
    pub gas_mist: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiSendResult {
    pub digest: String,
    /// Base64 tx bytes — stored for rebroadcast.
    pub tx_bytes_b64: String,
    /// Base64 signature — stored for rebroadcast.
    pub sig_b64: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct SuiClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl SuiClient {
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
                    return Err(format!("sui rpc error: {err}"));
                }
                resp.get("result")
                    .cloned()
                    .ok_or_else(|| "missing result".to_string())
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<SuiBalance, String> {
        let result = self
            .call("suix_getBalance", json!([address, "0x2::sui::SUI"]))
            .await?;
        let mist: u64 = result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or("suix_getBalance: missing totalBalance")?;
        Ok(SuiBalance {
            mist,
            sui_display: format_sui(mist),
        })
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<SuiHistoryEntry>, String> {
        let result = self
            .call(
                "suix_queryTransactionBlocks",
                json!([
                    {"ToAddress": address},
                    null,
                    20,
                    true
                ]),
            )
            .await
            .unwrap_or(json!({"data": []}));

        let data = result
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(data
            .into_iter()
            .map(|item| {
                let digest = item.get("digest").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let timestamp_ms = item
                    .get("timestampMs")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                SuiHistoryEntry {
                    digest,
                    timestamp_ms,
                    is_incoming: true,
                    amount_mist: 0,
                    gas_mist: 0,
                }
            })
            .collect())
    }

    /// Fetch the balance for a specific coin type (e.g. `0x5d4b...::coin::COIN`).
    /// Returns the raw balance in the coin's smallest unit.
    pub async fn fetch_coin_balance(&self, address: &str, coin_type: &str) -> Result<u64, String> {
        let result = self
            .call("suix_getBalance", json!([address, coin_type]))
            .await?;
        result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("suix_getBalance: missing totalBalance for {coin_type}"))
    }

    /// Request an unsigned transfer transaction, sign it, and execute.
    pub async fn sign_and_send(
        &self,
        from_address: &str,
        to_address: &str,
        mist: u64,
        gas_budget: u64,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<SuiSendResult, String> {
        // Build an unsafe transfer (node constructs the tx bytes).
        let tx_result = self
            .call(
                "unsafe_transferSui",
                json!([from_address, to_address, gas_budget.to_string(), mist.to_string()]),
            )
            .await?;

        let tx_bytes_b64 = tx_result
            .get("txBytes")
            .and_then(|v| v.as_str())
            .ok_or("unsafe_transferSui: missing txBytes")?;

        use base64::Engine;
        let tx_bytes = base64::engine::general_purpose::STANDARD
            .decode(tx_bytes_b64)
            .map_err(|e| format!("b64 decode: {e}"))?;

        // Signing: intent prefix [0,0,0] + tx_bytes.
        let mut signing_payload = vec![0u8, 0u8, 0u8];
        signing_payload.extend_from_slice(&tx_bytes);

        use ed25519_dalek::{Signer, SigningKey};
        use sha2::{Digest, Sha256};
        let digest: [u8; 32] = Sha256::digest(&signing_payload).into();
        // ed25519-dalek SigningKey::from_bytes takes the 32-byte seed (first half of the 64-byte keypair).
        let seed: [u8; 32] = private_key_bytes[..32].try_into().map_err(|_| "privkey too short")?;
        let signing_key = SigningKey::from_bytes(&seed);
        let signature = signing_key.sign(&digest);

        // Sui signature format: [flag(1)] + [sig(64)] + [pk(32)], base64.
        let mut sig_bytes = vec![0x00u8]; // Ed25519 flag
        sig_bytes.extend_from_slice(signature.to_bytes().as_ref());
        sig_bytes.extend_from_slice(public_key_bytes);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&sig_bytes);

        let execute_result = self
            .call(
                "sui_executeTransactionBlock",
                json!([tx_bytes_b64, [sig_b64], {"showEffects": true}, "WaitForLocalExecution"]),
            )
            .await?;

        let digest = execute_result
            .get("digest")
            .and_then(|v| v.as_str())
            .ok_or("executeTransactionBlock: missing digest")?
            .to_string();

        Ok(SuiSendResult { digest, tx_bytes_b64: tx_bytes_b64.to_string(), sig_b64 })
    }

    /// Execute a pre-signed transaction block (for rebroadcast).
    pub async fn execute_signed_tx(&self, tx_bytes_b64: &str, sig_b64: &str) -> Result<SuiSendResult, String> {
        let execute_result = self
            .call(
                "sui_executeTransactionBlock",
                json!([tx_bytes_b64, [sig_b64], {"showEffects": true}, "WaitForLocalExecution"]),
            )
            .await?;
        let digest = execute_result
            .get("digest")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(SuiSendResult { digest, tx_bytes_b64: tx_bytes_b64.to_string(), sig_b64: sig_b64.to_string() })
    }
}

// ----------------------------------------------------------------
// Address derivation
// ----------------------------------------------------------------

/// Derive a Sui address from an Ed25519 public key.
pub fn pubkey_to_sui_address(public_key: &[u8; 32]) -> String {
    use blake2::{Blake2b, Digest};
    use blake2::digest::consts::U32;
    let mut input = vec![0x00u8]; // Ed25519 flag
    input.extend_from_slice(public_key);
    let mut h = Blake2b::<U32>::new();
    h.update(&input);
    let hash = h.finalize();
    format!("0x{}", hex::encode(&hash[..]))
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_sui(mist: u64) -> String {
    let whole = mist / 1_000_000_000;
    let frac = mist % 1_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:09}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

pub fn validate_sui_address(address: &str) -> bool {
    let s = address.strip_prefix("0x").unwrap_or(address);
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}
