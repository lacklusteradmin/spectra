//! The Esplora REST adapter. `api::utxo` decides which adapter serves a
//! request.

use std::sync::Arc;

use serde::Deserialize;

use crate::api::http::HttpClient;
use crate::api::utxo::{FeeRate, Utxo, UtxoBalance, UtxoHistoryEntry, UtxoTxStatus};

// ── Esplora API types

#[derive(Debug, Deserialize)]
pub struct EsploraAddressStats {
    pub address: String,
    pub chain_stats: EsploraChainStats,
    pub mempool_stats: EsploraChainStats,
}

#[derive(Debug, Deserialize)]
pub struct EsploraChainStats {
    pub funded_txo_sum: u64,
    pub spent_txo_sum: u64,
    pub tx_count: u64,
}

#[derive(Debug, Deserialize)]
pub struct EsploraTx {
    pub txid: String,
    pub status: EsploraTxStatus,
    pub vout: Vec<EsploraTxVout>,
    pub vin: Vec<EsploraTxVin>,
    pub fee: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct EsploraTxStatus {
    pub confirmed: bool,
    pub block_height: Option<u64>,
    pub block_time: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct EsploraTxVout {
    pub scriptpubkey_address: Option<String>,
    pub value: u64,
}

#[derive(Debug, Deserialize)]
pub struct EsploraTxVin {
    pub prevout: Option<EsploraTxVout>,
}

#[derive(Debug, Deserialize)]
pub struct EsploraFeeEstimates {
    // Keys are confirmation-target strings ("1", "6", "144", etc.)
    #[serde(flatten)]
    pub targets: std::collections::HashMap<String, f64>,
}

/// One or more Esplora base URLs.
pub struct EsploraClient {
    pub(crate) http: Arc<HttpClient>,
    pub(crate) endpoints: Arc<Vec<String>>,
}

impl EsploraClient {
    pub fn new(http: Arc<HttpClient>, endpoints: Arc<Vec<String>>) -> Self {
        Self { http, endpoints }
    }
}

use crate::api::http::{RetryProfile, race};

impl EsploraClient {
    /// Address counters include spent history and pending transactions, without tx bodies.
    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, String> {
        race(&self.endpoints, |base| {
            let url = format!("{base}/address/{address}");
            async move {
                let stats: EsploraAddressStats =
                    self.http.get_json(&url, RetryProfile::ChainRead).await?;
                Ok(stats.chain_stats.tx_count > 0 || stats.mempool_stats.tx_count > 0)
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<UtxoBalance, String> {
        let addr = address.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        race(&endpoints, |base| {
            let addr = addr.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/address/{addr}");
                let stats: EsploraAddressStats =
                    http.get_json(&url, RetryProfile::ChainRead).await?;
                let confirmed_sats = stats
                    .chain_stats
                    .funded_txo_sum
                    .saturating_sub(stats.chain_stats.spent_txo_sum);
                let unconfirmed_sats = stats.mempool_stats.funded_txo_sum as i64
                    - stats.mempool_stats.spent_txo_sum as i64;
                Ok(UtxoBalance {
                    confirmed_sats,
                    unconfirmed_sats,
                })
            }
        })
        .await
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<Utxo>, String> {
        let addr = address.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        race(&endpoints, |base| {
            let addr = addr.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/address/{addr}/utxo");
                http.get_json(&url, RetryProfile::ChainRead).await
            }
        })
        .await
    }

    pub async fn fetch_history(
        &self,
        address: &str,
        after_txid: Option<&str>,
    ) -> Result<Vec<UtxoHistoryEntry>, String> {
        let addr = address.to_string();
        let cursor = after_txid.map(str::to_string);
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        race(&endpoints, |base| {
            let addr = addr.clone();
            let cursor = cursor.clone();
            let http = http.clone();
            async move {
                let url = match &cursor {
                    Some(txid) => format!("{base}/address/{addr}/txs/chain/{txid}"),
                    None => format!("{base}/address/{addr}/txs"),
                };
                let txs: Vec<EsploraTx> = http.get_json(&url, RetryProfile::ChainRead).await?;

                Ok(txs
                    .into_iter()
                    .map(|tx| {
                        // Net change = sum of outputs to this address - sum of inputs from this address
                        let received: u64 = tx
                            .vout
                            .iter()
                            .filter(|o| o.scriptpubkey_address.as_deref() == Some(&addr))
                            .map(|o| o.value)
                            .sum();
                        let spent: u64 = tx
                            .vin
                            .iter()
                            .filter_map(|i| i.prevout.as_ref())
                            .filter(|o| o.scriptpubkey_address.as_deref() == Some(&addr))
                            .map(|o| o.value)
                            .sum();
                        UtxoHistoryEntry {
                            txid: tx.txid,
                            confirmed: tx.status.confirmed,
                            block_height: tx.status.block_height,
                            block_time: tx.status.block_time,
                            net_sats: received as i64 - spent as i64,
                            fee_sats: tx.fee,
                        }
                    })
                    .collect())
            }
        })
        .await
    }

    /// Returns the fee rate for `confirmation_target` blocks (typically
    /// 1, 6, or 144). Falls back to a conservative 10 sat/vB if the
    /// estimate is unavailable.
    pub async fn fetch_fee_rate(&self, confirmation_target: u32) -> Result<FeeRate, String> {
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        let estimates: EsploraFeeEstimates = race(&endpoints, |base| {
            let http = http.clone();
            async move {
                let url = format!("{base}/fee-estimates");
                http.get_json(&url, RetryProfile::ChainRead).await
            }
        })
        .await?;

        let key = confirmation_target.to_string();
        let sats_per_vbyte = estimates
            .targets
            .get(&key)
            // Fallback: take the next available target above.
            .or_else(|| {
                estimates
                    .targets
                    .iter()
                    .filter(|(k, _)| k.parse::<u32>().unwrap_or(u32::MAX) >= confirmation_target)
                    .min_by_key(|(k, _)| k.parse::<u32>().unwrap_or(u32::MAX))
                    .map(|(_, v)| v)
            })
            .copied()
            .unwrap_or(10.0);

        Ok(FeeRate { sats_per_vbyte })
    }

    /// Fetch the confirmation status for a single txid.
    /// Esplora `GET /tx/{txid}/status` returns `EsploraTxStatus` directly.
    pub async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, String> {
        let txid = txid.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();
        race(&endpoints, |base| {
            let txid = txid.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/tx/{txid}/status");
                let s: EsploraTxStatus = http.get_json(&url, RetryProfile::ChainRead).await?;
                Ok(UtxoTxStatus {
                    txid: txid.clone(),
                    confirmed: s.confirmed,
                    block_height: s.block_height,
                    block_time: s.block_time,
                    confirmations: None,
                })
            }
        })
        .await
    }
}

impl EsploraClient {
    pub async fn broadcast_raw_tx(&self, raw_tx_hex: &str) -> Result<String, String> {
        let raw = raw_tx_hex.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        race(&endpoints, |base| {
            let raw = raw.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/tx");
                // Esplora broadcast: POST hex-encoded tx as plain text, returns txid.
                http.post_text(&url, raw, RetryProfile::ChainWrite).await
            }
        })
        .await
        .map(|s| s.trim().to_string())
    }
}
