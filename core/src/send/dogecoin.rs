//! Dogecoin send: the P2PKH signer. Broadcast goes through `api::utxo`.

use crate::send::error::SendError;

use super::bitcoin_wire::p2pkh_script;

use super::bitcoin_wire::{
    build_input, build_tx, decode_txid_le, dsha256, p2pkh_script_sig, varint,
};
use crate::derivation::utxo_address::parse_utxo_address;
use crate::registry::Chain;

// ── Dogecoin P2PKH signing

/// Sign and serialize a Dogecoin P2PKH transaction.
///
/// `utxos` — selected UTXOs with their redeeming scripts (the previous P2PKH
/// scriptPubKey for each input).
/// Returns raw transaction bytes ready for broadcast.
#[allow(clippy::too_many_arguments)]
pub fn sign_doge_p2pkh(
    chain: Chain,
    utxos: &[(String, u32, u64, Vec<u8>)], // (txid, vout, value_koin, script_pubkey)
    to_address: &str,
    amount_koin: u64,
    fee_koin: u64,
    change_address: &str,
    private_key_bytes: &[u8],
    dust_threshold: Option<u64>,
) -> Result<Vec<u8>, SendError> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(private_key_bytes)
        .map_err(|e| SendError::Invalid(format!("invalid key: {e}").into()))?;
    let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let pubkey_bytes = pubkey.serialize(); // compressed

    let change = super::accounting::checked_change(
        utxos.iter().map(|(_, _, v, _)| *v),
        amount_koin,
        fee_koin,
    )?;

    let to_script = parse_utxo_address(chain, to_address)?.script_pubkey();
    let change_hash = parse_utxo_address(chain, change_address)?.require_p2pkh()?;
    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(to_script, amount_koin)];
    if change > dust_threshold.unwrap_or(546) {
        outputs.push((p2pkh_script(&change_hash), change));
    }

    // Sign each input.
    let mut signed_inputs: Vec<Vec<u8>> = Vec::new();
    for (txid, vout, _, script_pubkey) in utxos {
        let preimage = build_sighash_preimage(utxos, *vout, txid, script_pubkey, &outputs, 1)?;
        let hash = dsha256(&preimage);
        let msg = Message::from_digest_slice(&hash).map_err(SendError::invalid)?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(0x01); // SIGHASH_ALL

        let script_sig = p2pkh_script_sig(&der, &pubkey_bytes);
        signed_inputs.push(build_input(txid, *vout, &script_sig, 0xffff_ffff)?);
    }

    Ok(build_tx(&signed_inputs, &outputs))
}

fn build_sighash_preimage(
    utxos: &[(String, u32, u64, Vec<u8>)],
    signing_vout: u32,
    signing_txid: &str,
    _script_pubkey: &[u8],
    outputs: &[(Vec<u8>, u64)],
    sighash_type: u32,
) -> Result<Vec<u8>, SendError> {
    let mut raw = Vec::new();
    // version
    raw.extend_from_slice(&1u32.to_le_bytes());
    // inputs
    raw.extend_from_slice(&varint(utxos.len()));
    for (txid, vout, _, spk) in utxos {
        let txid_bytes = decode_txid_le(txid)?;
        raw.extend_from_slice(&txid_bytes);
        raw.extend_from_slice(&vout.to_le_bytes());
        if vout == &signing_vout && txid == signing_txid {
            raw.extend_from_slice(&varint(spk.len()));
            raw.extend_from_slice(spk);
        } else {
            raw.push(0x00); // empty script for other inputs
        }
        raw.extend_from_slice(&0xffffffffu32.to_le_bytes());
    }
    raw.extend_from_slice(&varint(outputs.len()));
    for (script, value) in outputs {
        raw.extend_from_slice(&value.to_le_bytes());
        raw.extend_from_slice(&varint(script.len()));
        raw.extend_from_slice(script);
    }
    // locktime
    raw.extend_from_slice(&0u32.to_le_bytes());
    raw.extend_from_slice(&sighash_type.to_le_bytes());
    Ok(raw)
}
