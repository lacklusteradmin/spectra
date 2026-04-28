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

impl super::SignedSubmission for CardanoSendResult {
    fn submission_id(&self) -> &str { &self.txid }
    fn signed_payload(&self) -> &str { &self.cbor_hex }
    fn signed_payload_format(&self) -> super::SignedPayloadFormat { super::SignedPayloadFormat::Hex }
}

// ----------------------------------------------------------------
// Blockfrost response types (shared within the chain module)
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct BfAddress {
    pub(crate) amount: Vec<BfAmount>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BfAmount {
    pub(crate) unit: String,
    pub(crate) quantity: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BfUtxo {
    pub(crate) tx_hash: String,
    pub(crate) tx_index: u32,
    pub(crate) amount: Vec<BfAmount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct BfTx {
    pub(crate) hash: String,
    pub(crate) block: String,
    pub(crate) block_time: u64,
    #[serde(default)]
    pub(crate) output_amount: Vec<BfAmount>,
    pub(crate) fees: String,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct CardanoClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) api_key: String,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl CardanoClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>, api_key: String) -> Self {
        Self {
            endpoints,
            api_key,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
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
// Cardano fetch paths (Blockfrost REST): balance, UTXOs, history, latest slot.



impl CardanoClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<CardanoBalance, String> {
        let info: BfAddress = self.get(&format!("/addresses/{address}")).await?;
        let lovelace: u64 = info
            .amount
            .iter()
            .find(|a| a.unit == "lovelace")
            .and_then(|a| a.quantity.parse().ok())
            .unwrap_or(0);
        Ok(CardanoBalance {
            lovelace,
            ada_display: format_ada(lovelace),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<CardanoUtxo>, String> {
        let utxos: Vec<BfUtxo> = self.get(&format!("/addresses/{address}/utxos")).await?;
        Ok(utxos
            .into_iter()
            .map(|u| {
                let lovelace = u
                    .amount
                    .iter()
                    .find(|a| a.unit == "lovelace")
                    .and_then(|a| a.quantity.parse().ok())
                    .unwrap_or(0);
                CardanoUtxo {
                    tx_hash: u.tx_hash,
                    tx_index: u.tx_index,
                    lovelace,
                }
            })
            .collect())
    }

    pub async fn fetch_history(
        &self,
        address: &str,
    ) -> Result<Vec<CardanoHistoryEntry>, String> {
        #[derive(Deserialize)]
        struct BfTxRef {
            tx_hash: String,
        }
        let tx_refs: Vec<BfTxRef> = self
            .get(&format!("/addresses/{address}/transactions?count=50&order=desc"))
            .await?;

        let mut entries = Vec::new();
        for tx_ref in tx_refs {
            let tx: BfTx = match self.get(&format!("/txs/{}", tx_ref.tx_hash)).await {
                Ok(t) => t,
                Err(_) => continue,
            };
            let amount_lovelace: i64 = tx
                .output_amount
                .iter()
                .find(|a| a.unit == "lovelace")
                .and_then(|a| a.quantity.parse().ok())
                .unwrap_or(0i64);
            let fee_lovelace: u64 = tx.fees.parse().unwrap_or(0);
            entries.push(CardanoHistoryEntry {
                txid: tx.hash,
                block: tx.block,
                block_time: tx.block_time,
                is_incoming: amount_lovelace > 0,
                amount_lovelace,
                fee_lovelace,
            });
        }
        Ok(entries)
    }

    /// Fetch current slot from the latest block.
    pub async fn fetch_latest_slot(&self) -> Result<u64, String> {
        #[derive(Deserialize)]
        struct LatestBlock {
            slot: u64,
        }
        let block: LatestBlock = self.get("/blocks/latest").await?;
        Ok(block.slot)
    }
}

fn format_ada(lovelace: u64) -> String {
    let whole = lovelace / 1_000_000;
    let frac = lovelace % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}
