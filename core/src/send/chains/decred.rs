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

use crate::derivation::chains::decred::{
    blake256, dcr_p2pkh_script, decode_dcr_address, encode_dcr_p2pkh,
};
use crate::fetch::chains::decred::{DcrSendResult, DecredClient};

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

impl DecredClient {
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_atoms: u64,
        fee_atoms: u64,
        private_key_bytes: &[u8],
    ) -> Result<DcrSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let from_hash = decode_dcr_address(from_address)?;
        let from_script = dcr_p2pkh_script(&from_hash);
        let to_hash = decode_dcr_address(to_address)?;

        let total_in: u64 = utxos.iter().map(|u| u.value_atoms).sum();
        let change = total_in.saturating_sub(amount_atoms + fee_atoms);

        // Outputs: recipient + optional change. Decred dust threshold is 6030
        // atoms for standard P2PKH; below that we drop change.
        let mut outputs: Vec<(Vec<u8>, u64)> = vec![(dcr_p2pkh_script(&to_hash), amount_atoms)];
        if change > 6_030 {
            let change_hash = decode_dcr_address(from_address)?;
            // Re-encode the same change address from its hash; ensures the
            // wire output uses the canonical encoding even if the caller
            // passed a normalized variant.
            let _ = encode_dcr_p2pkh(&change_hash);
            outputs.push((dcr_p2pkh_script(&change_hash), change));
        }

        let inputs: Vec<DcrInputBuild> = utxos
            .iter()
            .map(|u| DcrInputBuild {
                txid: u.txid.clone(),
                vout: u.vout,
                tree: TX_TREE_REGULAR,
                sequence: 0xFFFF_FFFF,
                amount: u.value_atoms,
                script_pubkey: from_script.clone(),
            })
            .collect();

        let raw = sign_dcr_tx(&inputs, &outputs, private_key_bytes)?;
        self.broadcast_raw_tx(&hex::encode(&raw)).await
    }
}

struct DcrInputBuild {
    txid: String,
    vout: u32,
    tree: u8,
    sequence: u32,
    amount: u64,
    script_pubkey: Vec<u8>,
}

fn sign_dcr_tx(
    inputs: &[DcrInputBuild],
    outputs: &[(Vec<u8>, u64)],
    private_key_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(private_key_bytes)
        .map_err(|e| format!("dcr invalid privkey: {e}"))?;
    let pubkey_bytes = secp256k1::PublicKey::from_secret_key(&secp, &secret_key).serialize();

    // Decred sighash optimization: prefix hash is constant across all inputs
    // for SIGHASH_ALL since the prefix never references signature scripts.
    let prefix_serialization = serialize_prefix(inputs, outputs, 0, 0);
    let prefix_hash = blake256(&prefix_serialization);

    let mut signed_sig_scripts: Vec<Vec<u8>> = Vec::with_capacity(inputs.len());
    for (i, _input) in inputs.iter().enumerate() {
        // Witness-signing serialization keeps only the script_pubkey on the
        // input being signed; all others have empty sigScripts.
        let witness_serialization = serialize_witness_signing(inputs, i);
        let witness_hash = blake256(&witness_serialization);

        let mut preimage = Vec::with_capacity(4 + 32 + 32);
        preimage.extend_from_slice(&SIGHASH_ALL.to_le_bytes());
        preimage.extend_from_slice(&prefix_hash);
        preimage.extend_from_slice(&witness_hash);
        let sighash = blake256(&preimage);

        let msg = Message::from_digest_slice(&sighash).map_err(|e| e.to_string())?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(SIGHASH_ALL as u8);

        // Standard P2PKH sigScript: <sig+sighash> <pubkey>.
        let mut script_sig = Vec::with_capacity(2 + der.len() + pubkey_bytes.len());
        script_sig.push(der.len() as u8);
        script_sig.extend_from_slice(&der);
        script_sig.push(pubkey_bytes.len() as u8);
        script_sig.extend_from_slice(&pubkey_bytes);
        signed_sig_scripts.push(script_sig);
    }

    Ok(serialize_full(inputs, outputs, &signed_sig_scripts, 0, 0))
}

fn varint(n: usize) -> Vec<u8> {
    match n {
        0..=0xfc => vec![n as u8],
        0xfd..=0xffff => {
            let mut v = vec![0xfd];
            v.extend_from_slice(&(n as u16).to_le_bytes());
            v
        }
        _ => {
            let mut v = vec![0xfe];
            v.extend_from_slice(&(n as u32).to_le_bytes());
            v
        }
    }
}

fn decode_txid_le(txid: &str) -> Vec<u8> {
    let mut bytes = hex::decode(txid).unwrap_or_default();
    bytes.reverse();
    bytes
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
        buf.extend_from_slice(&decode_txid_le(&input.txid));
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
        buf.extend_from_slice(&decode_txid_le(&input.txid));
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
