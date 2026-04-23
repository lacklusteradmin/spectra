//! Aptos chain client.
//!
//! Uses the Aptos REST API (api.mainnet.aptoslabs.com/v1).
//! Transactions use BCS serialization (Binary Canonical Serialization).
//! Signing uses Ed25519 via ed25519-dalek.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AptosBalance {
    /// Octas (1 APT = 100_000_000 octas).
    pub octas: u64,
    pub apt_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AptosHistoryEntry {
    pub txid: String,
    pub version: u64,
    pub timestamp_us: u64,
    pub from: String,
    pub to: String,
    pub amount_octas: u64,
    pub gas_used: u64,
    pub gas_unit_price: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AptosSendResult {
    pub txid: String,
    pub version: Option<u64>,
    /// JSON-encoded signed transaction body — stored for rebroadcast.
    pub signed_body_json: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct AptosClient {
    endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl AptosClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    async fn post_val(&self, path: &str, body: &Value) -> Result<Value, String> {
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
}
