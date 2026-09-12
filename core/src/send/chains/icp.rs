//! ICP send: Rosetta construction flow (preprocess → metadata → payloads →
//! sign → combine → submit).

use serde_json::{json, Value};

use crate::fetch::chains::icp::{IcpClient, IcpSendResult};

impl IcpClient {
    /// Submit an ICP transfer via the Rosetta construction API.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        e8s: u64,
        private_key_bytes: &[u8],
        public_key_bytes: &[u8],
    ) -> Result<IcpSendResult, String> {
        let key = secp256k1::SecretKey::from_slice(private_key_bytes)
            .map_err(|e| format!("invalid key: {e}"))?;
        let derived = secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), &key);
        let supplied = secp256k1::PublicKey::from_slice(public_key_bytes)
            .map_err(|e| format!("invalid public key: {e}"))?;
        if supplied != derived {
            return Err("public key does not match signing key".into());
        }
        let public_key_bytes = derived.serialize();
        let network = json!({
            "blockchain": "Internet Computer",
            "network": "00000000000000020101"
        });

        // Step 1: /construction/preprocess
        let ops = build_transfer_ops(from_address, to_address, e8s);
        let preprocess: Value = self
            .rosetta_post(
                "/construction/preprocess",
                &json!({"network_identifier": network, "operations": ops}),
            )
            .await?;

        // Step 2: /construction/metadata
        let metadata: Value = self
            .rosetta_post(
                "/construction/metadata",
                &json!({
                    "network_identifier": network,
                    "options": preprocess.get("options").cloned().unwrap_or(json!({}))
                }),
            )
            .await?;

        // Step 3: /construction/payloads
        let payloads: Value = self
            .rosetta_post(
                "/construction/payloads",
                &json!({
                    "network_identifier": network,
                    "operations": ops,
                    "metadata": metadata.get("metadata").cloned().unwrap_or(json!({})),
                    "public_keys": [{
                        "hex_bytes": hex::encode(public_key_bytes),
                        "curve_type": "secp256k1"
                    }]
                }),
            )
            .await?;

        let unsigned_tx = payloads
            .get("unsigned_transaction")
            .and_then(|v| v.as_str())
            .ok_or("payloads: missing unsigned_transaction")?;
        let to_sign_payloads = payloads
            .get("payloads")
            .and_then(|v| v.as_array())
            .ok_or("payloads: missing payload array")?;
        if to_sign_payloads.is_empty() {
            return Err("payloads: empty payload array".into());
        }

        // Step 4: Sign each payload.
        let mut signatures = Vec::new();
        for payload_item in to_sign_payloads {
            let hex_bytes = payload_item
                .get("hex_bytes")
                .and_then(|v| v.as_str())
                .ok_or("payloads: missing hex_bytes")?;
            if payload_item.get("signature_type").and_then(Value::as_str) != Some("ecdsa") {
                return Err("payloads: expected ecdsa signature type".into());
            }
            let hash_bytes =
                hex::decode(hex_bytes).map_err(|e| format!("payloads: invalid hex: {e}"))?;
            let sig_hex = sign_icp_payload(&hash_bytes, private_key_bytes)?;
            signatures.push(json!({
                "signing_payload": payload_item,
                "public_key": {
                    "hex_bytes": hex::encode(public_key_bytes),
                    "curve_type": "secp256k1"
                },
                "signature_type": "ecdsa",
                "hex_bytes": sig_hex
            }));
        }

        // Step 5: /construction/combine
        let combined: Value = self
            .rosetta_post(
                "/construction/combine",
                &json!({
                    "network_identifier": network,
                    "unsigned_transaction": unsigned_tx,
                    "signatures": signatures
                }),
            )
            .await?;
        let signed_tx = combined
            .get("signed_transaction")
            .and_then(|v| v.as_str())
            .ok_or("combine: missing signed_transaction")?;

        let payload = json!({"network_identifier": network, "signed_transaction": signed_tx});
        crate::send::payload::before_submission(payload.to_string(), "txid", None, None).await?;
        self.submit_signed_transaction(&payload.to_string()).await
    }

    pub(crate) async fn submit_signed_transaction(
        &self,
        payload: &str,
    ) -> Result<IcpSendResult, String> {
        let body: Value = serde_json::from_str(payload).map_err(|e| e.to_string())?;
        let submit: Value = self.rosetta_post("/construction/submit", &body).await?;
        let txid = submit
            .pointer("/transaction_identifier/hash")
            .and_then(Value::as_str)
            .ok_or("submit: missing transaction hash")?;
        if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("submit: invalid transaction hash".into());
        }
        Ok(IcpSendResult {
            txid: txid.to_lowercase(),
        })
    }
}

fn build_transfer_ops(from: &str, to: &str, e8s: u64) -> Value {
    json!([
        {
            "operation_identifier": {"index": 0},
            "type": "TRANSACTION",
            "account": {"address": from},
            "amount": {"value": format!("-{}", e8s), "currency": {"symbol": "ICP", "decimals": 8}}
        },
        {
            "operation_identifier": {"index": 1},
            "type": "TRANSACTION",
            "account": {"address": to},
            "amount": {"value": e8s.to_string(), "currency": {"symbol": "ICP", "decimals": 8}}
        },
        {
            "operation_identifier": {"index": 2},
            "type": "FEE",
            "account": {"address": from},
            "amount": {"value": "-10000", "currency": {"symbol": "ICP", "decimals": 8}}
        }
    ])
}

fn sign_icp_payload(hash_bytes: &[u8], private_key_bytes: &[u8]) -> Result<String, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    // ICP signs the domain-separated request id (11-byte prefix + 32-byte id).
    // https://docs.internetcomputer.org/references/ic-interface-spec/https-interface/
    if hash_bytes.len() != 43 || !hash_bytes.starts_with(b"\x0aic-request") {
        return Err("payloads: expected an IC request signing preimage".into());
    }
    use sha2::{Digest, Sha256};
    let msg_hash: [u8; 32] = Sha256::digest(hash_bytes).into();
    let msg = Message::from_digest_slice(&msg_hash).map_err(|e| format!("msg: {e}"))?;
    let sig = secp.sign_ecdsa(&msg, &secret_key);
    Ok(hex::encode(sig.serialize_compact()))
}

#[cfg(test)]
mod strict_payload_tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

    #[test]
    fn icp_signing_requires_a_domain_separated_request() {
        for bytes in [vec![], vec![0; 32], vec![0; 43], vec![0; 44]] {
            assert!(sign_icp_payload(&bytes, &[1; 32]).is_err());
        }
        let mut bytes = b"\x0aic-request".to_vec();
        bytes.extend_from_slice(&[7; 32]);
        let sig = sign_icp_payload(&bytes, &[1; 32]).unwrap();
        use sha2::{Digest, Sha256};
        let key = secp256k1::SecretKey::from_slice(&[1; 32]).unwrap();
        let secp = secp256k1::Secp256k1::new();
        let signature =
            secp256k1::ecdsa::Signature::from_compact(&hex::decode(sig).unwrap()).unwrap();
        let message = secp256k1::Message::from_digest_slice(&Sha256::digest(&bytes)).unwrap();
        secp.verify_ecdsa(
            &message,
            &signature,
            &secp256k1::PublicKey::from_secret_key(&secp, &key),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn icp_rejects_malformed_payloads_before_combine_and_preserves_transaction_hash() {
        for fault in [
            "ok",
            "missing",
            "empty",
            "hex",
            "prefix",
            "type",
            "missing_hash",
            "bad_hash",
        ] {
            let server = MockServer::start().await;
            Mock::given(any()).respond_with(move |request: &Request| {
                let body: Value = request.body_json().unwrap();
                let response = match request.url.path() {
                    "/construction/preprocess" => json!({"options":{}}),
                    "/construction/metadata" => json!({"metadata":{}}),
                    "/construction/payloads" => {
                        let mut bytes = b"\x0aic-request".to_vec(); bytes.extend_from_slice(&[7;32]);
                        let mut result = json!({"unsigned_transaction":"unsigned","payloads":[{"hex_bytes":hex::encode(bytes),"signature_type":"ecdsa"}]});
                        match fault {
                            "missing" => { result.as_object_mut().unwrap().remove("payloads"); },
                            "empty" => result["payloads"] = json!([]),
                            "hex" => result["payloads"][0]["hex_bytes"] = json!("zz"),
                            "prefix" => result["payloads"][0]["hex_bytes"] = json!("00".repeat(43)),
                            "type" => result["payloads"][0]["signature_type"] = json!("ed25519"),
                            _ => {}
                        } result
                    },
                    "/construction/combine" => {
                        assert_eq!(body["signatures"].as_array().unwrap().len(),1);
                        json!({"signed_transaction":"signed"})
                    },
                    "/construction/submit" => match fault {
                        "missing_hash" => json!({}),
                        "bad_hash" => json!({"transaction_identifier":{"hash":"0"}}),
                        _ => json!({"transaction_identifier":{"hash":"ab".repeat(32)}})
                    },
                    p => panic!("unexpected request {p}"),
                };
                ResponseTemplate::new(200).set_body_json(response)
            }).mount(&server).await;
            let key = secp256k1::SecretKey::from_slice(&[1; 32]).unwrap();
            let public = secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), &key)
                .serialize();
            let result = IcpClient::new(Arc::new(vec![server.uri()]))
                .sign_and_submit("from", "to", 1, &[1; 32], &public)
                .await;
            if fault == "ok" {
                assert_eq!(result.unwrap().txid, "ab".repeat(32));
            } else {
                assert!(result.is_err(), "{fault}");
            }
            if ["missing", "empty", "hex", "prefix", "type"].contains(&fault) {
                assert!(server.received_requests().await.unwrap().iter().all(|r| ![
                    "/construction/combine",
                    "/construction/submit"
                ]
                .contains(&r.url.path())));
            }
        }
    }
}
