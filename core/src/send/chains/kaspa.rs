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

use serde::Serialize;

use crate::derivation::chains::kaspa::{decode_kaspa_address, encode_kaspa_schnorr};
use crate::fetch::chains::kaspa::{KasSendResult, KaspaClient};

const TX_VERSION: u16 = 0;
const SIGHASH_ALL: u8 = 1;
const SIG_OP_COUNT_DEFAULT: u8 = 1;
const KASPA_SIGHASH_KEY: &[u8] = b"TransactionSigningHash";

impl KaspaClient {
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sompi: u64,
        fee_sompi: u64,
        private_key_bytes: &[u8],
        min_fee_sompi: Option<u64>,
        dust_threshold_sompi: Option<u64>,
    ) -> Result<KasSendResult, String> {
        let secret =
            secp256k1::SecretKey::from_slice(private_key_bytes).map_err(|e| e.to_string())?;
        let public = secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), &secret);
        if encode_kaspa_schnorr(&public.x_only_public_key().0.serialize()) != from_address {
            return Err("kaspa sender does not match signing key".into());
        }
        if amount_sompi == 0 {
            return Err("kaspa amount must be positive".into());
        }
        let utxos = self.fetch_utxos(from_address).await?;
        if utxos.is_empty() {
            return Err("kaspa: no spendable UTXOs at source address".to_string());
        }
        let from_decoded = decode_kaspa_address(from_address)?;
        let to_decoded = decode_kaspa_address(to_address)?;
        if from_decoded.0 != 0 {
            return Err("kaspa: only Schnorr (version 0) sender addresses supported".to_string());
        }
        if to_decoded.0 != 0 && to_decoded.0 != 1 && to_decoded.0 != 8 {
            return Err(format!(
                "kaspa: unsupported destination version 0x{:02x}",
                to_decoded.0
            ));
        }

        let total_in = utxos.iter().try_fold(0u64, |sum, u| {
            sum.checked_add(u.value_sompi)
                .ok_or("kaspa input sum overflow")
        })?;
        let actual_fee = fee_sompi.max(min_fee_sompi.unwrap_or(1_000));
        let needed = amount_sompi
            .checked_add(actual_fee)
            .ok_or("kaspa amount plus fee overflow")?;
        if total_in < needed {
            return Err(format!(
                "kaspa: insufficient balance: have {total_in} sompi, need {needed} sompi"
            ));
        }
        let change = total_in - needed;

        // Outputs: recipient + optional change. Kaspa dust threshold is 1000
        // sompi for a 2-output Schnorr send; below that we drop the change
        // output and let it become fee.
        let mut outputs: Vec<KaspaOutputBuild> = vec![KaspaOutputBuild {
            amount: amount_sompi,
            script_pubkey: kaspa_payment_script(to_decoded.0, &to_decoded.1)?,
            script_version: 0,
        }];
        if change > dust_threshold_sompi.unwrap_or(1_000) {
            outputs.push(KaspaOutputBuild {
                amount: change,
                script_pubkey: kaspa_payment_script(from_decoded.0, &from_decoded.1)?,
                script_version: 0,
            });
        }

        // Per-input snapshot needed for both sighash and the final wire body.
        let inputs: Vec<KaspaInputBuild> = utxos
            .iter()
            .map(|u| {
                let script_pubkey = hex::decode(&u.script_pubkey_hex)
                    .map_err(|e| format!("kaspa utxo script hex: {e}"))?;
                Ok::<KaspaInputBuild, String>(KaspaInputBuild {
                    txid: u.txid.clone(),
                    vout: u.vout,
                    sequence: 0,
                    sig_op_count: SIG_OP_COUNT_DEFAULT,
                    amount: u.value_sompi,
                    script_pubkey,
                    script_version: u.script_version as u16,
                })
            })
            .collect::<Result<_, _>>()?;

        let signed = sign_kaspa_inputs(&inputs, &outputs, private_key_bytes)?;
        let body = build_broadcast_body(&inputs, &outputs, &signed);
        self.broadcast_tx_body(body).await
    }
}

struct KaspaInputBuild {
    txid: String,
    vout: u32,
    sequence: u64,
    sig_op_count: u8,
    amount: u64,
    script_pubkey: Vec<u8>,
    script_version: u16,
}

struct KaspaOutputBuild {
    amount: u64,
    script_pubkey: Vec<u8>,
    script_version: u16,
}

/// Standard Kaspa P2PK script for a Schnorr 32-byte x-only pubkey:
///   `<32 bytes pubkey> OP_CHECKSIG (0xAC)`
/// For ECDSA (33-byte compressed pubkey, version 1): `<33 bytes> OP_CODESEPARATOR? OP_CHECKSIGECDSA (0xAB)`
/// For P2SH (32-byte script hash, version 8): `OP_BLAKE2B (0xAA) <32-byte hash> OP_EQUAL (0x87)`
fn kaspa_payment_script(version: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
    match version {
        0x00 => {
            if payload.len() != 32 {
                return Err("kaspa: schnorr payload must be 32 bytes".to_string());
            }
            let mut s = Vec::with_capacity(34);
            s.push(0x20); // push 32 bytes
            s.extend_from_slice(payload);
            s.push(0xAC); // OP_CHECKSIG
            Ok(s)
        }
        0x01 => {
            if payload.len() != 33 {
                return Err("kaspa: ecdsa payload must be 33 bytes".to_string());
            }
            let mut s = Vec::with_capacity(35);
            s.push(0x21); // push 33 bytes
            s.extend_from_slice(payload);
            s.push(0xAB); // OP_CHECKSIGECDSA
            Ok(s)
        }
        0x08 => {
            if payload.len() != 32 {
                return Err("kaspa: p2sh payload must be 32 bytes".to_string());
            }
            let mut s = Vec::with_capacity(35);
            s.push(0xAA); // OP_BLAKE2B
            s.push(0x20);
            s.extend_from_slice(payload);
            s.push(0x87); // OP_EQUAL
            Ok(s)
        }
        v => Err(format!("kaspa: unsupported address version: 0x{v:02x}")),
    }
}

fn decode_transaction_id(txid: &str) -> Result<[u8; 32], String> {
    hex::decode(txid)
        .map_err(|e| format!("kaspa txid: {e}"))?
        .try_into()
        .map_err(|_| "kaspa txid must contain 32 bytes".into())
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

fn prev_outputs_hash(inputs: &[KaspaInputBuild]) -> Result<[u8; 32], String> {
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
) -> Result<[u8; 32], String> {
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
    private_key_bytes: &[u8],
) -> Result<Vec<Vec<u8>>, String> {
    use secp256k1::{Keypair, Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(private_key_bytes)
        .map_err(|e| format!("kaspa invalid privkey: {e}"))?;
    let keypair = Keypair::from_secret_key(&secp, &secret_key);

    let prevouts = prev_outputs_hash(inputs)?;
    let sequences = sequences_hash(inputs);
    let sigopcounts = sig_op_counts_hash(inputs);
    let outputs_h = outputs_hash(outputs);
    let payload_h = payload_hash();

    let mut signed = Vec::with_capacity(inputs.len());
    for i in 0..inputs.len() {
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
        let msg = Message::from_digest_slice(&sighash).map_err(|e| e.to_string())?;
        let sig = secp.sign_schnorr(&msg, &keypair);
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
