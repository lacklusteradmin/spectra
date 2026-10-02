//! Litecoin send: P2PKH transactions and MWEB peg-in transactions.

use crate::send::error::SendError;

use super::bitcoin_wire::p2pkh_script;
use super::bitcoin_wire::{decode_txid_le, dsha256, varint};
use crate::derivation::utxo_address::parse_utxo_address;
use crate::registry::Chain;

// ── Litecoin transaction signing

/// Sign a Litecoin transaction spending P2PKH inputs to an arbitrary output script.
///
/// `to_script` is the full scriptPubKey for the primary output (recipient).
/// For ordinary sends it is a P2PKH script; for MWEB peg-ins it is the HogEx
/// witness-v8 script produced by `build_peg_in_extension`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sign_ltc_with_output_script(
    chain: Chain,
    utxos: &[(String, u32, u64, Vec<u8>)],
    to_script: &[u8],
    amount_sat: u64,
    fee_sat: u64,
    change_address: &str,
    private_key_bytes: &[u8],
    dust_threshold: Option<u64>,
) -> Result<Vec<u8>, SendError> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(private_key_bytes)
        .map_err(|e| SendError::Invalid(format!("invalid key: {e}").into()))?;
    let pubkey_bytes = secp256k1::PublicKey::from_secret_key(&secp, &secret_key).serialize();

    let change = super::accounting::checked_change(
        utxos.iter().map(|(_, _, v, _)| *v),
        amount_sat,
        fee_sat,
    )?;

    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(to_script.to_vec(), amount_sat)];
    if change > dust_threshold.unwrap_or(546) {
        outputs.push((
            p2pkh_script(&parse_utxo_address(chain, change_address)?.require_p2pkh()?),
            change,
        ));
    }

    let mut signed_inputs: Vec<Vec<u8>> = Vec::new();
    for (txid, vout, _, _script_pubkey) in utxos {
        // Build SIGHASH_ALL preimage.
        let mut pre = Vec::new();
        pre.extend_from_slice(&1u32.to_le_bytes()); // version
        pre.extend_from_slice(&varint(utxos.len()));
        for (t, v, _, spk) in utxos {
            pre.extend_from_slice(&decode_txid_le(t)?);
            pre.extend_from_slice(&v.to_le_bytes());
            if v == vout && t == txid {
                pre.extend_from_slice(&varint(spk.len()));
                pre.extend_from_slice(spk);
            } else {
                pre.push(0x00);
            }
            pre.extend_from_slice(&0xffffffffu32.to_le_bytes());
        }
        pre.extend_from_slice(&varint(outputs.len()));
        for (s, val) in &outputs {
            pre.extend_from_slice(&val.to_le_bytes());
            pre.extend_from_slice(&varint(s.len()));
            pre.extend_from_slice(s);
        }
        pre.extend_from_slice(&0u32.to_le_bytes()); // locktime
        pre.extend_from_slice(&1u32.to_le_bytes()); // SIGHASH_ALL

        let hash = dsha256(&pre);
        let msg = Message::from_digest_slice(&hash).map_err(SendError::invalid)?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(0x01); // SIGHASH_ALL

        let mut script_sig = Vec::new();
        script_sig.push(der.len() as u8);
        script_sig.extend_from_slice(&der);
        script_sig.push(pubkey_bytes.len() as u8);
        script_sig.extend_from_slice(&pubkey_bytes);

        let mut inp = Vec::new();
        inp.extend_from_slice(&decode_txid_le(txid)?);
        inp.extend_from_slice(&vout.to_le_bytes());
        inp.extend_from_slice(&varint(script_sig.len()));
        inp.extend_from_slice(&script_sig);
        inp.extend_from_slice(&0xffffffffu32.to_le_bytes());
        signed_inputs.push(inp);
    }

    let mut raw = Vec::new();
    raw.extend_from_slice(&1u32.to_le_bytes()); // version
    raw.extend_from_slice(&varint(signed_inputs.len()));
    for inp in &signed_inputs {
        raw.extend_from_slice(inp);
    }
    raw.extend_from_slice(&varint(outputs.len()));
    for (s, val) in &outputs {
        raw.extend_from_slice(&val.to_le_bytes());
        raw.extend_from_slice(&varint(s.len()));
        raw.extend_from_slice(s);
    }
    raw.extend_from_slice(&0u32.to_le_bytes()); // locktime
    Ok(raw)
}
