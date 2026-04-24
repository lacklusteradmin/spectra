//! Cardano send: minimal CBOR encoder for an ADA-only Shelley transfer,
//! Blockfrost submission.

use crate::http::{with_fallback, RetryProfile};

use crate::derivation::chains::cardano::decode_cardano_addr_bytes;
use crate::fetch::chains::cardano::{CardanoClient, CardanoSendResult};

impl CardanoClient {
    /// Fetch UTXOs, sign an ADA-only P2PKH-equivalent Shelley transaction, and submit.
    ///
    /// `to_address` and `from_address` must be Shelley bech32 or Byron base58 addresses.
    /// `fee_lovelace` is caller-supplied (estimate from Blockfrost or a fixed value).
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_lovelace: u64,
        fee_lovelace: u64,
        signing_key_bytes: &[u8; 64],
        verification_key_bytes: &[u8; 32],
    ) -> Result<CardanoSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let slot = self.fetch_latest_slot().await?;
        let ttl = slot + 7200; // valid for ~2 hours (2 slots/s * 3600 * 2)

        let to_addr_bytes = decode_cardano_addr_bytes(to_address)?;
        let change_addr_bytes = decode_cardano_addr_bytes(from_address)?;

        let utxo_tuples: Vec<(String, u32, u64)> = utxos
            .iter()
            .map(|u| (u.tx_hash.clone(), u.tx_index, u.lovelace))
            .collect();

        let cbor_hex = build_signed_ada_tx(
            &utxo_tuples,
            &to_addr_bytes,
            amount_lovelace,
            fee_lovelace,
            &change_addr_bytes,
            signing_key_bytes,
            verification_key_bytes,
            ttl,
        )?;
        self.submit_tx(&cbor_hex).await
    }

    /// Submit a CBOR-encoded signed transaction.
    pub async fn submit_tx(&self, cbor_hex: &str) -> Result<CardanoSendResult, String> {
        let cbor_hex_owned = cbor_hex.to_string();
        let cbor_bytes = hex::decode(cbor_hex).map_err(|e| format!("hex decode: {e}"))?;
        let api_key = self.api_key.clone();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let cbor_bytes = cbor_bytes.clone();
            let api_key = api_key.clone();
            let cbor_hex_owned = cbor_hex_owned.clone();
            let url = format!("{}/tx/submit", base.trim_end_matches('/'));
            async move {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&cbor_bytes);
                let body = serde_json::json!({ "cbor": b64 });
                let mut headers = std::collections::HashMap::new();
                headers.insert("project_id", api_key.as_str());
                let txid: String = client
                    .post_json_with_headers(&url, &body, &headers, RetryProfile::ChainWrite)
                    .await?;
                Ok(CardanoSendResult { txid, cbor_hex: cbor_hex_owned })
            }
        })
        .await
    }
}

// ----------------------------------------------------------------
// Cardano transaction building (minimal CBOR for ADA-only transfer)
// ----------------------------------------------------------------

/// Build a signed Shelley-era ADA transfer transaction.
/// Returns raw CBOR bytes as hex.
#[allow(clippy::too_many_arguments)]
pub fn build_signed_ada_tx(
    utxos: &[(String, u32, u64)], // (tx_hash, tx_index, lovelace)
    to_address_bytes: &[u8],
    amount_lovelace: u64,
    fee_lovelace: u64,
    change_address_bytes: &[u8],
    signing_key_bytes: &[u8; 64],
    verification_key_bytes: &[u8; 32],
    ttl: u64,
) -> Result<String, String> {
    use ed25519_dalek::{Signer, SigningKey};

    let total_in: u64 = utxos.iter().map(|(_, _, v)| v).sum();
    let change = total_in.saturating_sub(amount_lovelace + fee_lovelace);

    // Encode transaction body (map with fields 0-3).
    let mut outputs: Vec<(&[u8], u64)> = vec![(to_address_bytes, amount_lovelace)];
    if change > 1_000_000 {
        // min ADA per output
        outputs.push((change_address_bytes, change));
    }

    let tx_body = encode_tx_body(utxos, &outputs, fee_lovelace, ttl);

    // Transaction body hash (Blake2b-256).
    let body_hash = blake2b_256(&tx_body);

    // Sign.
    let signing_key = SigningKey::from_bytes(
        &signing_key_bytes[..32]
            .try_into()
            .map_err(|_| "key too short")?,
    );
    let signature = signing_key.sign(&body_hash);

    // Witness set: [{0: [[vkey, sig]]}]
    let witness_set = encode_witness_set(verification_key_bytes, signature.to_bytes().as_ref());

    // Transaction: [tx_body, witness_set, true, null]
    let tx = cbor_array(&[tx_body.clone(), witness_set, cbor_bool(true), cbor_null()]);

    Ok(hex::encode(&tx))
}

fn encode_tx_body(
    inputs: &[(String, u32, u64)],
    outputs: &[(&[u8], u64)],
    fee: u64,
    ttl: u64,
) -> Vec<u8> {
    // CBOR map {0: inputs, 1: outputs, 2: fee, 3: ttl}
    let mut map_entries = Vec::new();

    // Inputs (field 0): set of [tx_hash, index]
    let encoded_inputs: Vec<Vec<u8>> = inputs
        .iter()
        .map(|(hash, idx, _)| {
            let hash_bytes = hex::decode(hash).unwrap_or_default();
            cbor_array(&[cbor_bytes(&hash_bytes), cbor_uint(*idx as u64)])
        })
        .collect();
    map_entries.push((cbor_uint(0), cbor_tagged_set(&encoded_inputs)));

    // Outputs (field 1): array of [address, lovelace]
    let encoded_outputs: Vec<Vec<u8>> = outputs
        .iter()
        .map(|(addr, lovelace)| cbor_array(&[cbor_bytes(addr), cbor_uint(*lovelace)]))
        .collect();
    map_entries.push((cbor_uint(1), cbor_array_of(&encoded_outputs)));

    // Fee (field 2)
    map_entries.push((cbor_uint(2), cbor_uint(fee)));

    // TTL (field 3)
    map_entries.push((cbor_uint(3), cbor_uint(ttl)));

    cbor_map(&map_entries)
}

fn encode_witness_set(vkey: &[u8], sig: &[u8]) -> Vec<u8> {
    // {0: [[vkey_bytes, sig_bytes]]}
    let vkey_sig = cbor_array(&[cbor_bytes(vkey), cbor_bytes(sig)]);
    cbor_map(&[(cbor_uint(0), cbor_array_of(&[vkey_sig]))])
}

// ----------------------------------------------------------------
// Minimal CBOR encoder
// ----------------------------------------------------------------

fn cbor_uint(n: u64) -> Vec<u8> {
    if n <= 23 {
        vec![n as u8]
    } else if n <= 0xff {
        vec![0x18, n as u8]
    } else if n <= 0xffff {
        let mut v = vec![0x19];
        v.extend_from_slice(&(n as u16).to_be_bytes());
        v
    } else if n <= 0xffff_ffff {
        let mut v = vec![0x1a];
        v.extend_from_slice(&(n as u32).to_be_bytes());
        v
    } else {
        let mut v = vec![0x1b];
        v.extend_from_slice(&n.to_be_bytes());
        v
    }
}

fn cbor_bytes(data: &[u8]) -> Vec<u8> {
    let mut out = cbor_len_prefix(2, data.len());
    out.extend_from_slice(data);
    out
}

fn cbor_array(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = cbor_len_prefix(4, items.len());
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn cbor_array_of(items: &[Vec<u8>]) -> Vec<u8> {
    cbor_array(items)
}

fn cbor_tagged_set(items: &[Vec<u8>]) -> Vec<u8> {
    // Tag 258 = finite set
    let mut out = vec![0xd9, 0x01, 0x02]; // tag(258)
    out.extend_from_slice(&cbor_array(items));
    out
}

fn cbor_map(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = cbor_len_prefix(5, entries.len());
    for (k, v) in entries {
        out.extend_from_slice(k);
        out.extend_from_slice(v);
    }
    out
}

fn cbor_bool(b: bool) -> Vec<u8> {
    vec![if b { 0xf5 } else { 0xf4 }]
}

fn cbor_null() -> Vec<u8> {
    vec![0xf6]
}

fn cbor_len_prefix(major: u8, len: usize) -> Vec<u8> {
    let major = major << 5;
    if len <= 23 {
        vec![major | len as u8]
    } else if len <= 0xff {
        vec![major | 24, len as u8]
    } else if len <= 0xffff {
        let mut v = vec![major | 25];
        v.extend_from_slice(&(len as u16).to_be_bytes());
        v
    } else {
        let mut v = vec![major | 26];
        v.extend_from_slice(&(len as u32).to_be_bytes());
        v
    }
}

fn blake2b_256(data: &[u8]) -> [u8; 32] {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};
    let mut h = Blake2b::<U32>::new();
    h.update(data);
    h.finalize().into()
}
