//! Tron send: create + sign + broadcast native TRX transfers and TRC-20
//! `transfer(address,uint256)` calls; plus raw rebroadcast of a signed JSON tx.

use serde_json::json;

use super::derive::tron_base58_to_evm_hex;
use super::{TronClient, TronSendResult};

impl TronClient {
    /// Create, sign, and broadcast a TRX transfer.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sun: u64,
        private_key_bytes: &[u8],
    ) -> Result<TronSendResult, String> {
        // Step 1: Create unsigned transaction via /wallet/createtransaction.
        let resp = self
            .post(
                "/wallet/createtransaction",
                &json!({
                    "owner_address": from_address,
                    "to_address": to_address,
                    "amount": amount_sun,
                    "visible": true
                }),
            )
            .await?;

        // Extract the raw_data_hex for signing.
        let _raw_data_hex = resp
            .get("raw_data_hex")
            .and_then(|v| v.as_str())
            .ok_or("createtransaction: missing raw_data_hex")?;
        let txid = resp
            .get("txID")
            .and_then(|v| v.as_str())
            .ok_or("createtransaction: missing txID")?
            .to_string();

        // Step 2: Sign txID (which is the sha256 of raw_data).
        let txid_bytes = hex::decode(&txid).map_err(|e| format!("txid hex: {e}"))?;
        let signature = sign_tron_hash(&txid_bytes, private_key_bytes)?;

        // Step 3: Broadcast.
        let mut broadcast_body = resp.clone();
        broadcast_body["signature"] = json!([signature]);
        let signed_tx_json = broadcast_body.to_string();
        let broadcast_resp = self
            .post("/wallet/broadcasttransaction", &broadcast_body)
            .await?;
        let result = broadcast_resp
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !result {
            let msg = broadcast_resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(format!("broadcast failed: {msg}"));
        }
        Ok(TronSendResult { txid, signed_tx_json })
    }

    /// Build, sign, and broadcast a TRC-20 `transfer(to, amount)` via
    /// `triggersmartcontract` → `broadcasttransaction`.
    pub async fn sign_and_broadcast_trc20(
        &self,
        from_base58: &str,
        contract_base58: &str,
        to_base58: &str,
        amount_raw: u128,
        fee_limit_sun: u64,
        private_key_bytes: &[u8],
    ) -> Result<TronSendResult, String> {
        let to_hex = tron_base58_to_evm_hex(to_base58)?;
        let to_padded = format!("{:0>64}", to_hex);
        let amount_padded = format!("{:064x}", amount_raw);
        let parameter = format!("{}{}", to_padded, amount_padded);

        let resp = self
            .post(
                "/wallet/triggersmartcontract",
                &json!({
                    "owner_address": from_base58,
                    "contract_address": contract_base58,
                    "function_selector": "transfer(address,uint256)",
                    "parameter": parameter,
                    "fee_limit": fee_limit_sun,
                    "call_value": 0,
                    "visible": true
                }),
            )
            .await?;

        let tx_obj = resp
            .get("transaction")
            .ok_or("triggersmartcontract: missing transaction")?;
        let txid = tx_obj
            .get("txID")
            .and_then(|v| v.as_str())
            .ok_or("triggersmartcontract: missing txID")?
            .to_string();

        // Check for contract execution errors at the trigger step.
        if let Some(result_obj) = resp.get("result") {
            if !result_obj.get("result").and_then(|v| v.as_bool()).unwrap_or(false) {
                if let Some(msg) = result_obj.get("message").and_then(|v| v.as_str()) {
                    let decoded = hex::decode(msg)
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                        .unwrap_or_else(|| msg.to_string());
                    return Err(format!("trc20 trigger failed: {decoded}"));
                }
            }
        }

        // Sign txID.
        let txid_bytes = hex::decode(&txid).map_err(|e| format!("txid hex: {e}"))?;
        let signature = sign_tron_hash(&txid_bytes, private_key_bytes)?;

        // Attach signature and broadcast.
        let mut broadcast_body = tx_obj.clone();
        broadcast_body["signature"] = json!([signature]);
        let signed_tx_json = broadcast_body.to_string();
        let broadcast_resp = self
            .post("/wallet/broadcasttransaction", &broadcast_body)
            .await?;
        let ok = broadcast_resp
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !ok {
            let msg = broadcast_resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let decoded = hex::decode(msg)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| msg.to_string());
            return Err(format!("trc20 broadcast failed: {decoded}"));
        }
        Ok(TronSendResult { txid, signed_tx_json })
    }

    /// Broadcast an already-signed transaction given as a JSON string.
    pub async fn broadcast_raw(&self, signed_tx_json: &str) -> Result<TronSendResult, String> {
        let body: serde_json::Value = serde_json::from_str(signed_tx_json)
            .map_err(|e| format!("broadcast_raw: invalid JSON: {e}"))?;
        let txid = body
            .get("txID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let broadcast_resp = self.post("/wallet/broadcasttransaction", &body).await?;
        let ok = broadcast_resp
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !ok {
            let msg = broadcast_resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let decoded = hex::decode(msg)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| msg.to_string());
            return Err(format!("broadcast failed: {decoded}"));
        }
        Ok(TronSendResult { txid, signed_tx_json: signed_tx_json.to_string() })
    }
}

fn sign_tron_hash(hash: &[u8], private_key_bytes: &[u8]) -> Result<String, String> {
    use secp256k1::{Message, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let msg = Message::from_digest_slice(hash).map_err(|e| format!("msg: {e}"))?;
    let (rec_id, sig) = secp
        .sign_ecdsa_recoverable(&msg, &secret_key)
        .serialize_compact();
    let mut out = sig.to_vec();
    out.push(rec_id.to_i32() as u8);
    Ok(hex::encode(&out))
}
