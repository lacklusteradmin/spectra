//! Version-1 P2PKH transactions of the networks that kept Bitcoin's legacy
//! wire format, spending a wallet account's inputs, each with its own key.
//!
//! Dogecoin and Dash sign the legacy SIGHASH_ALL hash. Bitcoin Cash, Bitcoin
//! SV and Bitcoin Gold sign BIP143's hash with SIGHASH_FORKID: the preimage's
//! 32-bit hash type carries the network's fork id above the flag
//! (`fork_id << 8 | 0x41`), and the signature ends in `0x41` alone. Bitcoin
//! Gold's fork id is 79; the other two keep 0.

use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};

use super::bitcoin_wire::{
    build_input, build_tx, decode_txid_le, dsha256, p2pkh_script, p2pkh_script_sig, varint,
};
use crate::send::error::SendError;

const SEQUENCE: u32 = 0xffff_ffff;
const SIGHASH_ALL: u8 = 0x01;
const SIGHASH_ALL_FORKID: u8 = 0x41;

/// One input and the key of the address it pays.
pub(crate) struct LegacyInput<'a> {
    pub utxo: &'a (String, u32, u64, Vec<u8>),
    pub private_key: &'a [u8],
}

/// Sign `inputs` into a version-1 transaction paying `outputs` in order.
/// `fork_id` is the network's SIGHASH_FORKID id, or `None` for the legacy
/// hash. Every input must pay its key's P2PKH script.
pub(crate) fn sign(
    inputs: &[LegacyInput<'_>],
    outputs: &[(Vec<u8>, u64)],
    fork_id: Option<u32>,
) -> Result<Vec<u8>, SendError> {
    if inputs.is_empty() || outputs.is_empty() {
        return Err(SendError::Invalid(
            "transaction must have inputs and outputs".into(),
        ));
    }
    let secp = Secp256k1::new();
    let mut keys = Vec::with_capacity(inputs.len());
    let mut outpoints = std::collections::HashSet::new();
    for input in inputs {
        let secret = SecretKey::from_slice(input.private_key).map_err(SendError::invalid)?;
        let public = PublicKey::from_secret_key(&secp, &secret).serialize();
        if input.utxo.3 != p2pkh_script(&crate::derivation::bitcoin::hash160(&public)) {
            return Err(SendError::Invalid(
                "Input script does not match the supplied private key".into(),
            ));
        }
        if !outpoints.insert((decode_txid_le(&input.utxo.0)?, input.utxo.1)) {
            return Err(SendError::Invalid("duplicate input".into()));
        }
        keys.push((secret, public));
    }
    let serialized_outputs = {
        let mut bytes = Vec::new();
        for (script, value) in outputs {
            bytes.extend_from_slice(&value.to_le_bytes());
            bytes.extend_from_slice(&varint(script.len()));
            bytes.extend_from_slice(script);
        }
        bytes
    };
    let forkid_hashes = fork_id
        .map(|_| -> Result<_, SendError> {
            let mut prevouts = Vec::new();
            let mut sequences = Vec::new();
            for input in inputs {
                prevouts.extend_from_slice(&decode_txid_le(&input.utxo.0)?);
                prevouts.extend_from_slice(&input.utxo.1.to_le_bytes());
                sequences.extend_from_slice(&SEQUENCE.to_le_bytes());
            }
            Ok((
                dsha256(&prevouts),
                dsha256(&sequences),
                dsha256(&serialized_outputs),
            ))
        })
        .transpose()?;

    let mut signed = Vec::with_capacity(inputs.len());
    for (index, (input, (secret, public))) in inputs.iter().zip(&keys).enumerate() {
        let (txid, vout, value, script) = input.utxo;
        let (digest, hash_type) = match (fork_id, &forkid_hashes) {
            (Some(fork_id), Some((prevouts, sequences, hashed_outputs))) => {
                let mut preimage = Vec::new();
                preimage.extend_from_slice(&1u32.to_le_bytes());
                preimage.extend_from_slice(prevouts);
                preimage.extend_from_slice(sequences);
                preimage.extend_from_slice(&decode_txid_le(txid)?);
                preimage.extend_from_slice(&vout.to_le_bytes());
                preimage.extend_from_slice(&varint(script.len()));
                preimage.extend_from_slice(script);
                preimage.extend_from_slice(&value.to_le_bytes());
                preimage.extend_from_slice(&SEQUENCE.to_le_bytes());
                preimage.extend_from_slice(hashed_outputs);
                preimage.extend_from_slice(&0u32.to_le_bytes());
                let hash_type = fork_id
                    .checked_shl(8)
                    .filter(|shifted| shifted >> 8 == fork_id)
                    .ok_or_else(|| SendError::Invalid("fork id out of range".into()))?
                    | u32::from(SIGHASH_ALL_FORKID);
                preimage.extend_from_slice(&hash_type.to_le_bytes());
                (dsha256(&preimage), SIGHASH_ALL_FORKID)
            }
            _ => {
                let mut preimage = Vec::new();
                preimage.extend_from_slice(&1u32.to_le_bytes());
                preimage.extend_from_slice(&varint(inputs.len()));
                for (other, candidate) in inputs.iter().enumerate() {
                    preimage.extend_from_slice(&decode_txid_le(&candidate.utxo.0)?);
                    preimage.extend_from_slice(&candidate.utxo.1.to_le_bytes());
                    if other == index {
                        preimage.extend_from_slice(&varint(script.len()));
                        preimage.extend_from_slice(script);
                    } else {
                        preimage.push(0x00);
                    }
                    preimage.extend_from_slice(&SEQUENCE.to_le_bytes());
                }
                preimage.extend_from_slice(&varint(outputs.len()));
                preimage.extend_from_slice(&serialized_outputs);
                preimage.extend_from_slice(&0u32.to_le_bytes());
                preimage.extend_from_slice(&u32::from(SIGHASH_ALL).to_le_bytes());
                (dsha256(&preimage), SIGHASH_ALL)
            }
        };
        let mut signature = secp
            .sign_ecdsa(&Message::from_digest(digest), secret)
            .serialize_der()
            .to_vec();
        signature.push(hash_type);
        signed.push(build_input(
            txid,
            *vout,
            &p2pkh_script_sig(&signature, public),
            SEQUENCE,
        )?);
    }
    Ok(build_tx(&signed, outputs))
}

#[cfg(test)]
#[path = "tests/legacy_p2pkh.rs"]
mod tests;
