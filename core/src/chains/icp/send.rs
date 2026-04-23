//! ICP send: Rosetta construction flow (preprocess → metadata → payloads →
//! sign → combine → submit).

use serde_json::{json, Value};

use super::{IcpClient, IcpSendResult};

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
            .cloned()
            .unwrap_or_default();

        // Step 4: Sign each payload.
        let mut signatures = Vec::new();
        for payload_item in &to_sign_payloads {
            let hex_bytes = payload_item
                .get("hex_bytes")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let hash_bytes = hex::decode(hex_bytes).unwrap_or_default();
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

        // Step 6: /construction/submit
        let submit: Value = self
            .rosetta_post(
                "/construction/submit",
                &json!({
                    "network_identifier": network,
                    "signed_transaction": signed_tx
                }),
            )
            .await?;
        let block_index: u64 = submit
            .pointer("/transaction_identifier/hash")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        Ok(IcpSendResult { block_index })
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
    let msg_hash = if hash_bytes.len() == 32 {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(hash_bytes);
        arr
    } else {
        use sha2::{Digest, Sha256};
        Sha256::digest(hash_bytes).into()
    };
    let msg = Message::from_digest_slice(&msg_hash).map_err(|e| format!("msg: {e}"))?;
    let sig = secp.sign_ecdsa(&msg, &secret_key);
    Ok(hex::encode(sig.serialize_compact()))
}
