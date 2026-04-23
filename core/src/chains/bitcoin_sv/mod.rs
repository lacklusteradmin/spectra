//! Bitcoin SV chain client.
//!
//! BSV uses legacy P2PKH addresses (base58check, version byte 0x00 on mainnet)
//! and inherits the BIP143-variant SIGHASH_FORKID = 0x41 signing rules from
//! the BCH fork. There is no SegWit, no CashAddr, and no Taproot.
//!
//! ## Endpoints
//!
//! The canonical BSV indexer is WhatsOnChain. The endpoints vector is
//! expected to contain one or more base URLs rooted at `/v1/bsv/main`
//! (or `/v1/bsv/test` for testnet). Paths appended below:
//!
//! - `GET /address/{addr}/balance` → `{confirmed, unconfirmed}`
//! - `GET /address/{addr}/unspent`  → `[{tx_hash, tx_pos, value, height}]`
//! - `POST /tx/raw`                 → body `{"txhex": "..."}` returning a txid string
//!
//! Failures fall through to the next endpoint via `with_fallback`.

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::sign_bsv_tx;

// ----------------------------------------------------------------
// WhatsOnChain response types
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct WocBalance {
    #[serde(default)]
    pub(super) confirmed: i64,
    #[serde(default)]
    pub(super) unconfirmed: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct WocUtxo {
    pub(super) tx_hash: String,
    pub(super) tx_pos: u32,
    pub(super) value: u64,
    #[serde(default)]
    pub(super) height: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct WocHistoryItem {
    pub(super) tx_hash: String,
    #[serde(default)]
    pub(super) height: i64,
}

/// Full tx JSON returned by WoC `/tx/hash/{hash}`. Only the fields we
/// actually use are modeled — `#[serde(default)]` lets unknown/missing
/// fields fall through cleanly.
#[derive(Debug, Default, Deserialize)]
pub(super) struct WocTxDetail {
    #[serde(default)]
    pub(super) time: Option<u64>,
    #[serde(default)]
    pub(super) blocktime: Option<u64>,
    #[serde(default)]
    pub(super) blockheight: Option<i64>,
    #[serde(default)]
    pub(super) vin: Vec<WocTxVin>,
    #[serde(default)]
    pub(super) vout: Vec<WocTxVout>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct WocTxVin {
    #[serde(default)]
    pub(super) addr: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct WocTxVout {
    /// BSV amount as a float (WoC convention). Convert ×1e8 for sats.
    #[serde(default)]
    pub(super) value: f64,
    #[serde(default)]
    #[serde(rename = "scriptPubKey")]
    pub(super) script_pub_key: Option<WocTxVoutScriptPubKey>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct WocTxVoutScriptPubKey {
    #[serde(default)]
    pub(super) addresses: Option<Vec<String>>,
}

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BsvBalance {
    pub balance_sat: u64,
    pub balance_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BsvUtxo {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BsvSendResult {
    pub txid: String,
    #[serde(default)]
    pub raw_tx_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BsvHistoryEntry {
    pub txid: String,
    pub block_height: u64,
    pub timestamp: u64,
    /// Best-effort net value change for the queried address in sats.
    /// Positive = incoming (sum of vout values paid to this address).
    /// Negative = outgoing (vin addresses include this address).
    /// Zero = indeterminate (no direct match on either side).
    pub amount_sat: i64,
    pub is_incoming: bool,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct BitcoinSvClient {
    pub(super) endpoints: Vec<String>,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl BitcoinSvClient {
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
