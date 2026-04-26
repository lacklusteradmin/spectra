//! Bitcoin chain: balance/UTXO/history fetch via Esplora, and
//! P2WPKH/P2PKH/P2SH-P2WPKH/P2TR transaction construction, signing,
//! and broadcast.
//!
//! ## Network providers (Esplora endpoints from AppEndpointDirectory.json)
//!
//! Mainnet:  blockstream.info/api, mempool.space/api, emzy.de/...
//! Testnet:  mempool.space/testnet/api, blockstream.info/testnet/api
//! Testnet4: mempool.space/testnet4/api
//! Signet:   mempool.space/signet/api
//!
//! All calls use `with_fallback` so that if the primary endpoint is
//! unreachable the next one is tried automatically.
//!
//! ## Transaction signing
//!
//! Private keys are derived in `main.rs` (the derivation runtime) and
//! passed into `sign_and_broadcast` as a hex-encoded 32-byte scalar.
//! We use the `bitcoin` 0.32 crate for transaction and address types,
//! and `secp256k1` 0.29 for signing. Keys are zeroized after use.

use std::sync::Arc;

use bitcoin::Network;
use serde::{Deserialize, Serialize};

use crate::http::HttpClient;

// ----------------------------------------------------------------
// Network helpers
// ----------------------------------------------------------------

pub(crate) fn bitcoin_network_for_mode(mode: &str) -> Network {
    match mode {
        "testnet" => Network::Testnet,
        "testnet4" => Network::Testnet4,
        "signet" => Network::Signet,
        _ => Network::Bitcoin, // mainnet
    }
}

// ----------------------------------------------------------------
// Esplora API types
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EsploraUtxo {
    pub txid: String,
    pub vout: u32,
    pub status: EsploraUtxoStatus,
    pub value: u64,
}

#[derive(Debug, Deserialize)]
pub struct EsploraUtxoStatus {
    pub confirmed: bool,
    pub block_height: Option<u64>,
}

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

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

/// Unified tx confirmation status returned by all UTXO chains.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct UtxoTxStatus {
    pub txid: String,
    pub confirmed: bool,
    pub block_height: Option<u64>,
    pub block_time: Option<u64>,
    /// Number of confirmations (populated by Blockbook-backed chains; None for Esplora/WoC).
    pub confirmations: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitcoinBalance {
    /// Confirmed balance in satoshis.
    pub confirmed_sats: u64,
    /// Unconfirmed balance delta (can be negative).
    pub unconfirmed_sats: i64,
    /// Total UTXOs.
    pub utxo_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitcoinHistoryEntry {
    pub txid: String,
    pub confirmed: bool,
    pub block_height: Option<u64>,
    pub block_time: Option<u64>,
    /// Net satoshi change for the watched address (positive = received,
    /// negative = sent).
    pub net_sats: i64,
    pub fee_sats: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BitcoinSendResult {
    pub txid: String,
    pub raw_tx_hex: String,
}

// ----------------------------------------------------------------
// Fee rate
// ----------------------------------------------------------------

/// Satoshis per virtual byte, as returned by `GET /fee-estimates`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FeeRate {
    /// Satoshis per virtual byte.
    pub sats_per_vbyte: f64,
}

impl FeeRate {
    pub fn sats_per_kwu(self) -> u64 {
        (self.sats_per_vbyte * 250.0) as u64
    }
}

// ----------------------------------------------------------------
// BitcoinClient
// ----------------------------------------------------------------

/// Stateless client for all Bitcoin Esplora interactions.
pub struct BitcoinClient {
    pub(crate) http: Arc<HttpClient>,
    /// Ordered list of Esplora base URLs for the current network mode.
    pub(crate) endpoints: Arc<Vec<String>>,
}

impl BitcoinClient {
    pub fn new(http: Arc<HttpClient>, endpoints: Arc<Vec<String>>) -> Self {
        Self { http, endpoints }
    }
}
// Bitcoin fetch paths (Esplora REST): balance, UTXOs, history, fee estimates,
// and tx status.

use crate::http::{with_fallback, RetryProfile};


impl BitcoinClient {
    // ----------------------------------------------------------------
    // Fetch: balance
    // ----------------------------------------------------------------

    pub async fn fetch_balance(&self, address: &str) -> Result<BitcoinBalance, String> {
        let addr = address.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        with_fallback(&endpoints, |base| {
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
                Ok(BitcoinBalance {
                    confirmed_sats,
                    unconfirmed_sats,
                    utxo_count: stats.chain_stats.tx_count as usize,
                })
            }
        })
        .await
    }

    // ----------------------------------------------------------------
    // Fetch: UTXOs
    // ----------------------------------------------------------------

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<EsploraUtxo>, String> {
        let addr = address.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        with_fallback(&endpoints, |base| {
            let addr = addr.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/address/{addr}/utxo");
                http.get_json(&url, RetryProfile::ChainRead).await
            }
        })
        .await
    }

    // ----------------------------------------------------------------
    // Fetch: transaction history
    // ----------------------------------------------------------------

    pub async fn fetch_history(
        &self,
        address: &str,
        after_txid: Option<&str>,
    ) -> Result<Vec<BitcoinHistoryEntry>, String> {
        let addr = address.to_string();
        let cursor = after_txid.map(str::to_string);
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        with_fallback(&endpoints, |base| {
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
                        BitcoinHistoryEntry {
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

    // ----------------------------------------------------------------
    // Fetch: fee estimates
    // ----------------------------------------------------------------

    /// Returns the fee rate for `confirmation_target` blocks (typically
    /// 1, 6, or 144). Falls back to a conservative 10 sat/vB if the
    /// estimate is unavailable.
    pub async fn fetch_fee_rate(&self, confirmation_target: u32) -> Result<FeeRate, String> {
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        let estimates: EsploraFeeEstimates = with_fallback(&endpoints, |base| {
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

    // ----------------------------------------------------------------
    // Fetch: tx status (confirmation lookup)
    // ----------------------------------------------------------------

    /// Fetch the confirmation status for a single txid.
    /// Esplora `GET /tx/{txid}/status` returns `EsploraTxStatus` directly.
    pub async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, String> {
        let txid = txid.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();
        with_fallback(&endpoints, |base| {
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
