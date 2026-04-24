//! Sui send: build, sign, execute (and rebroadcast of a pre-signed tx).

use serde_json::json;

use crate::fetch::chains::sui::{SuiClient, SuiSendResult};

impl SuiClient {
    /// Request an unsigned transfer transaction, sign it, and execute.
    pub async fn sign_and_send(
        &self,
        from_address: &str,
        to_address: &str,
        mist: u64,
        gas_budget: u64,
        private_key_bytes: &[u8; 64],
        public_key_bytes: &[u8; 32],
    ) -> Result<SuiSendResult, String> {
        // Build an unsafe transfer (node constructs the tx bytes).
        let tx_result = self
            .call(
                "unsafe_transferSui",
                json!([from_address, to_address, gas_budget.to_string(), mist.to_string()]),
            )
            .await?;

        let tx_bytes_b64 = tx_result
            .get("txBytes")
            .and_then(|v| v.as_str())
            .ok_or("unsafe_transferSui: missing txBytes")?;

        use base64::Engine;
        let tx_bytes = base64::engine::general_purpose::STANDARD
            .decode(tx_bytes_b64)
            .map_err(|e| format!("b64 decode: {e}"))?;

        // Signing: intent prefix [0,0,0] + tx_bytes.
        let mut signing_payload = vec![0u8, 0u8, 0u8];
        signing_payload.extend_from_slice(&tx_bytes);

        use ed25519_dalek::{Signer, SigningKey};
        use sha2::{Digest, Sha256};
        let digest: [u8; 32] = Sha256::digest(&signing_payload).into();
        // ed25519-dalek SigningKey::from_bytes takes the 32-byte seed (first half of the 64-byte keypair).
        let seed: [u8; 32] = private_key_bytes[..32].try_into().map_err(|_| "privkey too short")?;
        let signing_key = SigningKey::from_bytes(&seed);
        let signature = signing_key.sign(&digest);

        // Sui signature format: [flag(1)] + [sig(64)] + [pk(32)], base64.
        let mut sig_bytes = vec![0x00u8]; // Ed25519 flag
        sig_bytes.extend_from_slice(signature.to_bytes().as_ref());
        sig_bytes.extend_from_slice(public_key_bytes);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&sig_bytes);

        let execute_result = self
            .call(
                "sui_executeTransactionBlock",
                json!([tx_bytes_b64, [sig_b64], {"showEffects": true}, "WaitForLocalExecution"]),
            )
            .await?;

        let digest = execute_result
            .get("digest")
            .and_then(|v| v.as_str())
            .ok_or("executeTransactionBlock: missing digest")?
            .to_string();

        Ok(SuiSendResult { digest, tx_bytes_b64: tx_bytes_b64.to_string(), sig_b64 })
    }

    /// Execute a pre-signed transaction block (for rebroadcast).
    pub async fn execute_signed_tx(
        &self,
        tx_bytes_b64: &str,
        sig_b64: &str,
    ) -> Result<SuiSendResult, String> {
        let execute_result = self
            .call(
                "sui_executeTransactionBlock",
                json!([tx_bytes_b64, [sig_b64], {"showEffects": true}, "WaitForLocalExecution"]),
            )
            .await?;
        let digest = execute_result
            .get("digest")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(SuiSendResult {
            digest,
            tx_bytes_b64: tx_bytes_b64.to_string(),
            sig_b64: sig_b64.to_string(),
        })
    }
}
