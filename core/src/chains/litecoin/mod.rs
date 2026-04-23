//! Litecoin chain client.
//!
//! Litecoin supports both legacy P2PKH (L-addresses) and native SegWit P2WPKH
//! (ltc1q- addresses). Uses Blockbook REST API.
//! Network version byte: 0x30 (P2PKH), 0x32 (P2SH), bech32 HRP = "ltc".

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;

// ----------------------------------------------------------------
// Blockbook shared types (same shape as dogecoin)
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct BlockbookUtxo {
    pub(super) txid: String,
    pub(super) vout: u32,
    pub(super) value: String,
    #[serde(default)]
    pub(super) confirmations: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BlockbookAddress {
    pub(super) balance: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct BlockbookFeeEstimate {
    pub(super) result: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BlockbookTxList {
    #[serde(default)]
    pub(super) transactions: Vec<BlockbookTx>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BlockbookTx {
    pub(super) txid: String,
    pub(super) block_time: Option<u64>,
    pub(super) block_height: Option<u64>,
    #[serde(default)]
    pub(super) value: String,
    pub(super) fees: Option<String>,
    #[serde(default)]
    pub(super) vin: Vec<BlockbookVin>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BlockbookVin {
    pub(super) addresses: Option<Vec<String>>,
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtcBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtcUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtcHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    /// Net value change for the queried address. Negative = outgoing.
    pub amount_sat: i64,
    pub fee_sat: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtcSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct LitecoinClient {
    pub(super) endpoints: Vec<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl LitecoinClient {
    pub fn new(endpoints: Vec<String>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(super) async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}
