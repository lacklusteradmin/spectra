//! Tron chain client.
//!
//! Uses the TronGrid / TronScan REST API.
//! Transactions are built using a protobuf-like manual encoding (Tron uses
//! protobuf for its RawData but the on-wire format for transfers is simple).
//! Signing uses secp256k1 with keccak256 (same key derivation as Ethereum,
//! but Tron addresses use Base58Check with version byte 0x41).

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
pub struct TronBalance {
    /// SUN (1 TRX = 1_000_000 SUN).
    pub sun: u64,
    pub trx_display: String,
}

/// Unified history entry covering both native TRX and TRC-20 token transfers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronTransfer {
    pub txid: String,
    pub block_number: u64,
    /// Milliseconds since epoch (TronScan convention).
    pub timestamp_ms: u64,
    pub from: String,
    pub to: String,
    /// Human-readable amount string ("1.5", "10.0", …).
    pub amount_display: String,
    /// "TRX" for native, token abbreviation (e.g. "USDT") for TRC-20.
    pub symbol: String,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronSendResult {
    pub txid: String,
    /// Full signed transaction JSON for rebroadcast. Serialized as a JSON string.
    #[serde(default)]
    pub signed_tx_json: String,
}

/// TRC-20 balance payload. Mirrors `Erc20Balance` so the Swift-side decoder
/// can share a single response type if desired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc20Balance {
    pub contract: String,
    pub holder: String,
    pub balance_raw: String,
    pub balance_display: String,
    pub decimals: u8,
    pub symbol: String,
}

/// Lightweight TRC-20 metadata (symbol + decimals).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc20Metadata {
    pub symbol: String,
    pub decimals: u8,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct TronClient {
    pub(super) endpoints: Vec<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl TronClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(super) async fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
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

    #[allow(dead_code)]
    pub(super) async fn get_json_path<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}
