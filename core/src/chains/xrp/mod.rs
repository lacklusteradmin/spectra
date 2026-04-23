//! XRP (Ripple) chain client.
//!
//! Uses the XRP Ledger JSON-RPC / REST API (rippled / Clio).
//! Transactions are serialized using XRP's binary codec (STObject)
//! and signed with secp256k1.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::build_signed_payment;

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpBalance {
    /// XRP drops (1 XRP = 1_000_000 drops).
    pub drops: u64,
    pub xrp_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpHistoryEntry {
    pub txid: String,
    pub ledger_index: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_drops: u64,
    pub fee_drops: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpSendResult {
    pub txid: String,
    /// Signed tx blob hex — stored for rebroadcast.
    pub tx_blob_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct XrpClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl XrpClient {
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
                let result = resp
                    .get("result")
                    .ok_or_else(|| "missing result".to_string())?;
                if let Some(status) = result.get("status").and_then(|s| s.as_str()) {
                    if status == "error" {
                        let msg = result
                            .get("error_message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown error");
                        return Err(format!("xrp rpc error: {msg}"));
                    }
                }
                Ok(result.clone())
            }
        })
        .await
    }
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": [params] })
}
