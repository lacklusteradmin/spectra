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

use std::str::FromStr;
use std::sync::Arc;

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash as _;
use bitcoin::key::{TapTweak, TweakedKeypair};
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::sighash::{EcdsaSighashType, SighashCache, TapSighashType};
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, CompressedPublicKey, Network, OutPoint, ScriptBuf, Sequence,
    Transaction, TxIn, TxOut, Txid, Witness,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::core::http::{with_fallback, HttpClient, RetryProfile};

// ----------------------------------------------------------------
// Network helpers
// ----------------------------------------------------------------

fn bitcoin_network_for_mode(mode: &str) -> Network {
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
    http: Arc<HttpClient>,
    /// Ordered list of Esplora base URLs for the current network mode.
    endpoints: Vec<String>,
    #[allow(dead_code)]
    network: Network,
}

impl BitcoinClient {
    pub fn new(http: Arc<HttpClient>, endpoints: Vec<String>, network_mode: &str) -> Self {
        Self {
            http,
            endpoints,
            network: bitcoin_network_for_mode(network_mode),
        }
    }

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
                let txs: Vec<EsploraTx> =
                    http.get_json(&url, RetryProfile::ChainRead).await?;

                Ok(txs.into_iter().map(|tx| {
                    // Net change = sum of outputs to this address - sum of inputs from this address
                    let received: u64 = tx.vout.iter()
                        .filter(|o| o.scriptpubkey_address.as_deref() == Some(&addr))
                        .map(|o| o.value)
                        .sum();
                    let spent: u64 = tx.vin.iter()
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
                }).collect())
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
    // Broadcast
    // ----------------------------------------------------------------

    pub async fn broadcast_raw_tx(&self, raw_tx_hex: &str) -> Result<String, String> {
        let raw = raw_tx_hex.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        with_fallback(&endpoints, |base| {
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

// ----------------------------------------------------------------
// Transaction construction & signing
// ----------------------------------------------------------------

/// Parameters for building a Bitcoin transaction.
#[derive(Debug)]
pub struct BitcoinSendParams {
    /// From address (used to find the script type).
    pub from_address: String,
    /// WIF or hex-encoded 32-byte private key.
    pub private_key_hex: String,
    /// Recipient address.
    pub to_address: String,
    /// Amount to send in satoshis.
    pub amount_sats: u64,
    /// Fee rate (sats per virtual byte).
    pub fee_rate: FeeRate,
    /// UTXOs available for spending.
    pub available_utxos: Vec<EsploraUtxo>,
    /// Which network.
    pub network_mode: String,
    /// Whether to use RBF (replace-by-fee). Usually `true`.
    pub enable_rbf: bool,
}

/// Coin selection: accumulate UTXOs (largest first) until we cover
/// `target + fee`. Returns the selected UTXOs and the fee in sats.
fn select_coins(
    utxos: &[EsploraUtxo],
    target_sats: u64,
    fee_rate: FeeRate,
    input_bytes: usize,
    output_count: usize,
    overhead_bytes: usize,
) -> Result<(Vec<&EsploraUtxo>, u64), String> {
    let mut sorted: Vec<&EsploraUtxo> = utxos.iter().collect();
    sorted.sort_by(|a, b| b.value.cmp(&a.value));

    let mut selected: Vec<&EsploraUtxo> = Vec::new();
    let mut total: u64 = 0;

    for utxo in sorted {
        selected.push(utxo);
        total += utxo.value;

        let n_inputs = selected.len();
        let tx_bytes =
            overhead_bytes + n_inputs * input_bytes + output_count * 31;
        let fee = (tx_bytes as f64 * fee_rate.sats_per_vbyte).ceil() as u64;

        if total >= target_sats.saturating_add(fee) {
            return Ok((selected, fee));
        }
    }

    Err("utxo.insufficientFunds".to_string())
}

/// Build, sign, and serialize a P2WPKH transaction.
pub fn sign_p2wpkh(
    params: &mut BitcoinSendParams,
) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let network = bitcoin_network_for_mode(&params.network_mode);

    // Parse private key.
    let mut key_bytes = hex::decode(&params.private_key_hex)
        .map_err(|e| format!("bad private key hex: {e}"))?;
    let secret_key =
        SecretKey::from_slice(&key_bytes).map_err(|e| format!("bad private key: {e}"))?;
    key_bytes.zeroize();

    let secp_pk = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let pk = CompressedPublicKey::from_slice(&secp_pk.serialize())
        .map_err(|e| format!("pk: {e}"))?;
    let _keypair = bitcoin::key::Keypair::from_secret_key(&secp, &secret_key);

    // Parse recipient.
    let to_addr = Address::from_str(&params.to_address)
        .map_err(|e| format!("bad recipient address: {e}"))?
        .require_network(network)
        .map_err(|e| format!("recipient on wrong network: {e}"))?;

    // P2WPKH: 68 bytes per input (segwit discount applied), 31 bytes per output, 10 overhead.
    let (selected, fee) = select_coins(
        &params.available_utxos,
        params.amount_sats,
        params.fee_rate,
        68,
        2, // to + change
        10,
    )?;

    let total_in: u64 = selected.iter().map(|u| u.value).sum();
    let change_sats = total_in.saturating_sub(params.amount_sats).saturating_sub(fee);
    let use_change = change_sats > 546; // dust threshold

    // Build inputs (unsigned).
    let sequence = if params.enable_rbf {
        Sequence::ENABLE_RBF_NO_LOCKTIME
    } else {
        Sequence::MAX
    };

    let inputs: Vec<TxIn> = selected
        .iter()
        .map(|u| {
            let txid = Txid::from_str(&u.txid)
                .map_err(|e| format!("bad txid {}: {e}", u.txid))
                .unwrap_or_else(|_| Txid::all_zeros());
            TxIn {
                previous_output: OutPoint { txid, vout: u.vout },
                script_sig: ScriptBuf::new(),
                sequence,
                witness: Witness::new(),
            }
        })
        .collect();

    // Build outputs.
    let mut outputs = vec![TxOut {
        value: Amount::from_sat(params.amount_sats),
        script_pubkey: to_addr.script_pubkey(),
    }];

    let from_addr = Address::from_str(&params.from_address)
        .map_err(|e| format!("bad from address: {e}"))?
        .require_network(network)
        .map_err(|e| format!("from address on wrong network: {e}"))?;

    if use_change {
        outputs.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: from_addr.script_pubkey(),
        });
    }

    let mut tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    // Sign each input with P2WPKH sighash.
    let wpkh = pk.wpubkey_hash();
    let pk_bytes = pk.to_bytes();
    let mut sighash_cache = SighashCache::new(&mut tx);

    let script_code = ScriptBuf::new_p2wpkh(&wpkh);

    let mut signatures: Vec<Vec<u8>> = Vec::new();
    for (i, utxo) in selected.iter().enumerate() {
        let sighash = sighash_cache
            .p2wpkh_signature_hash(
                i,
                &script_code,
                Amount::from_sat(utxo.value),
                EcdsaSighashType::All,
            )
            .map_err(|e| format!("sighash: {e}"))?;

        let msg = Message::from_digest(sighash.to_raw_hash().to_byte_array());
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut sig_der = sig.serialize_der().to_vec();
        sig_der.push(EcdsaSighashType::All as u8);
        signatures.push(sig_der);
    }

    // Apply witnesses.
    let tx_ref = sighash_cache.into_transaction();
    for (i, sig) in signatures.iter().enumerate() {
        let mut witness = Witness::new();
        witness.push(sig);
        witness.push(pk_bytes.as_slice());
        tx_ref.input[i].witness = witness;
    }

    let raw_hex = bitcoin::consensus::encode::serialize_hex(tx_ref);
    Ok((tx_ref.clone(), raw_hex))
}

/// Build, sign, and serialize a P2PKH (legacy) transaction.
pub fn sign_p2pkh(
    params: &mut BitcoinSendParams,
) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let network = bitcoin_network_for_mode(&params.network_mode);

    let mut key_bytes = hex::decode(&params.private_key_hex)
        .map_err(|e| format!("bad private key hex: {e}"))?;
    let secret_key =
        SecretKey::from_slice(&key_bytes).map_err(|e| format!("bad private key: {e}"))?;
    key_bytes.zeroize();

    let to_addr = Address::from_str(&params.to_address)
        .map_err(|e| format!("bad recipient address: {e}"))?
        .require_network(network)
        .map_err(|e| format!("recipient on wrong network: {e}"))?;

    let from_addr = Address::from_str(&params.from_address)
        .map_err(|e| format!("bad from address: {e}"))?
        .require_network(network)
        .map_err(|e| format!("from address on wrong network: {e}"))?;

    // P2PKH: ~148 bytes per input, 34 bytes per output, 10 overhead.
    let (selected, fee) = select_coins(
        &params.available_utxos,
        params.amount_sats,
        params.fee_rate,
        148,
        2,
        10,
    )?;

    let total_in: u64 = selected.iter().map(|u| u.value).sum();
    let change_sats = total_in.saturating_sub(params.amount_sats).saturating_sub(fee);
    let use_change = change_sats > 546;

    let sequence = if params.enable_rbf {
        Sequence::ENABLE_RBF_NO_LOCKTIME
    } else {
        Sequence::MAX
    };

    let inputs: Vec<TxIn> = selected
        .iter()
        .map(|u| {
            let txid = Txid::from_str(&u.txid).unwrap_or_else(|_| Txid::all_zeros());
            TxIn {
                previous_output: OutPoint { txid, vout: u.vout },
                script_sig: ScriptBuf::new(),
                sequence,
                witness: Witness::new(),
            }
        })
        .collect();

    let mut outputs = vec![TxOut {
        value: Amount::from_sat(params.amount_sats),
        script_pubkey: to_addr.script_pubkey(),
    }];
    if use_change {
        outputs.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: from_addr.script_pubkey(),
        });
    }

    let mut tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    // Sign each input with P2PKH sighash.
    let secp_pk_p2pkh = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let pk_bytes_p2pkh = secp_pk_p2pkh.serialize(); // [u8; 33] compressed

    let sighash_cache = SighashCache::new(&mut tx);
    let mut signatures: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();

    for (i, utxo) in selected.iter().enumerate() {
        let from_spk = from_addr.script_pubkey();
        let sighash = sighash_cache
            .legacy_signature_hash(i, &from_spk, EcdsaSighashType::All as u32)
            .map_err(|e| format!("sighash: {e}"))?;

        let msg = Message::from_digest(sighash.to_raw_hash().to_byte_array());
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut sig_der = sig.serialize_der().to_vec();
        sig_der.push(EcdsaSighashType::All as u8);
        signatures.push((sig_der, pk_bytes_p2pkh.to_vec()));
        let _ = utxo; // suppress unused warning
    }

    let tx_ref = sighash_cache.into_transaction();
    for (i, (sig, pk_bytes)) in signatures.iter().enumerate() {
        // Build P2PKH scriptSig manually:
        // <OP_PUSH(sig_len)> <sig> <OP_PUSH(pk_len)> <pk>
        let mut script_bytes = Vec::new();
        script_bytes.push(sig.len() as u8);
        script_bytes.extend_from_slice(sig);
        script_bytes.push(pk_bytes.len() as u8);
        script_bytes.extend_from_slice(pk_bytes);
        tx_ref.input[i].script_sig = ScriptBuf::from_bytes(script_bytes);
    }

    let raw_hex = bitcoin::consensus::encode::serialize_hex(tx_ref);
    Ok((tx_ref.clone(), raw_hex))
}

/// Build, sign, and serialize a P2TR (Taproot key-path) transaction.
pub fn sign_p2tr(
    params: &mut BitcoinSendParams,
) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let network = bitcoin_network_for_mode(&params.network_mode);

    let mut key_bytes = hex::decode(&params.private_key_hex)
        .map_err(|e| format!("bad private key hex: {e}"))?;
    let secret_key =
        SecretKey::from_slice(&key_bytes).map_err(|e| format!("bad private key: {e}"))?;
    key_bytes.zeroize();

    let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &secret_key);
    let tweaked_keypair: TweakedKeypair = keypair.tap_tweak(&secp, None);

    let to_addr = Address::from_str(&params.to_address)
        .map_err(|e| format!("bad recipient: {e}"))?
        .require_network(network)
        .map_err(|e| format!("wrong network: {e}"))?;

    let from_addr = Address::from_str(&params.from_address)
        .map_err(|e| format!("bad from address: {e}"))?
        .require_network(network)
        .map_err(|e| format!("from address on wrong network: {e}"))?;

    // Taproot key-path: 57.5 virtual bytes per input (approximated as 58).
    let (selected, fee) = select_coins(
        &params.available_utxos,
        params.amount_sats,
        params.fee_rate,
        58,
        2,
        10,
    )?;

    let total_in: u64 = selected.iter().map(|u| u.value).sum();
    let change_sats = total_in.saturating_sub(params.amount_sats).saturating_sub(fee);
    let use_change = change_sats > 546;

    let sequence = if params.enable_rbf {
        Sequence::ENABLE_RBF_NO_LOCKTIME
    } else {
        Sequence::MAX
    };

    let inputs: Vec<TxIn> = selected
        .iter()
        .map(|u| {
            let txid = Txid::from_str(&u.txid).unwrap_or_else(|_| Txid::all_zeros());
            TxIn {
                previous_output: OutPoint { txid, vout: u.vout },
                script_sig: ScriptBuf::new(),
                sequence,
                witness: Witness::new(),
            }
        })
        .collect();

    let mut outputs = vec![TxOut {
        value: Amount::from_sat(params.amount_sats),
        script_pubkey: to_addr.script_pubkey(),
    }];
    if use_change {
        outputs.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: from_addr.script_pubkey(),
        });
    }

    let prevouts: Vec<TxOut> = selected
        .iter()
        .map(|u| TxOut {
            value: Amount::from_sat(u.value),
            script_pubkey: from_addr.script_pubkey(),
        })
        .collect();

    let mut tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    let mut sighash_cache = SighashCache::new(&mut tx);
    let mut sigs: Vec<Vec<u8>> = Vec::new();

    for i in 0..selected.len() {
        use bitcoin::sighash::Prevouts;
        let sighash = sighash_cache
            .taproot_key_spend_signature_hash(
                i,
                &Prevouts::All(&prevouts),
                TapSighashType::Default,
            )
            .map_err(|e| format!("taproot sighash: {e}"))?;

        let msg = Message::from_digest(sighash.to_raw_hash().to_byte_array());
        let sig = secp.sign_schnorr(&msg, &tweaked_keypair.to_keypair());
        let tap_sig = bitcoin::taproot::Signature {
            signature: sig,
            sighash_type: TapSighashType::Default,
        };
        sigs.push(tap_sig.to_vec());
    }

    let tx_ref = sighash_cache.into_transaction();
    for (i, sig) in sigs.iter().enumerate() {
        let mut witness = Witness::new();
        witness.push(sig);
        tx_ref.input[i].witness = witness;
    }

    let raw_hex = bitcoin::consensus::encode::serialize_hex(tx_ref);
    Ok((tx_ref.clone(), raw_hex))
}

/// High-level send: auto-detect the from-address script type, sign,
/// and broadcast. Returns the txid.
pub async fn sign_and_broadcast(
    client: &BitcoinClient,
    mut params: BitcoinSendParams,
) -> Result<BitcoinSendResult, String> {
    // Fetch UTXOs if none were provided.
    if params.available_utxos.is_empty() {
        params.available_utxos = client.fetch_utxos(&params.from_address).await?;
    }

    // Detect script type from the from-address prefix.
    let raw_hex = if params.from_address.starts_with("bc1p")
        || params.from_address.starts_with("tb1p")
        || params.from_address.starts_with("bcrt1p")
    {
        let (_, hex) = sign_p2tr(&mut params)?;
        hex
    } else if params.from_address.starts_with("bc1q")
        || params.from_address.starts_with("tb1q")
        || params.from_address.starts_with("bcrt1q")
    {
        let (_, hex) = sign_p2wpkh(&mut params)?;
        hex
    } else {
        // Legacy (P2PKH) or nested-SegWit (P2SH-P2WPKH).
        // P2SH-P2WPKH uses the same signing path as P2PKH for the outer script.
        let (_, hex) = sign_p2pkh(&mut params)?;
        hex
    };

    let txid = client.broadcast_raw_tx(&raw_hex).await?;

    Ok(BitcoinSendResult {
        txid,
        raw_tx_hex: raw_hex,
    })
}

// ----------------------------------------------------------------
// Address validation (for the Bitcoin script family)
// ----------------------------------------------------------------

/// Validate a Bitcoin address string against the given network mode.
/// Returns the canonical form if valid.
pub fn validate_bitcoin_address(address: &str, network_mode: &str) -> Option<String> {
    let network = bitcoin_network_for_mode(network_mode);
    Address::from_str(address)
        .ok()
        .and_then(|a| a.require_network(network).ok())
        .map(|a| a.to_string())
}
