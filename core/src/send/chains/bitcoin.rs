//! Bitcoin send path: P2WPKH / P2PKH / P2TR signers, coin selection, fee
//! calculation, and Esplora broadcast.

use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash as _;
use bitcoin::key::{TapTweak, TweakedKeypair};
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::sighash::{EcdsaSighashType, SighashCache, TapSighashType};
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
};
use zeroize::Zeroize;

use crate::http::{with_fallback, RetryProfile};

use crate::fetch::chains::bitcoin::{bitcoin_network_for_mode, BitcoinClient, BitcoinSendResult, EsploraUtxo, FeeRate};

impl BitcoinClient {
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

// ── Professional coin-selection and output customization ──────────────────

/// UTXO coin-selection strategy.
#[derive(Debug, Clone, Default)]
pub enum CoinSelectionStrategy {
    /// Accumulate the largest UTXOs first. Minimizes input count; may leave
    /// dust change when total overshoots the target significantly.
    #[default]
    LargestFirst,
    /// Accumulate the smallest UTXOs first. Consolidates dust coins at the
    /// cost of more inputs and higher fees. Useful for UTXO set hygiene.
    SmallestFirst,
}

/// An additional output appended to the transaction after the primary
/// recipient and change outputs. Enables batch sends and on-chain memos.
#[derive(Debug, Clone)]
pub enum BitcoinExtraOutput {
    /// Secondary payment to another address in the same transaction.
    Address { address: String, amount_sats: u64 },
    /// OP_RETURN metadata payload (max 80 bytes, zero value). The `data_hex`
    /// field is hex-encoded; no `0x` prefix required.
    OpReturn { data_hex: String },
}

/// Parameters for building a Bitcoin transaction.
#[derive(Debug)]
pub struct BitcoinSendParams {
    /// From address (used to find the script type).
    pub from_address: String,
    /// WIF or hex-encoded 32-byte private key.
    pub private_key_hex: String,
    /// Primary recipient address.
    pub to_address: String,
    /// Primary send amount in satoshis.
    pub amount_sats: u64,
    /// Fee rate (sats per virtual byte).
    pub fee_rate: FeeRate,
    /// UTXOs available for automatic coin selection. Ignored when
    /// `pinned_utxos` is set. Fetched from Esplora if empty.
    pub available_utxos: Vec<EsploraUtxo>,
    /// Which network.
    pub network_mode: String,
    /// Whether to signal RBF (replace-by-fee) on inputs.
    pub enable_rbf: bool,
    /// Minimum change output in satoshis; dust below this is absorbed into fee.
    /// `None` uses the standard 546-sat P2PKH dust threshold.
    pub dust_threshold: Option<u64>,
    // ── Professional controls ────────────────────────────────────────────────
    /// Manually selected UTXOs to spend. When `Some`, bypasses automatic coin
    /// selection entirely — the caller is responsible for ensuring the total
    /// input value covers `amount_sats` plus fee.
    pub pinned_utxos: Option<Vec<EsploraUtxo>>,
    /// Additional outputs appended after the primary recipient and change.
    /// Enables batch sends (multiple recipients in one tx) and OP_RETURN memos.
    pub extra_outputs: Vec<BitcoinExtraOutput>,
    /// Coin selection algorithm used when `pinned_utxos` is `None`.
    pub coin_selection: CoinSelectionStrategy,
    /// Sign the transaction but skip the Esplora broadcast. The signed raw
    /// transaction hex is returned in `BitcoinSendResult.raw_tx_hex` and
    /// `txid` is left empty. Useful for offline signing, PSBT handoffs, or
    /// pre-flight fee inspection.
    pub sign_only: bool,
}

/// Coin selection: accumulate UTXOs until we cover `target + fee`.
/// Sorting order is determined by `strategy`.
fn select_coins<'a>(
    utxos: &'a [EsploraUtxo],
    target_sats: u64,
    fee_rate: FeeRate,
    input_bytes: usize,
    output_count: usize,
    overhead_bytes: usize,
    strategy: &CoinSelectionStrategy,
) -> Result<(Vec<&'a EsploraUtxo>, u64), String> {
    let mut sorted: Vec<&EsploraUtxo> = utxos.iter().collect();
    match strategy {
        CoinSelectionStrategy::LargestFirst => sorted.sort_by(|a, b| b.value.cmp(&a.value)),
        CoinSelectionStrategy::SmallestFirst => sorted.sort_by(|a, b| a.value.cmp(&b.value)),
    }

    let mut selected: Vec<&EsploraUtxo> = Vec::new();
    let mut total: u64 = 0;

    for utxo in sorted {
        selected.push(utxo);
        total += utxo.value;

        let n_inputs = selected.len();
        let tx_bytes = overhead_bytes + n_inputs * input_bytes + output_count * 31;
        let fee = (tx_bytes as f64 * fee_rate.sats_per_vbyte).ceil() as u64;

        if total >= target_sats.saturating_add(fee) {
            return Ok((selected, fee));
        }
    }

    Err("utxo.insufficientFunds".to_string())
}

/// Build `TxOut` items for `extra_outputs`. OP_RETURN outputs are zero-value.
fn build_extra_outputs(
    extra: &[BitcoinExtraOutput],
    network: bitcoin::Network,
) -> Result<Vec<TxOut>, String> {
    let mut out = Vec::new();
    for item in extra {
        match item {
            BitcoinExtraOutput::Address { address, amount_sats } => {
                let addr = Address::from_str(address)
                    .map_err(|e| format!("extra output bad address: {e}"))?
                    .require_network(network)
                    .map_err(|e| format!("extra output wrong network: {e}"))?;
                out.push(TxOut {
                    value: Amount::from_sat(*amount_sats),
                    script_pubkey: addr.script_pubkey(),
                });
            }
            BitcoinExtraOutput::OpReturn { data_hex } => {
                let data = hex::decode(data_hex.trim_start_matches("0x"))
                    .map_err(|e| format!("OP_RETURN bad hex: {e}"))?;
                if data.len() > 80 {
                    return Err(format!(
                        "OP_RETURN data too large: {} bytes (max 80)",
                        data.len()
                    ));
                }
                let push_bytes = bitcoin::script::PushBytesBuf::try_from(data)
                    .map_err(|_| "OP_RETURN data exceeds PushBytes limit".to_string())?;
                out.push(TxOut {
                    value: Amount::ZERO,
                    script_pubkey: ScriptBuf::new_op_return(push_bytes.as_push_bytes()),
                });
            }
        }
    }
    Ok(out)
}

/// Build, sign, and serialize a P2WPKH transaction.
pub fn sign_p2wpkh(params: &mut BitcoinSendParams) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let network = bitcoin_network_for_mode(&params.network_mode);

    // Parse private key.
    let mut key_bytes =
        hex::decode(&params.private_key_hex).map_err(|e| format!("bad private key hex: {e}"))?;
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
    let extra_output_count = params.extra_outputs.len();
    let (selected_owned, fee) = if let Some(ref pinned) = params.pinned_utxos {
        let total_in: u64 = pinned.iter().map(|u| u.value).sum();
        let n = pinned.len();
        let output_count = 2 + extra_output_count;
        let tx_bytes = 10 + n * 68 + output_count * 31;
        let fee = (tx_bytes as f64 * params.fee_rate.sats_per_vbyte).ceil() as u64;
        let dummy: Vec<&EsploraUtxo> = pinned.iter().collect();
        if total_in < params.amount_sats.saturating_add(fee) {
            return Err("utxo.insufficientFunds".to_string());
        }
        (dummy, fee)
    } else {
        select_coins(
            &params.available_utxos,
            params.amount_sats,
            params.fee_rate,
            68,
            2 + extra_output_count,
            10,
            &params.coin_selection,
        )?
    };

    let utxos_to_use: Vec<&EsploraUtxo> = if params.pinned_utxos.is_some() {
        params.pinned_utxos.as_ref().unwrap().iter().collect()
    } else {
        selected_owned
    };

    let total_in: u64 = utxos_to_use.iter().map(|u| u.value).sum();
    let change_sats = total_in.saturating_sub(params.amount_sats).saturating_sub(fee);
    let use_change = change_sats > params.dust_threshold.unwrap_or(546);

    // Build inputs (unsigned).
    let sequence = if params.enable_rbf {
        Sequence::ENABLE_RBF_NO_LOCKTIME
    } else {
        Sequence::MAX
    };

    let inputs: Vec<TxIn> = utxos_to_use
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
    outputs.extend(build_extra_outputs(&params.extra_outputs, network)?);

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
    for (i, utxo) in utxos_to_use.iter().enumerate() {
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
pub fn sign_p2pkh(params: &mut BitcoinSendParams) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let network = bitcoin_network_for_mode(&params.network_mode);

    let mut key_bytes =
        hex::decode(&params.private_key_hex).map_err(|e| format!("bad private key hex: {e}"))?;
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
    let extra_output_count = params.extra_outputs.len();
    let (selected_owned, fee) = if let Some(ref pinned) = params.pinned_utxos {
        let total_in: u64 = pinned.iter().map(|u| u.value).sum();
        let n = pinned.len();
        let tx_bytes = 10 + n * 148 + (2 + extra_output_count) * 34;
        let fee = (tx_bytes as f64 * params.fee_rate.sats_per_vbyte).ceil() as u64;
        let dummy: Vec<&EsploraUtxo> = pinned.iter().collect();
        if total_in < params.amount_sats.saturating_add(fee) {
            return Err("utxo.insufficientFunds".to_string());
        }
        (dummy, fee)
    } else {
        select_coins(
            &params.available_utxos,
            params.amount_sats,
            params.fee_rate,
            148,
            2 + extra_output_count,
            10,
            &params.coin_selection,
        )?
    };

    let utxos_to_use: Vec<&EsploraUtxo> = if params.pinned_utxos.is_some() {
        params.pinned_utxos.as_ref().unwrap().iter().collect()
    } else {
        selected_owned
    };

    let total_in: u64 = utxos_to_use.iter().map(|u| u.value).sum();
    let change_sats = total_in.saturating_sub(params.amount_sats).saturating_sub(fee);
    let use_change = change_sats > params.dust_threshold.unwrap_or(546);

    let sequence = if params.enable_rbf {
        Sequence::ENABLE_RBF_NO_LOCKTIME
    } else {
        Sequence::MAX
    };

    let inputs: Vec<TxIn> = utxos_to_use
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
    outputs.extend(build_extra_outputs(&params.extra_outputs, network)?);

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

    for i in 0..utxos_to_use.len() {
        let from_spk = from_addr.script_pubkey();
        let sighash = sighash_cache
            .legacy_signature_hash(i, &from_spk, EcdsaSighashType::All as u32)
            .map_err(|e| format!("sighash: {e}"))?;

        let msg = Message::from_digest(sighash.to_raw_hash().to_byte_array());
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut sig_der = sig.serialize_der().to_vec();
        sig_der.push(EcdsaSighashType::All as u8);
        signatures.push((sig_der, pk_bytes_p2pkh.to_vec()));
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
pub fn sign_p2tr(params: &mut BitcoinSendParams) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let network = bitcoin_network_for_mode(&params.network_mode);

    let mut key_bytes =
        hex::decode(&params.private_key_hex).map_err(|e| format!("bad private key hex: {e}"))?;
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
    let extra_output_count = params.extra_outputs.len();
    let (selected_owned, fee) = if let Some(ref pinned) = params.pinned_utxos {
        let total_in: u64 = pinned.iter().map(|u| u.value).sum();
        let n = pinned.len();
        let tx_bytes = 10 + n * 58 + (2 + extra_output_count) * 31;
        let fee = (tx_bytes as f64 * params.fee_rate.sats_per_vbyte).ceil() as u64;
        let dummy: Vec<&EsploraUtxo> = pinned.iter().collect();
        if total_in < params.amount_sats.saturating_add(fee) {
            return Err("utxo.insufficientFunds".to_string());
        }
        (dummy, fee)
    } else {
        select_coins(
            &params.available_utxos,
            params.amount_sats,
            params.fee_rate,
            58,
            2 + extra_output_count,
            10,
            &params.coin_selection,
        )?
    };

    let utxos_to_use: Vec<&EsploraUtxo> = if params.pinned_utxos.is_some() {
        params.pinned_utxos.as_ref().unwrap().iter().collect()
    } else {
        selected_owned
    };

    let total_in: u64 = utxos_to_use.iter().map(|u| u.value).sum();
    let change_sats = total_in.saturating_sub(params.amount_sats).saturating_sub(fee);
    let use_change = change_sats > params.dust_threshold.unwrap_or(546);

    let sequence = if params.enable_rbf {
        Sequence::ENABLE_RBF_NO_LOCKTIME
    } else {
        Sequence::MAX
    };

    let inputs: Vec<TxIn> = utxos_to_use
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
    outputs.extend(build_extra_outputs(&params.extra_outputs, network)?);

    let prevouts: Vec<TxOut> = utxos_to_use
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

    for i in 0..utxos_to_use.len() {
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
/// and broadcast (unless `params.sign_only` is set).
pub async fn sign_and_broadcast(
    client: &BitcoinClient,
    mut params: BitcoinSendParams,
) -> Result<BitcoinSendResult, String> {
    // Fetch UTXOs when auto-selection is active and none were pre-supplied.
    if params.pinned_utxos.is_none() && params.available_utxos.is_empty() {
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

    if params.sign_only {
        return Ok(BitcoinSendResult {
            txid: String::new(),
            raw_tx_hex: raw_hex,
        });
    }

    let txid = client.broadcast_raw_tx(&raw_hex).await?;

    Ok(BitcoinSendResult {
        txid,
        raw_tx_hex: raw_hex,
    })
}
