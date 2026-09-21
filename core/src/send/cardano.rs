//! Cardano send: minimal CBOR encoder for an ADA-only Shelley transfer,
//! keyless Koios submission.

use crate::fetch::http::{http_request, with_fallback, HttpHeader, HttpRetryProfile};

use crate::derivation::cardano::decode_cardano_addr_bytes;
use crate::fetch::cardano::{CardanoClient, CardanoSendResult};

impl CardanoClient {
    /// Fetch UTXOs, sign an ADA-only P2PKH-equivalent Shelley transaction, and submit.
    ///
    /// `to_address` and `from_address` must be Shelley bech32 or Byron base58 addresses.
    /// `fee_lovelace` is caller-supplied (reviewed fee estimate).
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_lovelace: u64,
        fee_lovelace: u64,
        signing_key_bytes: &[u8; 64],
        verification_key_bytes: &[u8; 32],
        ttl_slots: Option<u64>,
        min_change_lovelace: Option<u64>,
    ) -> Result<CardanoSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let slot = self.fetch_latest_slot().await?;
        let ttl = slot + ttl_slots.unwrap_or(7200);

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
            min_change_lovelace,
        )?;
        self.submit_tx(&cbor_hex).await
    }

    /// Submit a CBOR-encoded signed transaction.
    pub async fn submit_tx(&self, cbor_hex: &str) -> Result<CardanoSendResult, String> {
        crate::send::payload::before_submission(
            serde_json::json!({"cbor_hex":cbor_hex}).to_string(),
            "txid",
            None,
            None,
        )
        .await?;
        let cbor_hex_owned = cbor_hex.to_string();
        let cbor_bytes = hex::decode(cbor_hex).map_err(|e| format!("hex decode: {e}"))?;
        with_fallback(&self.endpoints, |base| {
            let cbor_bytes = cbor_bytes.clone();
            let cbor_hex = cbor_hex_owned.clone();
            let url = format!("{}/submittx", base.trim_end_matches('/'));
            async move {
                // Koios submit-api accepts raw CBOR and returns a JSON transaction hash.
                let response = http_request(
                    "POST".into(),
                    url,
                    vec![HttpHeader {
                        name: "Content-Type".into(),
                        value: "application/cbor".into(),
                    }],
                    Some(cbor_bytes),
                    HttpRetryProfile::ChainWrite,
                )
                .await
                .map_err(|e| e.to_string())?;
                if response.status_code != 202 {
                    return Err(format!(
                        "Koios submission: expected HTTP 202, received {}",
                        response.status_code
                    ));
                }
                let txid: String = serde_json::from_slice(&response.body)
                    .map_err(|e| format!("Koios transaction id: {e}"))?;
                if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Koios submission returned an invalid transaction id".into());
                }
                Ok(CardanoSendResult { txid, cbor_hex })
            }
        })
        .await
    }
}

// ── Cardano transaction building (minimal CBOR for ADA-only transfer)

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
    min_change_lovelace: Option<u64>,
) -> Result<String, String> {
    use ed25519_dalek::{Signer, SigningKey};

    let change = super::accounting::checked_change(
        utxos.iter().map(|(_, _, v)| *v),
        amount_lovelace,
        fee_lovelace,
    )?;

    // Encode transaction body (map with fields 0-3).
    let mut outputs: Vec<(&[u8], u64)> = vec![(to_address_bytes, amount_lovelace)];
    if change > 0 && change < min_change_lovelace.unwrap_or(1_000_000) {
        return Err("change below minimum output; choose an exact amount or fee".into());
    }
    if change > 0 {
        outputs.push((change_address_bytes, change));
    }

    let tx_body = encode_tx_body(utxos, &outputs, fee_lovelace, ttl)?;

    let body_hash = blake2b_256(&tx_body);

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
) -> Result<Vec<u8>, String> {
    // CBOR map {0: inputs, 1: outputs, 2: fee, 3: ttl}
    let mut map_entries = Vec::new();

    // Inputs (field 0): set of [tx_hash, index]
    let encoded_inputs: Vec<Vec<u8>> = inputs
        .iter()
        .map(|(hash, idx, _)| {
            let hash_bytes = hex::decode(hash).map_err(|e| format!("input txid: {e}"))?;
            if hash_bytes.len() != 32 {
                return Err("input txid must contain exactly 32 bytes".into());
            }
            Ok(cbor_array(&[
                cbor_bytes(&hash_bytes),
                cbor_uint(*idx as u64),
            ]))
        })
        .collect::<Result<_, String>>()?;
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

    Ok(cbor_map(&map_entries))
}

fn encode_witness_set(vkey: &[u8], sig: &[u8]) -> Vec<u8> {
    // {0: [[vkey_bytes, sig_bytes]]}
    let vkey_sig = cbor_array(&[cbor_bytes(vkey), cbor_bytes(sig)]);
    cbor_map(&[(cbor_uint(0), cbor_array_of(&[vkey_sig]))])
}

// ── Minimal CBOR encoder

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
    let mut out = vec![0xd9, 0x01, 0x02];
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

#[cfg(test)]
mod accounting_tests {
    use super::*;
    fn build(values: &[u64], amount: u64, fee: u64) -> Result<String, String> {
        let inputs: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(i, v)| ("00".repeat(32), i as u32, *v))
            .collect();
        build_signed_ada_tx(
            &inputs,
            &[0x61; 29],
            amount,
            fee,
            &[0x62; 29],
            &[1; 64],
            &[2; 32],
            100,
            Some(1000000),
        )
    }
    #[test]
    fn cardano_refuses_malformed_input_hashes() {
        for hash in [
            String::new(),
            "not hex".into(),
            "00".repeat(31),
            "00".repeat(33),
        ] {
            let result = build_signed_ada_tx(
                &[(hash, 0, 1170000)],
                &[0x61; 29],
                1000000,
                170000,
                &[0x62; 29],
                &[1; 64],
                &[2; 32],
                100,
                None,
            );
            assert!(result.unwrap_err().contains("txid"));
        }
    }
    #[test]
    fn cardano_refuses_unbalanced_or_dust_transactions() {
        assert!(build(&[2000000], 2000000, 1).is_err());
        assert!(build(&[2000000], 1000000, 170000).is_err());
        assert!(build(&[u64::MAX, 1], 1, 1).is_err());
        assert!(build(&[u64::MAX], u64::MAX, 1).is_err());
        assert!(build(&[], 1, 1).is_err());
        assert!(build(&[1170000], 1000000, 170000).is_ok());
        assert!(
            build(&[2170000], 1000000, 170000).is_ok(),
            "minimum change is valid"
        );
    }
}

#[cfg(test)]
mod keyless_submission_tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{
        matchers::{body_bytes, header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[tokio::test]
    async fn koios_receives_raw_cbor_without_credentials_and_requires_a_transaction_id() {
        let server = MockServer::start().await;
        let client = CardanoClient::new(Arc::new(vec![server.uri()]));
        let txid = "ab".repeat(32);
        Mock::given(method("POST"))
            .and(path("/submittx"))
            .and(header("content-type", "application/cbor"))
            .and(body_bytes(vec![0x81, 0x00]))
            .respond_with(ResponseTemplate::new(202).set_body_json(&txid))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(client.submit_tx("8100").await.unwrap().txid, txid);
        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].headers.contains_key("authorization"));
        assert!(!requests[0].headers.contains_key("project_id"));
        assert!(requests[0].url.query().is_none());
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(202).set_body_json(""))
            .mount(&server)
            .await;
        assert!(client
            .submit_tx("8100")
            .await
            .unwrap_err()
            .contains("invalid transaction id"));
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("invalid transaction"))
            .mount(&server)
            .await;
        assert!(client.submit_tx("8100").await.is_err());
    }
}
