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

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;

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
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}
