//! Decred send pipeline.
//!
//! Decred transactions diverge from Bitcoin in two important ways:
//!   1. Inputs and witnesses are split. The "prefix" (inputs without
//!      sigScripts + outputs + locktime + expiry) is serialized separately
//!      from the "witness" (per-input amount/blockHeight/blockIndex/sigScript).
//!   2. Sighash is computed via BLAKE-256, not double-SHA-256, and uses a
//!      precomputed prefix-hash + per-input witness-hash combination so it
//!      doesn't redo the prefix work for every input.
//!
//! Spectra ships SIGHASH_ALL only — the dominant case for normal transfers.
//! Tree-stake (PoS) inputs and split-tx flows are out of scope.

use crate::send::error::SendError;

use super::bitcoin_wire::{decode_txid_le, varint};
use crate::derivation::decred::{blake256, parse_decred_address};
use crate::derivation::utxo_address::ParsedUtxoAddress;
use crate::registry::Chain;

/// Decred wire `version | serType` 32-bit header, encoded little-endian. The
/// low 16 bits hold the tx version (1 for standard transfers); the high 16
/// bits hold the serialization type used for the message.
const VERSION_FULL: u32 = 1; // serType = 0 (full) << 16 | version 1
const VERSION_NO_WITNESS: u32 = (1u32 << 16) | 1; // serType = 1 (no witness) << 16 | version 1
const VERSION_ONLY_WITNESS: u32 = (2u32 << 16) | 1; // serType = 2 (only witness) << 16 | version 1

const SIGHASH_ALL: u32 = 1;

/// Block-index sentinel used in unsigned witnesses to indicate a UTXO whose
/// confirmation height is not being committed to.
const TX_TREE_REGULAR: u8 = 0;

/// The script a wallet's own address pays: a secp256k1 key's P2PKH, the
/// only kind a Decred wallet signs for.
pub(crate) fn sender_script(chain: Chain, address: &str) -> Result<Vec<u8>, SendError> {
    Ok(
        ParsedUtxoAddress::P2pkh(parse_decred_address(chain, address)?.require_p2pkh()?)
            .script_pubkey(),
    )
}

/// The script `to_address` names on `chain`'s network: a pubkey hash or a
/// script hash. Any other form, and an address of the other network, is
/// refused before an input is read.
pub(crate) fn recipient_script(chain: Chain, to_address: &str) -> Result<Vec<u8>, SendError> {
    Ok(parse_decred_address(chain, to_address)?.script_pubkey())
}

/// One regular-tree input and the key of the address it pays.
pub(crate) struct DecredSigningInput<'a> {
    pub utxo: &'a (String, u32, u64, Vec<u8>),
    pub private_key: &'a [u8],
}

/// Sign `inputs`, each with its own key, into a transaction paying
/// `outputs` in order. Every input must pay its key's P2PKH script.
pub(crate) fn sign(
    inputs: &[DecredSigningInput<'_>],
    outputs: &[(Vec<u8>, u64)],
) -> Result<Vec<u8>, SendError> {
    let builds = inputs
        .iter()
        .map(|input| {
            Ok(DcrInputBuild {
                outpoint_txid: decode_txid_le(&input.utxo.0)?,
                vout: input.utxo.1,
                tree: TX_TREE_REGULAR,
                sequence: 0xFFFF_FFFF,
                amount: input.utxo.2,
                script_pubkey: input.utxo.3.clone(),
            })
        })
        .collect::<Result<Vec<_>, SendError>>()?;
    let keys: Vec<&[u8]> = inputs.iter().map(|input| input.private_key).collect();
    sign_dcr_tx(&builds, outputs, &keys)
}

#[derive(Debug, Clone)]
struct DcrInputBuild {
    /// The outpoint txid in wire (little-endian) order.
    outpoint_txid: Vec<u8>,
    vout: u32,
    tree: u8,
    sequence: u32,
    amount: u64,
    script_pubkey: Vec<u8>,
}

fn sign_dcr_tx(
    inputs: &[DcrInputBuild],
    outputs: &[(Vec<u8>, u64)],
    private_keys: &[&[u8]],
) -> Result<Vec<u8>, SendError> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    if inputs.is_empty() || outputs.is_empty() || inputs.len() != private_keys.len() {
        return Err(SendError::Invalid(
            "dcr transaction must have inputs, outputs and one key per input".into(),
        ));
    }
    let mut keys = Vec::with_capacity(inputs.len());
    let mut outpoints = std::collections::HashSet::new();
    for (input, private_key) in inputs.iter().zip(private_keys) {
        let secret_key = SecretKey::from_slice(private_key)
            .map_err(|e| SendError::Invalid(format!("dcr invalid privkey: {e}").into()))?;
        let pubkey_bytes = secp256k1::PublicKey::from_secret_key(&secp, &secret_key).serialize();
        if input.script_pubkey
            != ParsedUtxoAddress::P2pkh(crate::derivation::decred::dcr_hash160(&pubkey_bytes))
                .script_pubkey()
        {
            return Err(SendError::Invalid(
                "dcr input does not belong to its signing key".into(),
            ));
        }
        if !outpoints.insert((input.outpoint_txid.clone(), input.vout)) {
            return Err(SendError::Invalid("dcr: duplicate input".into()));
        }
        keys.push((secret_key, pubkey_bytes));
    }

    // Decred sighash optimization: prefix hash is constant across all inputs
    // for SIGHASH_ALL since the prefix never references signature scripts.
    let prefix_serialization = serialize_prefix(inputs, outputs, 0, 0);
    let prefix_hash = blake256(&prefix_serialization);

    let mut signed_sig_scripts: Vec<Vec<u8>> = Vec::with_capacity(inputs.len());
    for (i, (secret_key, pubkey_bytes)) in keys.iter().enumerate() {
        let sighash = signature_hash(inputs, &prefix_hash, i);
        let msg = Message::from_digest_slice(&sighash).map_err(SendError::invalid)?;
        let sig = secp.sign_ecdsa(&msg, secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(SIGHASH_ALL as u8);

        // Standard P2PKH sigScript: <sig+sighash> <pubkey>.
        let mut script_sig = Vec::with_capacity(2 + der.len() + pubkey_bytes.len());
        script_sig.push(der.len() as u8);
        script_sig.extend_from_slice(&der);
        script_sig.push(pubkey_bytes.len() as u8);
        script_sig.extend_from_slice(pubkey_bytes);
        signed_sig_scripts.push(script_sig);
    }

    Ok(serialize_full(inputs, outputs, &signed_sig_scripts, 0, 0))
}

/// dcrd's `CalcSignatureHash` for `SIGHASH_ALL`: the hash type, the prefix
/// hash and the hash of the witness-signing serialization.
fn signature_hash(inputs: &[DcrInputBuild], prefix_hash: &[u8; 32], index: usize) -> [u8; 32] {
    // Witness-signing serialization keeps only the script_pubkey on the
    // input being signed; all others have empty sigScripts.
    let witness_hash = blake256(&serialize_witness_signing(inputs, index));
    let mut preimage = Vec::with_capacity(4 + 32 + 32);
    preimage.extend_from_slice(&SIGHASH_ALL.to_le_bytes());
    preimage.extend_from_slice(prefix_hash);
    preimage.extend_from_slice(&witness_hash);
    blake256(&preimage)
}

fn serialize_outputs(buf: &mut Vec<u8>, outputs: &[(Vec<u8>, u64)]) {
    buf.extend_from_slice(&varint(outputs.len()));
    for (script, value) in outputs {
        buf.extend_from_slice(&value.to_le_bytes());
        // script_version: 2 bytes, 0 = standard
        buf.extend_from_slice(&[0u8, 0u8]);
        buf.extend_from_slice(&varint(script.len()));
        buf.extend_from_slice(script);
    }
}

/// Decred prefix-only serialization (serType = 1). Inputs have no sigScripts.
fn serialize_prefix(
    inputs: &[DcrInputBuild],
    outputs: &[(Vec<u8>, u64)],
    locktime: u32,
    expiry: u32,
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&VERSION_NO_WITNESS.to_le_bytes());
    buf.extend_from_slice(&varint(inputs.len()));
    for input in inputs {
        buf.extend_from_slice(&input.outpoint_txid);
        buf.extend_from_slice(&input.vout.to_le_bytes());
        buf.push(input.tree);
        buf.extend_from_slice(&input.sequence.to_le_bytes());
    }
    serialize_outputs(&mut buf, outputs);
    buf.extend_from_slice(&locktime.to_le_bytes());
    buf.extend_from_slice(&expiry.to_le_bytes());
    buf
}

/// Decred witness-only serialization for the sighash digest (serType = 3).
/// Per dcrd's `CalcSignatureHash`, when computing the witness hash for input
/// `signing_index`, that input gets the previous-output script as its sigScript
/// and all other inputs get an empty sigScript. The amount/blockHeight/
/// blockIndex fields are NOT included in this signing serialization.
fn serialize_witness_signing(inputs: &[DcrInputBuild], signing_index: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    // Decred's witness-signing serialization type is 3.
    let header = (3u32 << 16) | 1u32;
    buf.extend_from_slice(&header.to_le_bytes());
    buf.extend_from_slice(&varint(inputs.len()));
    for (i, input) in inputs.iter().enumerate() {
        let script: &[u8] = if i == signing_index {
            &input.script_pubkey
        } else {
            &[]
        };
        buf.extend_from_slice(&varint(script.len()));
        buf.extend_from_slice(script);
    }
    buf
}

/// Full Decred V1 transaction serialization (serType = 0): prefix followed
/// by the witness section (one entry per input with `value_in`, `block_height`,
/// `block_index`, and `signature_script`).
fn serialize_full(
    inputs: &[DcrInputBuild],
    outputs: &[(Vec<u8>, u64)],
    signed_sig_scripts: &[Vec<u8>],
    locktime: u32,
    expiry: u32,
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&VERSION_FULL.to_le_bytes());
    buf.extend_from_slice(&varint(inputs.len()));
    for input in inputs {
        buf.extend_from_slice(&input.outpoint_txid);
        buf.extend_from_slice(&input.vout.to_le_bytes());
        buf.push(input.tree);
        buf.extend_from_slice(&input.sequence.to_le_bytes());
    }
    serialize_outputs(&mut buf, outputs);
    buf.extend_from_slice(&locktime.to_le_bytes());
    buf.extend_from_slice(&expiry.to_le_bytes());

    // Witness section: one entry per input.
    buf.extend_from_slice(&varint(inputs.len()));
    for (input, script) in inputs.iter().zip(signed_sig_scripts) {
        buf.extend_from_slice(&input.amount.to_le_bytes()); // value_in
        // block_height: signing wallet doesn't know the UTXO's confirmation
        // height; setting 0xFFFFFFFF (the "unknown" sentinel) is accepted by
        // mempool when paired with block_index = 0xFFFFFFFF.
        buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // block_index
        buf.extend_from_slice(&varint(script.len()));
        buf.extend_from_slice(script);
    }
    let _ = VERSION_ONLY_WITNESS;
    buf
}

#[cfg(test)]
#[path = "tests/decred.rs"]
mod tests;
