//! Aptos send: sign and submit APT transfers via the REST API encoder.

use serde_json::{json, Value};

use crate::fetch::chains::aptos::{AptosClient, AptosSendResult};

impl AptosClient {
    /// Sign and submit an APT coin transfer.
    pub async fn sign_and_submit(
        &self,
        from_address: &str,
        to_address: &str,
        octas: u64,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<AptosSendResult, String> {
        let (sequence_number, _) = self.fetch_account_info(from_address).await?;
        let (_chain_id, _) = self.fetch_ledger_info().await?;
        let gas_unit_price = self.fetch_gas_price().await?;

        // Use the REST API to encode the transaction (simpler than full BCS).
        let payload = json!({
            "type": "entry_function_payload",
            "function": "0x1::coin::transfer",
            "type_arguments": ["0x1::aptos_coin::AptosCoin"],
            "arguments": [to_address, octas.to_string()]
        });

        let raw_tx_body = json!({
            "sender": from_address,
            "sequence_number": sequence_number.to_string(),
            "max_gas_amount": "10000",
            "gas_unit_price": gas_unit_price.to_string(),
            "expiration_timestamp_secs": (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() + 600).to_string(),
            "payload": payload,
        });

        // Encode signing message via /transactions/encode_submission.
        let encode_resp: Value = self
            .post_val("/transactions/encode_submission", &raw_tx_body)
            .await?;
        let signing_msg_hex = encode_resp
            .as_str()
            .ok_or("encode_submission: expected string")?;
        let signing_bytes =
            hex::decode(signing_msg_hex.strip_prefix("0x").unwrap_or(signing_msg_hex))
                .map_err(|e| format!("hex: {e}"))?;

        use ed25519_dalek::{Signer, SigningKey};
        let seed: [u8; 32] = private_key_bytes[..32].try_into().map_err(|_| "privkey too short")?;
        let signing_key = SigningKey::from_bytes(&seed);
        let signature = signing_key.sign(&signing_bytes);

        let mut submit_body = raw_tx_body.clone();
        submit_body["signature"] = json!({
            "type": "ed25519_signature",
            "public_key": format!("0x{}", hex::encode(public_key_bytes)),
            "signature": format!("0x{}", hex::encode(signature.to_bytes()))
        });

        let signed_body_json = submit_body.to_string();
        let submit_resp: Value = self.post_val("/transactions", &submit_body).await?;
        let txid = submit_resp
            .get("hash")
            .and_then(|v| v.as_str())
            .ok_or("submit: missing hash")?
            .to_string();
        let version = submit_resp
            .get("version")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());

        Ok(AptosSendResult { txid, version, signed_body_json })
    }

    /// Submit a pre-signed transaction body JSON (for rebroadcast).
    pub async fn submit_signed_body(&self, signed_json: &str) -> Result<AptosSendResult, String> {
        let body: Value = serde_json::from_str(signed_json)
            .map_err(|e| format!("json parse: {e}"))?;
        let submit_resp: Value = self.post_val("/transactions", &body).await?;
        let txid = submit_resp
            .get("hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let version = submit_resp
            .get("version")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());
        Ok(AptosSendResult { txid, version, signed_body_json: signed_json.to_string() })
    }
}
