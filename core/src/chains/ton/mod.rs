//! TON (The Open Network) chain client.
//!
//! Uses the TON Center REST API (toncenter.com/api/v2).
//! Signing uses Ed25519 (ed25519-dalek).
//! TON cells are complex; for transfers we use the tonlib-compatible
//! approach of sending via the `walletv4r2` contract message format.

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::build_wallet_v4r2_transfer;

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonBalance {
    /// Nanotons (1 TON = 1_000_000_000 nanotons).
    pub nanotons: u64,
    pub ton_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonHistoryEntry {
    pub txid: String,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_nanotons: u64,
    pub fee_nanotons: u64,
    pub is_incoming: bool,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonSendResult {
    pub message_hash: String,
    /// Base64-encoded BOC — stored for rebroadcast.
    pub boc_b64: String,
}

/// One jetton (token) balance entry returned by the v3 API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonJettonBalance {
    /// Jetton master contract address (matches the tracked-token `contract` field).
    pub master_address: String,
    /// Jetton wallet contract address (holder's personal wallet for this token).
    pub wallet_address: String,
    /// Raw balance in the token's smallest unit.
    pub balance_raw: u128,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct TonClient {
    pub(super) endpoints: Vec<String>,
    pub(super) v3_endpoints: Vec<String>,
    pub(super) api_key: Option<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl TonClient {
    pub fn new(endpoints: Vec<String>, api_key: Option<String>) -> Self {
        Self {
            endpoints,
            v3_endpoints: vec![],
            api_key,
            client: HttpClient::shared(),
        }
    }

    pub fn with_v3_endpoints(mut self, v3_endpoints: Vec<String>) -> Self {
        self.v3_endpoints = v3_endpoints;
        self
    }

    pub(super) async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        let api_key = self.api_key.clone();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let api_key = api_key.clone();
            let mut url = format!("{}{}", base.trim_end_matches('/'), path);
            if let Some(key) = &api_key {
                url.push_str(&format!("&api_key={key}"));
            }
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    /// GET from the TonCenter v3 base URL (if configured).
    pub(super) async fn get_v3<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        if self.v3_endpoints.is_empty() {
            return Err("ton: no v3 endpoints configured".to_string());
        }
        let path = path.to_string();
        let api_key = self.api_key.clone();
        with_fallback(&self.v3_endpoints, |base| {
            let client = self.client.clone();
            let api_key = api_key.clone();
            let mut url = format!("{}{}", base.trim_end_matches('/'), path);
            if let Some(key) = &api_key {
                // v3 uses query param `api_key` as well
                if url.contains('?') {
                    url.push_str(&format!("&api_key={key}"));
                } else {
                    url.push_str(&format!("?api_key={key}"));
                }
            }
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}
