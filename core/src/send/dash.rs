//! Dash send: legacy V1 P2PKH transactions with standard SIGHASH_ALL
//! double-SHA-256 sighash. The wire format is identical to Bitcoin/Litecoin
//! legacy — Dash never adopted SegWit on mainnet.

use crate::send::error::SendError;

use super::bitcoin_wire::p2pkh_script;
use super::bitcoin_wire::{decode_txid_le, dsha256, varint};
use crate::derivation::utxo_address::parse_utxo_address;
use crate::registry::Chain;

const SIGHASH_ALL: u32 = 1;

#[allow(clippy::too_many_arguments)]
pub(crate) fn sign_dash_p2pkh(
    chain: Chain,
    utxos: &[(String, u32, u64, Vec<u8>)],
    to_address: &str,
    amount_sat: u64,
    fee_sat: u64,
    change_address: &str,
    private_key_bytes: &[u8],
    dust_threshold: Option<u64>,
) -> Result<Vec<u8>, SendError> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(private_key_bytes)
        .map_err(|e| SendError::Invalid(format!("dash invalid privkey: {e}").into()))?;
    let pubkey_bytes = secp256k1::PublicKey::from_secret_key(&secp, &secret_key).serialize();

    let change = super::accounting::checked_change(
        utxos.iter().map(|(_, _, v, _)| *v),
        amount_sat,
        fee_sat,
    )?;

    let to_script = parse_utxo_address(chain, to_address)?.script_pubkey();
    let change_hash = parse_utxo_address(chain, change_address)?.require_p2pkh()?;
    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(to_script, amount_sat)];
    if change > dust_threshold.unwrap_or(546) {
        outputs.push((p2pkh_script(&change_hash), change));
    }

    let mut signed_inputs: Vec<Vec<u8>> = Vec::new();
    for (txid, vout, _, _script_pubkey) in utxos {
        // Build SIGHASH_ALL preimage (legacy Bitcoin form).
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
            pre.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        }
        pre.extend_from_slice(&varint(outputs.len()));
        for (s, val) in &outputs {
            pre.extend_from_slice(&val.to_le_bytes());
            pre.extend_from_slice(&varint(s.len()));
            pre.extend_from_slice(s);
        }
        pre.extend_from_slice(&0u32.to_le_bytes()); // locktime
        pre.extend_from_slice(&SIGHASH_ALL.to_le_bytes());

        let hash = dsha256(&pre);
        let msg = Message::from_digest_slice(&hash).map_err(SendError::invalid)?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(SIGHASH_ALL as u8);

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
        inp.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
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
