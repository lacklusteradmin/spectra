//! Polkadot / Substrate chain client.
//!
//! Uses the Subscan REST API for balance and history.
//! For transaction building, uses the SCALE codec (minimal subset)
//! with the Polkadot RPC for nonce, runtime version, genesis hash.
//! Signing uses Sr25519 via the `schnorrkel` crate — however, since
//! that crate is not in our Cargo.toml, we sign with Ed25519 via
//! ed25519-dalek (which Substrate also supports via the `ed25519`
//! MultiSignature variant). Production wallets typically use Sr25519;
//! we use Ed25519 here as it matches our existing dependency set.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::build_signed_transfer;

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotBalance {
    /// Planck (1 DOT = 10^10 planck).
    pub planck: u128,
    pub dot_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotHistoryEntry {
    pub txid: String,
    pub block_num: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_planck: u128,
    pub fee_planck: u128,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotSendResult {
    pub txid: String,
    /// Hex-encoded signed extrinsic (0x-prefixed) — stored for rebroadcast.
    pub extrinsic_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct PolkadotClient {
    /// Polkadot RPC endpoints (wss:// or https://).
    pub(super) rpc_endpoints: Vec<String>,
    /// Subscan API endpoints (https://polkadot.api.subscan.io).
    pub(super) subscan_endpoints: Vec<String>,
    pub(super) subscan_api_key: Option<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl PolkadotClient {
    pub fn new(
        rpc_endpoints: Vec<String>,
        subscan_endpoints: Vec<String>,
        subscan_api_key: Option<String>,
    ) -> Self {
        Self {
            rpc_endpoints,
            subscan_endpoints,
            subscan_api_key,
            client: HttpClient::shared(),
        }
    }

    pub(super) async fn rpc_call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        with_fallback(&self.rpc_endpoints, |url| {
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

    pub(super) async fn subscan_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, String> {
        let path = path.to_string();
        let body = body.clone();
        let api_key = self.subscan_api_key.clone();
        with_fallback(&self.subscan_endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = body.clone();
            let api_key = api_key.clone();
            async move {
                let mut headers = std::collections::HashMap::new();
                if let Some(key) = &api_key {
                    headers.insert("X-API-Key", key.as_str());
                }
                let resp: Value = client
                    .post_json_with_headers(&url, &body, &headers, RetryProfile::ChainRead)
                    .await?;
                let data = resp.get("data").cloned().unwrap_or(resp);
                serde_json::from_value(data).map_err(|e| format!("parse: {e}"))
            }
        })
        .await
    }
}
