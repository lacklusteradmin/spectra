//! Sui chain client.
//!
//! Uses the Sui JSON-RPC API (sui_getBalance, sui_getCoins,
//! sui_queryTransactionBlocks, unsafe_transferSui / sui_executeTransactionBlock).
//! Signing uses Ed25519 via ed25519-dalek.
//! Sui addresses are 32-byte Blake2b-256 hashes of the public key,
//! prefixed with a flag byte (0x00 for Ed25519).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};



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
    /// Base64 tx bytes — stored for rebroadcast.
    pub tx_bytes_b64: String,
    /// Base64 signature — stored for rebroadcast.
    pub sig_b64: String,
    pub digest: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct SuiClient {
    endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl SuiClient {
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
                    return Err(format!("sui rpc error: {err}"));
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
// Sui fetch paths: native balance, per-coin balance, history.



impl SuiClient {
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

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<SuiHistoryEntry>, String> {
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
}

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
