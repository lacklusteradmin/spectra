//! NEAR Protocol chain client.
//!
//! Uses the NEAR JSON-RPC API for balance, nonce, block hash, history,
//! and transaction broadcast.
//! Transactions are BORSH-serialized and signed with Ed25519.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use fetch::near_parse_history_response;
pub use send::{build_near_function_call_tx, build_near_transfer_tx};

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
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
    /// Base64-encoded signed transaction — stored for rebroadcast.
    pub signed_tx_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearFtBalance {
    pub contract: String,
    pub holder: String,
    pub balance_raw: String,
    pub balance_display: String,
    pub decimals: u8,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearFtMetadata {
    pub spec: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

// ----------------------------------------------------------------
// UniFFI-exported history parsed snapshot
// ----------------------------------------------------------------

#[derive(Debug, Clone, uniffi::Record)]
pub struct NearHistoryParsedSnapshot {
    pub transaction_hash: String,
    /// "send" or "receive"
    pub kind: String,
    pub amount_near: f64,
    pub counterparty_address: String,
    /// Unix seconds (0 = fall back to "now" on the Swift side).
    pub created_at_unix_seconds: f64,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct NearClient {
    pub(super) endpoints: Vec<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl NearClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(super) async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
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
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": "1", "method": method, "params": params })
}
