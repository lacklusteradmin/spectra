//! Dogecoin send: P2PKH signer and Blockbook broadcast.

use crate::http::{with_fallback, RetryProfile};

use crate::derivation::chains::dogecoin::{decode_doge_address, p2pkh_script};
use crate::fetch::chains::dogecoin::{DogeSendResult, DogecoinClient};

impl DogecoinClient {
    /// Broadcast a raw hex transaction.
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<DogeSendResult, String> {
        let hex = hex_tx.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let hex = hex.clone();
            let url = format!("{}/api/v2/sendtx/", base.trim_end_matches('/'));
            async move {
                let txid: String = client
                    .post_text(&url, hex.clone(), RetryProfile::ChainWrite)
                    .await?;
                Ok(DogeSendResult {
                    txid: txid.trim().to_string(),
                    raw_tx_hex: hex.clone(),
                })
            }
        })
        .await
    }

    /// Fetch UTXOs for `from_address`, sign a P2PKH transaction, and broadcast.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
        dust_threshold: Option<u64>,
    ) -> Result<DogeSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let script_pubkey = p2pkh_script(&decode_doge_address(from_address)?)?;
        let utxo_tuples: Vec<(String, u32, u64, Vec<u8>)> = utxos
            .iter()
            .map(|u| (u.txid.clone(), u.vout, u.value_koin, script_pubkey.clone()))
            .collect();
        let raw = sign_doge_p2pkh(
            &utxo_tuples,
            to_address,
            amount_sat,
            fee_sat,
            from_address,
            private_key_bytes,
            dust_threshold,
        )?;
        self.broadcast_raw_tx(&hex::encode(&raw)).await
    }
}

// ----------------------------------------------------------------
// Dogecoin P2PKH signing
// ----------------------------------------------------------------

/// Sign and serialize a Dogecoin P2PKH transaction.
///
/// `utxos` — selected UTXOs with their redeeming scripts (the previous P2PKH
/// scriptPubKey for each input).
/// Returns raw transaction bytes ready for broadcast.
pub fn sign_doge_p2pkh(
    utxos: &[(String, u32, u64, Vec<u8>)], // (txid, vout, value_koin, script_pubkey)
    to_address: &str,
    amount_koin: u64,
    fee_koin: u64,
    change_address: &str,
    private_key_bytes: &[u8],
    dust_threshold: Option<u64>,
) -> Result<Vec<u8>, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let pubkey_bytes = pubkey.serialize(); // compressed

    let total_in: u64 = utxos.iter().map(|(_, _, v, _)| v).sum();
    let change = total_in.saturating_sub(amount_koin + fee_koin);

    // Build outputs.
    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(
        p2pkh_script(&decode_doge_address(to_address)?)?,
        amount_koin,
    )];
    if change > dust_threshold.unwrap_or(546) {
        outputs.push((p2pkh_script(&decode_doge_address(change_address)?)?, change));
    }

    // Sign each input.
    let mut signed_inputs: Vec<Vec<u8>> = Vec::new();
    for (txid, vout, _, script_pubkey) in utxos {
        // SIGHASH_ALL preimage.
        let preimage = build_sighash_preimage(utxos, *vout, txid, script_pubkey, &outputs, 1)?;
        let hash = dsha256(&preimage);
        let msg = Message::from_digest_slice(&hash).map_err(|e| e.to_string())?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(0x01); // SIGHASH_ALL

        let script_sig = build_p2pkh_script_sig(&der, &pubkey_bytes);
        signed_inputs.push(build_input(txid, *vout, &script_sig, 0xffffffff));
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
) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    // version
    raw.extend_from_slice(&1u32.to_le_bytes());
    // inputs
    raw.extend_from_slice(&varint(utxos.len()));
    for (txid, vout, _, spk) in utxos {
        let txid_bytes = decode_txid(txid)?;
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
    // outputs
    raw.extend_from_slice(&varint(outputs.len()));
    for (script, value) in outputs {
        raw.extend_from_slice(&value.to_le_bytes());
        raw.extend_from_slice(&varint(script.len()));
        raw.extend_from_slice(script);
    }
    // locktime
    raw.extend_from_slice(&0u32.to_le_bytes());
    // sighash type
    raw.extend_from_slice(&sighash_type.to_le_bytes());
    Ok(raw)
}

fn build_input(txid: &str, vout: u32, script_sig: &[u8], sequence: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let txid_bytes = decode_txid(txid).unwrap_or_default();
    out.extend_from_slice(&txid_bytes);
    out.extend_from_slice(&vout.to_le_bytes());
    out.extend_from_slice(&varint(script_sig.len()));
    out.extend_from_slice(script_sig);
    out.extend_from_slice(&sequence.to_le_bytes());
    out
}

fn build_p2pkh_script_sig(der_sig: &[u8], pubkey: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(der_sig.len() as u8);
    out.extend_from_slice(der_sig);
    out.push(pubkey.len() as u8);
    out.extend_from_slice(pubkey);
    out
}

fn build_tx(inputs: &[Vec<u8>], outputs: &[(Vec<u8>, u64)]) -> Vec<u8> {
    let mut raw = Vec::new();
    raw.extend_from_slice(&1u32.to_le_bytes()); // version
    raw.extend_from_slice(&varint(inputs.len()));
    for inp in inputs {
        raw.extend_from_slice(inp);
    }
    raw.extend_from_slice(&varint(outputs.len()));
    for (script, value) in outputs {
        raw.extend_from_slice(&value.to_le_bytes());
        raw.extend_from_slice(&varint(script.len()));
        raw.extend_from_slice(script);
    }
    raw.extend_from_slice(&0u32.to_le_bytes()); // locktime
    raw
}

fn decode_txid(txid: &str) -> Result<Vec<u8>, String> {
    let mut bytes = hex::decode(txid).map_err(|e| format!("txid decode: {e}"))?;
    bytes.reverse(); // little-endian
    Ok(bytes)
}

fn dsha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let first = Sha256::digest(data);
    let second = Sha256::digest(first);
    second.into()
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
