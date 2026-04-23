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

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;
pub use send::{
    sign_and_broadcast, sign_p2pkh, sign_p2tr, sign_p2wpkh, BitcoinSendParams,
};

// ----------------------------------------------------------------
// Network helpers
// ----------------------------------------------------------------

pub(super) fn bitcoin_network_for_mode(mode: &str) -> Network {
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
    pub(super) http: Arc<HttpClient>,
    /// Ordered list of Esplora base URLs for the current network mode.
    pub(super) endpoints: Vec<String>,
    #[allow(dead_code)]
    pub(super) network: Network,
}

impl BitcoinClient {
    pub fn new(http: Arc<HttpClient>, endpoints: Vec<String>, network_mode: &str) -> Self {
        Self {
            http,
            endpoints,
            network: bitcoin_network_for_mode(network_mode),
        }
    }
}
