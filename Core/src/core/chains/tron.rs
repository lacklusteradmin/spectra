//! Tron chain client.
//!
//! Uses the TronGrid / TronScan REST API.
//! Transactions are built using a protobuf-like manual encoding (Tron uses
//! protobuf for its RawData but the on-wire format for transfers is simple).
//! Signing uses secp256k1 with keccak256 (same key derivation as Ethereum,
//! but Tron addresses use Base58Check with version byte 0x41).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronBalance {
    /// SUN (1 TRX = 1_000_000 SUN).
    pub sun: u64,
    pub trx_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronHistoryEntry {
    pub txid: String,
    pub block_number: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_sun: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronSendResult {
    pub txid: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct TronClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl TronClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let path = path.to_string();
        let body = body.clone();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = body.clone();
            async move { client.post_json(&url, &body, RetryProfile::ChainRead).await }
        })
        .await
    }

    async fn get_json_path<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<TronBalance, String> {
        let resp = self
            .post("/wallet/getaccount", &json!({"address": address, "visible": true}))
            .await?;
        let sun = resp.get("balance").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(TronBalance {
            sun,
            trx_display: format_trx(sun),
        })
    }

    pub async fn fetch_latest_block(&self) -> Result<(u64, String), String> {
        let resp = self.post("/wallet/getnowblock", &json!({})).await?;
        let block_num = resp
            .pointer("/block_header/raw_data/number")
            .and_then(|v| v.as_u64())
            .ok_or("getnowblock: missing number")?;
        let timestamp = resp
            .pointer("/block_header/raw_data/timestamp")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        // blockID is the hash; use its hex string.
        let block_hash = resp
            .get("blockID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok((block_num, block_hash))
    }

    pub async fn fetch_history(
        &self,
        address: &str,
        api_base: &str,
    ) -> Result<Vec<TronHistoryEntry>, String> {
        // TronScan transactions API.
        let url = format!(
            "{}/api/transaction?sort=-timestamp&count=true&limit=50&address={}",
            api_base.trim_end_matches('/'),
            address
        );
        let resp: Value = self
            .client
            .get_json(&url, RetryProfile::ChainRead)
            .await?;
        let data = resp
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(data
            .into_iter()
            .map(|tx| {
                let txid = tx.get("hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let block_number = tx.get("block").and_then(|v| v.as_u64()).unwrap_or(0);
                let timestamp = tx.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
                let from = tx
                    .pointer("/contractData/owner_address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let to = tx
                    .pointer("/contractData/to_address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let amount_sun = tx
                    .pointer("/contractData/amount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let is_incoming = to.eq_ignore_ascii_case(address);
                TronHistoryEntry {
                    txid,
                    block_number,
                    timestamp,
                    from,
                    to,
                    amount_sun,
                    is_incoming,
                }
            })
            .collect())
    }

    /// Create, sign, and broadcast a TRX transfer.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sun: u64,
        private_key_bytes: &[u8],
    ) -> Result<TronSendResult, String> {
        // Step 1: Create unsigned transaction via /wallet/createtransaction.
        let resp = self
            .post(
                "/wallet/createtransaction",
                &json!({
                    "owner_address": from_address,
                    "to_address": to_address,
                    "amount": amount_sun,
                    "visible": true
                }),
            )
            .await?;

        // Extract the raw_data_hex for signing.
        let raw_data_hex = resp
            .get("raw_data_hex")
            .and_then(|v| v.as_str())
            .ok_or("createtransaction: missing raw_data_hex")?;
        let txid = resp
            .get("txID")
            .and_then(|v| v.as_str())
            .ok_or("createtransaction: missing txID")?
            .to_string();

        // Step 2: Sign txID (which is the sha256 of raw_data).
        let txid_bytes = hex::decode(&txid).map_err(|e| format!("txid hex: {e}"))?;
        let signature = sign_tron_hash(&txid_bytes, private_key_bytes)?;

        // Step 3: Broadcast.
        let mut broadcast_body = resp.clone();
        broadcast_body["signature"] = json!([signature]);
        let broadcast_resp = self
            .post("/wallet/broadcasttransaction", &broadcast_body)
            .await?;
        let result = broadcast_resp
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !result {
            let msg = broadcast_resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(format!("broadcast failed: {msg}"));
        }
        Ok(TronSendResult { txid })
    }
}

// ----------------------------------------------------------------
// Signing
// ----------------------------------------------------------------

fn sign_tron_hash(hash: &[u8], private_key_bytes: &[u8]) -> Result<String, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let msg = Message::from_digest_slice(hash).map_err(|e| format!("msg: {e}"))?;
    let (rec_id, sig) = secp
        .sign_ecdsa_recoverable(&msg, &secret_key)
        .serialize_compact();
    let mut out = sig.to_vec();
    out.push(rec_id.to_i32() as u8);
    Ok(hex::encode(&out))
}

// ----------------------------------------------------------------
// Address helpers
// ----------------------------------------------------------------

/// Derive a Tron address from a secp256k1 public key (uncompressed, 65 bytes).
pub fn pubkey_to_tron_address(pubkey_uncompressed: &[u8]) -> Result<String, String> {
    if pubkey_uncompressed.len() != 65 || pubkey_uncompressed[0] != 0x04 {
        return Err("expected 65-byte uncompressed public key".to_string());
    }
    let hash = keccak256(&pubkey_uncompressed[1..]);
    let addr_bytes = &hash[12..]; // last 20 bytes
    let mut versioned = vec![0x41u8]; // Tron mainnet prefix
    versioned.extend_from_slice(addr_bytes);
    Ok(bs58::encode(&versioned).with_check().into_string())
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut h = Keccak::v256();
    h.update(data);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

// ----------------------------------------------------------------
// Formatting / validation
// ----------------------------------------------------------------

fn format_trx(sun: u64) -> String {
    let whole = sun / 1_000_000;
    let frac = sun % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

pub fn validate_tron_address(address: &str) -> bool {
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && b[0] == 0x41)
        .unwrap_or(false)
}
