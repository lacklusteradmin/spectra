//! Dogecoin chain client.
//!
//! Uses Blockbook-compatible REST API (same as most UTXO explorers).
//! Signing uses secp256k1 / P2PKH (Dogecoin does not support SegWit).
//! Network params: version byte 0x1e (addresses start with 'D').

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::sign_doge_p2pkh;

// ----------------------------------------------------------------
// Blockbook response types
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
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) unconfirmed_balance: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) txs: u64,
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
    pub(super) confirmations: u64,
    #[serde(default)]
    pub(super) value: String,
    pub(super) fees: Option<String>,
    #[allow(dead_code)]
    pub(super) vin: Vec<BlockbookVin>,
    pub(super) vout: Vec<BlockbookVout>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BlockbookVin {
    #[allow(dead_code)]
    pub(super) addresses: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BlockbookVout {
    pub(super) addresses: Option<Vec<String>>,
    #[allow(dead_code)]
    pub(super) value: Option<String>,
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogeBalance {
    /// Confirmed balance in koinus (1 DOGE = 100_000_000 koinus).
    pub balance_koin: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogeHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    pub amount_koin: i64, // negative = outgoing
    pub fee_koin: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogeUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_koin: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogeSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct DogecoinClient {
    pub(super) endpoints: Vec<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl DogecoinClient {
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
