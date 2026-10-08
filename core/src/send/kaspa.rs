//! Kaspa send pipeline.
//!
//! Kaspa uses keyed BLAKE2b-256 (key = `"TransactionSigningHash"`) for the
//! sighash and Schnorr-secp256k1 (BIP-340 style) signatures. The wire body
//! posted to `api.kaspa.org/transactions` is JSON, not a raw hex blob.
//!
//! Sighash preimage (SigHashAll) over the standard "TxHashes" subset:
//!   ```text
//!   version (u16 LE) ||
//!   prev_outputs_hash (32) ||
//!   sequences_hash (32) ||
//!   sig_op_counts_hash (32) ||
//!   <input being signed: prevout (txid+index) || script_pubkey_version (u16 LE)
//!     || script_pubkey_len (u64 LE) || script_pubkey || amount (u64 LE)
//!     || sequence (u64 LE) || sig_op_count (u8)> ||
//!   outputs_hash (32) ||
//!   lock_time (u64 LE) ||
//!   subnetwork_id (20) ||
//!   gas (u64 LE) ||
//!   payload_hash (32) ||
//!   sighash_type (u8)
//!   ```
//! Each *_hash is a BLAKE2b-256 of the corresponding section serialized in
//! the canonical Kaspa form, also keyed with `"TransactionSigningHash"`.

use crate::send::error::SendError;

use serde::Serialize;

use crate::derivation::kaspa::decode_kaspa_address;
use crate::registry::Chain;

const TX_VERSION: u16 = 0;
const SIGHASH_ALL: u8 = 1;
const SIG_OP_COUNT_DEFAULT: u8 = 1;
const KASPA_SIGHASH_KEY: &[u8] = b"TransactionSigningHash";

/// The version and payload of an address on `chain`'s own network.
fn network_address(chain: Chain, address: &str) -> Result<(u8, Vec<u8>), SendError> {
    let (version, payload, is_testnet) = decode_kaspa_address(address)?;
    if is_testnet != chain.is_testnet() {
        return Err(SendError::Invalid(
            "kaspa address belongs to another network".into(),
        ));
    }
    Ok((version, payload))
}

/// The script a wallet's own address pays: a Schnorr key's P2PK, the only
/// kind a Kaspa wallet signs for.
pub(crate) fn sender_script(chain: Chain, address: &str) -> Result<Vec<u8>, SendError> {
    let (version, payload) = network_address(chain, address)?;
    if version != 0 {
        return Err(SendError::Invalid(
            "kaspa: only Schnorr (version 0) sender addresses supported".into(),
        ));
    }
    kaspa_payment_script(version, &payload)
}

/// The script paying `address`: a Schnorr or ECDSA key's P2PK, or P2SH, on
/// `chain`'s own network.
pub(crate) fn recipient_script(chain: Chain, address: &str) -> Result<Vec<u8>, SendError> {
    let (version, payload) = network_address(chain, address)?;
    kaspa_payment_script(version, &payload)
}

/// One input and the key of the address it pays.
pub(crate) struct KaspaSigningInput<'a> {
    pub utxo: &'a (String, u32, u64, Vec<u8>),
    pub private_key: &'a [u8],
}

/// Sign `inputs`, each with its own key, into the `/transactions` request
/// paying `outputs` in order. Every input must pay its key's Schnorr P2PK.
pub(crate) fn sign(
    inputs: &[KaspaSigningInput<'_>],
    outputs: &[(Vec<u8>, u64)],
) -> Result<serde_json::Value, SendError> {
    if inputs.is_empty() || outputs.is_empty() {
        return Err(SendError::Invalid(
            "kaspa transaction must have inputs and outputs".into(),
        ));
    }
    let builds: Vec<KaspaInputBuild> = inputs
        .iter()
        .map(|input| KaspaInputBuild {
            txid: input.utxo.0.clone(),
            vout: input.utxo.1,
            sequence: 0,
            sig_op_count: SIG_OP_COUNT_DEFAULT,
            amount: input.utxo.2,
            script_pubkey: input.utxo.3.clone(),
            script_version: 0,
        })
        .collect();
    let outputs: Vec<KaspaOutputBuild> = outputs
        .iter()
        .map(|(script_pubkey, amount)| KaspaOutputBuild {
            amount: *amount,
            script_pubkey: script_pubkey.clone(),
            script_version: 0,
        })
        .collect();
    let keys: Vec<&[u8]> = inputs.iter().map(|input| input.private_key).collect();
    let signatures = sign_kaspa_inputs(&builds, &outputs, &keys)?;
    Ok(build_broadcast_body(&builds, &outputs, &signatures))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct KaspaInputBuild {
    txid: String,
    vout: u32,
    sequence: u64,
    sig_op_count: u8,
    amount: u64,
    script_pubkey: Vec<u8>,
    script_version: u16,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct KaspaOutputBuild {
    amount: u64,
    script_pubkey: Vec<u8>,
    script_version: u16,
}

/// Standard Kaspa P2PK script for a Schnorr 32-byte x-only pubkey:
///   `<32 bytes pubkey> OP_CHECKSIG (0xAC)`
/// For ECDSA (33-byte compressed pubkey, version 1): `<33 bytes> OP_CODESEPARATOR? OP_CHECKSIGECDSA (0xAB)`
/// For P2SH (32-byte script hash, version 8): `OP_BLAKE2B (0xAA) <32-byte hash> OP_EQUAL (0x87)`
fn kaspa_payment_script(version: u8, payload: &[u8]) -> Result<Vec<u8>, SendError> {
    match version {
        0x00 => {
            if payload.len() != 32 {
                return Err(SendError::Invalid(
                    "kaspa: schnorr payload must be 32 bytes".into(),
                ));
            }
            let mut s = Vec::with_capacity(34);
            s.push(0x20); // push 32 bytes
            s.extend_from_slice(payload);
            s.push(0xAC); // OP_CHECKSIG
            Ok(s)
        }
        0x01 => {
            if payload.len() != 33 {
                return Err(SendError::Invalid(
                    "kaspa: ecdsa payload must be 33 bytes".into(),
                ));
            }
            let mut s = Vec::with_capacity(35);
            s.push(0x21); // push 33 bytes
            s.extend_from_slice(payload);
            s.push(0xAB); // OP_CHECKSIGECDSA
            Ok(s)
        }
        0x08 => {
            if payload.len() != 32 {
                return Err(SendError::Invalid(
                    "kaspa: p2sh payload must be 32 bytes".into(),
                ));
            }
            let mut s = Vec::with_capacity(35);
            s.push(0xAA); // OP_BLAKE2B
            s.push(0x20);
            s.extend_from_slice(payload);
            s.push(0x87); // OP_EQUAL
            Ok(s)
        }
        v => Err(SendError::Invalid(
            format!("kaspa: unsupported address version: 0x{v:02x}").into(),
        )),
    }
}

fn decode_transaction_id(txid: &str) -> Result<[u8; 32], SendError> {
    hex::decode(txid)
        .map_err(|e| SendError::Invalid(format!("kaspa txid: {e}").into()))?
        .try_into()
        .map_err(|_| SendError::Invalid("kaspa txid must contain 32 bytes".into()))
}

// ── Sighash construction ──────────────────────────────────────────────────

fn blake2b256_keyed(key: &[u8], data: &[u8]) -> [u8; 32] {
    let hash = blake2b_simd::Params::new()
        .hash_length(32)
        .key(key)
        .hash(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(hash.as_bytes());
    out
}

fn prev_outputs_hash(inputs: &[KaspaInputBuild]) -> Result<[u8; 32], SendError> {
    let mut buf = Vec::with_capacity(36 * inputs.len());
    for input in inputs {
        buf.extend_from_slice(&decode_transaction_id(&input.txid)?);
        buf.extend_from_slice(&input.vout.to_le_bytes());
    }
    Ok(blake2b256_keyed(KASPA_SIGHASH_KEY, &buf))
}

fn sequences_hash(inputs: &[KaspaInputBuild]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(8 * inputs.len());
    for input in inputs {
        buf.extend_from_slice(&input.sequence.to_le_bytes());
    }
    blake2b256_keyed(KASPA_SIGHASH_KEY, &buf)
}

fn sig_op_counts_hash(inputs: &[KaspaInputBuild]) -> [u8; 32] {
    let buf: Vec<u8> = inputs.iter().map(|i| i.sig_op_count).collect();
    blake2b256_keyed(KASPA_SIGHASH_KEY, &buf)
}

fn outputs_hash(outputs: &[KaspaOutputBuild]) -> [u8; 32] {
    let mut buf = Vec::new();
    for output in outputs {
        buf.extend_from_slice(&output.amount.to_le_bytes());
        buf.extend_from_slice(&output.script_version.to_le_bytes());
        buf.extend_from_slice(&(output.script_pubkey.len() as u64).to_le_bytes());
        buf.extend_from_slice(&output.script_pubkey);
    }
    blake2b256_keyed(KASPA_SIGHASH_KEY, &buf)
}

fn payload_hash() -> [u8; 32] {
    [0; 32]
}

fn sighash_for_input_with_lock_time(
    inputs: &[KaspaInputBuild],
    signing_index: usize,
    prevouts: &[u8; 32],
    sequences: &[u8; 32],
    sigopcounts: &[u8; 32],
    outputs_h: &[u8; 32],
    payload_h: &[u8; 32],
    lock_time: u64,
) -> Result<[u8; 32], SendError> {
    let input = &inputs[signing_index];
    let mut buf = Vec::new();
    buf.extend_from_slice(&TX_VERSION.to_le_bytes());
    buf.extend_from_slice(prevouts);
    buf.extend_from_slice(sequences);
    buf.extend_from_slice(sigopcounts);
    // The input being signed, serialized inline.
    buf.extend_from_slice(&decode_transaction_id(&input.txid)?);
    buf.extend_from_slice(&input.vout.to_le_bytes());
    buf.extend_from_slice(&input.script_version.to_le_bytes());
    buf.extend_from_slice(&(input.script_pubkey.len() as u64).to_le_bytes());
    buf.extend_from_slice(&input.script_pubkey);
    buf.extend_from_slice(&input.amount.to_le_bytes());
    buf.extend_from_slice(&input.sequence.to_le_bytes());
    buf.push(input.sig_op_count);
    buf.extend_from_slice(outputs_h);
    buf.extend_from_slice(&lock_time.to_le_bytes());
    buf.extend_from_slice(&[0u8; 20]); // subnetwork_id (zero for native)
    buf.extend_from_slice(&0u64.to_le_bytes()); // gas
    buf.extend_from_slice(payload_h);
    buf.push(SIGHASH_ALL);
    Ok(blake2b256_keyed(KASPA_SIGHASH_KEY, &buf))
}

fn sign_kaspa_inputs(
    inputs: &[KaspaInputBuild],
    outputs: &[KaspaOutputBuild],
    private_keys: &[&[u8]],
) -> Result<Vec<Vec<u8>>, SendError> {
    use secp256k1::{Keypair, Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let mut keypairs = Vec::with_capacity(inputs.len());
    let mut outpoints = std::collections::HashSet::new();
    for (input, private_key) in inputs.iter().zip(private_keys) {
        let secret_key = SecretKey::from_slice(private_key)
            .map_err(|e| SendError::Invalid(format!("kaspa invalid privkey: {e}").into()))?;
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        if input.script_pubkey
            != kaspa_payment_script(0, &keypair.x_only_public_key().0.serialize())?
        {
            return Err(SendError::Invalid(
                "kaspa input does not belong to its signing key".into(),
            ));
        }
        if !outpoints.insert((decode_transaction_id(&input.txid)?, input.vout)) {
            return Err(SendError::Invalid("kaspa: duplicate input".into()));
        }
        keypairs.push(keypair);
    }
    if keypairs.len() != inputs.len() {
        return Err(SendError::Invalid("kaspa: one key per input".into()));
    }

    let prevouts = prev_outputs_hash(inputs)?;
    let sequences = sequences_hash(inputs);
    let sigopcounts = sig_op_counts_hash(inputs);
    let outputs_h = outputs_hash(outputs);
    let payload_h = payload_hash();

    let mut signed = Vec::with_capacity(inputs.len());
    for (i, keypair) in keypairs.iter().enumerate() {
        let sighash = sighash_for_input_with_lock_time(
            inputs,
            i,
            &prevouts,
            &sequences,
            &sigopcounts,
            &outputs_h,
            &payload_h,
            0,
        )?;
        let msg = Message::from_digest_slice(&sighash).map_err(SendError::invalid)?;
        let sig = secp.sign_schnorr(&msg, keypair);
        let mut sig_with_type = sig.as_ref().to_vec();
        sig_with_type.push(SIGHASH_ALL);

        // Standard Schnorr P2PK signature script: `<sig_with_sighash_type>` (push opcode).
        let mut script_sig = Vec::with_capacity(2 + sig_with_type.len());
        script_sig.push(sig_with_type.len() as u8);
        script_sig.extend_from_slice(&sig_with_type);
        signed.push(script_sig);
    }
    Ok(signed)
}

// ── REST broadcast body ───────────────────────────────────────────────────
//
// Canonical /transactions request shape per kaspa-rest spec:
//   {"transaction": {"version": 0, "inputs": [...], "outputs": [...],
//                    "lockTime": "0", "subnetworkId": "00…",
//                    "gas": "0", "payload": ""}}
//   inputs[i] = {"previousOutpoint": {"transactionId": txid,
//                                     "index": vout},
//                "signatureScript": <hex>,
//                "sequence": "0",
//                "sigOpCount": 1}
//   outputs[i] = {"value": "<amount>",
//                 "scriptPublicKey": {"version": 0, "scriptPublicKey": <hex>}}

#[derive(Serialize)]
struct WireOutpoint {
    #[serde(rename = "transactionId")]
    transaction_id: String,
    index: u32,
}

#[derive(Serialize)]
struct WireInput {
    #[serde(rename = "previousOutpoint")]
    previous_outpoint: WireOutpoint,
    #[serde(rename = "signatureScript")]
    signature_script: String,
    sequence: String,
    #[serde(rename = "sigOpCount")]
    sig_op_count: u8,
}

#[derive(Serialize)]
struct WireScriptPublicKey {
    version: u16,
    #[serde(rename = "scriptPublicKey")]
    script_public_key: String,
}

#[derive(Serialize)]
struct WireOutput {
    value: String,
    #[serde(rename = "scriptPublicKey")]
    script_public_key: WireScriptPublicKey,
}

#[derive(Serialize)]
struct WireTransaction {
    version: u16,
    inputs: Vec<WireInput>,
    outputs: Vec<WireOutput>,
    #[serde(rename = "lockTime")]
    lock_time: String,
    #[serde(rename = "subnetworkId")]
    subnetwork_id: String,
    gas: String,
    payload: String,
}

#[derive(Serialize)]
struct WireBroadcast {
    transaction: WireTransaction,
}

fn build_broadcast_body(
    inputs: &[KaspaInputBuild],
    outputs: &[KaspaOutputBuild],
    signed_sig_scripts: &[Vec<u8>],
) -> serde_json::Value {
    let wire_inputs = inputs
        .iter()
        .zip(signed_sig_scripts)
        .map(|(input, script)| WireInput {
            previous_outpoint: WireOutpoint {
                transaction_id: input.txid.clone(),
                index: input.vout,
            },
            signature_script: hex::encode(script),
            sequence: input.sequence.to_string(),
            sig_op_count: input.sig_op_count,
        })
        .collect();
    let wire_outputs = outputs
        .iter()
        .map(|output| WireOutput {
            value: output.amount.to_string(),
            script_public_key: WireScriptPublicKey {
                version: output.script_version,
                script_public_key: hex::encode(&output.script_pubkey),
            },
        })
        .collect();
    let body = WireBroadcast {
        transaction: WireTransaction {
            version: TX_VERSION,
            inputs: wire_inputs,
            outputs: wire_outputs,
            lock_time: "0".to_string(),
            subnetwork_id: "0000000000000000000000000000000000000000".to_string(),
            gas: "0".to_string(),
            payload: String::new(),
        },
    };
    serde_json::to_value(body).expect("static schema")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// kaspa-wasm's per-input sighashes for an account spend with two keys
    /// (`account-utxo-transactions.json`), and each signature verifies under
    /// its own input's key.
    #[test]
    fn account_inputs_sign_kaspa_wasm_sighashes_each_with_its_key() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/account-utxo-transactions.json"
        ))
        .unwrap();
        let vector = &fixtures["kaspa"];
        let utxos: Vec<(String, u32, u64, Vec<u8>)> = vector["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| {
                (
                    input["txid"].as_str().unwrap().to_string(),
                    input["vout"].as_u64().unwrap() as u32,
                    input["value"].as_u64().unwrap(),
                    hex::decode(input["script"].as_str().unwrap()).unwrap(),
                )
            })
            .collect();
        let keys: Vec<Vec<u8>> = vector["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| hex::decode(input["key"].as_str().unwrap()).unwrap())
            .collect();
        let outputs: Vec<(Vec<u8>, u64)> = vector["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|output| {
                (
                    hex::decode(output["script"].as_str().unwrap()).unwrap(),
                    output["value"].as_u64().unwrap(),
                )
            })
            .collect();
        let inputs: Vec<_> = utxos
            .iter()
            .zip(&keys)
            .map(|(utxo, key)| KaspaSigningInput {
                utxo,
                private_key: key,
            })
            .collect();
        let body = sign(&inputs, &outputs).unwrap();
        let builds: Vec<KaspaInputBuild> = utxos
            .iter()
            .map(|utxo| KaspaInputBuild {
                txid: utxo.0.clone(),
                vout: utxo.1,
                sequence: 0,
                sig_op_count: SIG_OP_COUNT_DEFAULT,
                amount: utxo.2,
                script_pubkey: utxo.3.clone(),
                script_version: 0,
            })
            .collect();
        let output_builds: Vec<KaspaOutputBuild> = outputs
            .iter()
            .map(|(script_pubkey, amount)| KaspaOutputBuild {
                amount: *amount,
                script_pubkey: script_pubkey.clone(),
                script_version: 0,
            })
            .collect();
        let secp = secp256k1::Secp256k1::verification_only();
        for (index, expected) in vector["sighashes"].as_array().unwrap().iter().enumerate() {
            let digest = sighash_for_input_with_lock_time(
                &builds,
                index,
                &prev_outputs_hash(&builds).unwrap(),
                &sequences_hash(&builds),
                &sig_op_counts_hash(&builds),
                &outputs_hash(&output_builds),
                &payload_hash(),
                0,
            )
            .unwrap();
            assert_eq!(
                hex::encode(digest),
                expected.as_str().unwrap(),
                "input {index}"
            );
            let script = hex::decode(
                body["transaction"]["inputs"][index]["signatureScript"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!((script[0], script[65]), (65, SIGHASH_ALL));
            let key = secp256k1::XOnlyPublicKey::from_slice(&utxos[index].3[1..33]).unwrap();
            secp.verify_schnorr(
                &secp256k1::schnorr::Signature::from_slice(&script[1..65]).unwrap(),
                &secp256k1::Message::from_digest(digest),
                &key,
            )
            .unwrap();
        }
        // A key that does not own its input signs nothing.
        let mut swapped = keys.clone();
        swapped.swap(0, 1);
        let wrong: Vec<_> = utxos
            .iter()
            .zip(&swapped)
            .map(|(utxo, key)| KaspaSigningInput {
                utxo,
                private_key: key,
            })
            .collect();
        assert!(sign(&wrong, &outputs).is_err());
    }

    /// rusty-kaspa consensus/core/src/hashing/sighash.rs, native-all-0.
    /// This is an independent expected digest, not a round trip through our encoder.
    #[test]
    fn kaspa_official_native_sighash_vector() {
        let id = "880eb9819a31821d9d2399e2f35e2433b72637e393d71ecc9b8d0250f49153c3";
        let a = hex::decode("208325613d2eeaf7176ac6c670b13c0043156c427438ed72d74b7800862ad884e8ac")
            .unwrap();
        let b = hex::decode("20fcef4c106cf11135bbd70f02a726a92162d2fb8b22f0469126f800862ad884e8ac")
            .unwrap();
        let inputs = (0..3)
            .map(|i| KaspaInputBuild {
                txid: id.into(),
                vout: i,
                sequence: i as u64,
                sig_op_count: 0,
                amount: (i as u64 + 1) * 100,
                script_pubkey: if i == 0 { a.clone() } else { b.clone() },
                script_version: 0,
            })
            .collect::<Vec<_>>();
        let outputs = [b, a]
            .into_iter()
            .map(|script_pubkey| KaspaOutputBuild {
                amount: 300,
                script_pubkey,
                script_version: 0,
            })
            .collect::<Vec<_>>();
        let digest = sighash_for_input_with_lock_time(
            &inputs,
            0,
            &prev_outputs_hash(&inputs).unwrap(),
            &sequences_hash(&inputs),
            &sig_op_counts_hash(&inputs),
            &outputs_hash(&outputs),
            &payload_hash(),
            1615462089000,
        )
        .unwrap();
        assert_eq!(
            hex::encode(digest),
            "03b7ac6927b2b67100734c3cc313ff8c2e8b3ce3e746d46dd660b706a916b1f5"
        );
    }
}
