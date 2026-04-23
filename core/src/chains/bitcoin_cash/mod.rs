//! Bitcoin Cash chain client.
//!
//! BCH uses the CashAddr address format (prefix "bitcoincash:") but can also
//! accept legacy P2PKH addresses (version 0x00, same as BTC). Signing is
//! SIGHASH_ALL with replay protection (BIP143 SegWit-style digest for BCH
//! is NOT used; BCH uses its own SIGHASH_FORKID = 0x40).
//!
//! We use Blockbook for balance/UTXO/broadcast.

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::sign_bch_tx;

// ----------------------------------------------------------------
// Blockbook types
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
pub struct BchBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    /// Net value change for the queried address. Negative = outgoing.
    pub amount_sat: i64,
    pub fee_sat: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BchSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct BitcoinCashClient {
    pub(super) endpoints: Vec<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl BitcoinCashClient {
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
