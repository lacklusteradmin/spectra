//! Cardano chain client.
//!
//! Uses the Blockfrost REST API (api.blockfrost.io/v0) for balance,
//! history, protocol params, and transaction submission.
//! Cardano transactions are encoded in CBOR (cardano-multiplatform-lib
//! is too heavy; we use a minimal handwritten CBOR encoder for simple
//! ADA-only transfers).
//! Signing uses Ed25519 (ed25519-dalek).

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::build_signed_ada_tx;

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoBalance {
    /// Lovelace (1 ADA = 1_000_000 lovelace).
    pub lovelace: u64,
    pub ada_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoUtxo {
    pub tx_hash: String,
    pub tx_index: u32,
    pub lovelace: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoHistoryEntry {
    pub txid: String,
    pub block: String,
    pub block_time: u64,
    pub is_incoming: bool,
    pub amount_lovelace: i64,
    pub fee_lovelace: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoSendResult {
    pub txid: String,
    /// CBOR hex of the signed transaction — stored for rebroadcast.
    pub cbor_hex: String,
}

// ----------------------------------------------------------------
// Blockfrost response types (shared within the chain module)
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct BfAddress {
    pub(super) amount: Vec<BfAmount>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BfAmount {
    pub(super) unit: String,
    pub(super) quantity: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct BfUtxo {
    pub(super) tx_hash: String,
    pub(super) tx_index: u32,
    pub(super) amount: Vec<BfAmount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct BfTx {
    pub(super) hash: String,
    pub(super) block: String,
    pub(super) block_time: u64,
    #[serde(default)]
    pub(super) output_amount: Vec<BfAmount>,
    pub(super) fees: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct CardanoClient {
    pub(super) endpoints: Vec<String>,
    pub(super) api_key: String,
    pub(super) client: std::sync::Arc<HttpClient>,
}

impl CardanoClient {
    pub fn new(endpoints: Vec<String>, api_key: String) -> Self {
        Self {
            endpoints,
            api_key,
            client: HttpClient::shared(),
        }
    }

    pub(super) async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        let api_key = self.api_key.clone();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let api_key = api_key.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move {
                let mut headers = std::collections::HashMap::new();
                headers.insert("project_id", api_key.as_str());
                client
                    .get_json_with_headers(
                        &url,
                        &{
                            let mut h = std::collections::HashMap::new();
                            h.insert("project_id", api_key.as_str());
                            h
                        },
                        RetryProfile::ChainRead,
                    )
                    .await
            }
        })
        .await
    }
}
