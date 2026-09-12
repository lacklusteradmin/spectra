//! APT: read sequence/gas, construct the BCS signing message locally, sign, submit.
use super::bcs;
use crate::fetch::chains::aptos::{AptosClient, AptosSendResult};
use crate::send::keys::Ed25519Seed;
use serde_json::{json, Value};
use sha3::{Digest, Sha3_256};

pub(crate) struct PreparedAptosTransfer {
    sender: [u8; 32],
    message: Vec<u8>,
    body: Value,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_transfer(
    from: &str,
    to: &str,
    amount: u64,
    sequence: u64,
    gas_price: u64,
    max_gas: u64,
    expiration: u64,
    chain_id: u8,
) -> Result<PreparedAptosTransfer, String> {
    let sender = bcs::address(from)?;
    let recipient = bcs::address(to)?;
    if amount == 0 || gas_price == 0 || max_gas == 0 || expiration == 0 {
        return Err("invalid Aptos amount, gas or expiration".into());
    }
    let mut message = Sha3_256::digest(b"APTOS::RawTransaction").to_vec();
    message.extend_from_slice(&sender);
    message.extend_from_slice(&sequence.to_le_bytes());
    message.push(2); // TransactionPayload::EntryFunction
    let one = bcs::address("0x1")?;
    message.extend_from_slice(&one);
    bcs::bytes(b"coin", &mut message);
    bcs::bytes(b"transfer", &mut message);
    message.push(1); // one type argument
    message.push(7); // TypeTag::Struct
    message.extend_from_slice(&one);
    bcs::bytes(b"aptos_coin", &mut message);
    bcs::bytes(b"AptosCoin", &mut message);
    message.push(0);
    message.push(2); // arguments: recipient and octas
    bcs::bytes(&recipient, &mut message);
    bcs::bytes(&amount.to_le_bytes(), &mut message);
    message.extend_from_slice(&max_gas.to_le_bytes());
    message.extend_from_slice(&gas_price.to_le_bytes());
    message.extend_from_slice(&expiration.to_le_bytes());
    message.push(chain_id);
    let body = json!({"sender":format!("0x{}",hex::encode(sender)),"sequence_number":sequence.to_string(),"max_gas_amount":max_gas.to_string(),"gas_unit_price":gas_price.to_string(),"expiration_timestamp_secs":expiration.to_string(),"payload":{"type":"entry_function_payload","function":"0x1::coin::transfer","type_arguments":["0x1::aptos_coin::AptosCoin"],"arguments":[format!("0x{}",hex::encode(recipient)),amount.to_string()]}});
    Ok(PreparedAptosTransfer {
        sender,
        message,
        body,
    })
}
impl PreparedAptosTransfer {
    pub(crate) fn sign(mut self, key: &Ed25519Seed) -> Result<String, String> {
        let public = key.public_key();
        let address: [u8; 32] = Sha3_256::new()
            .chain_update(public)
            .chain_update([0])
            .finalize()
            .into();
        if self.sender != address {
            return Err("Aptos sender does not match signing seed".into());
        }
        self.body["signature"] = json!({"type":"ed25519_signature","public_key":format!("0x{}",hex::encode(public)),"signature":format!("0x{}",hex::encode(key.sign(&self.message)))});
        Ok(self.body.to_string())
    }
}
impl AptosClient {
    pub async fn sign_and_submit(
        &self,
        from: &str,
        to: &str,
        octas: u64,
        key: &Ed25519Seed,
        public: &[u8; 32],
        expected_chain_id: u8,
    ) -> Result<AptosSendResult, String> {
        key.require_public_key(public)?;
        bcs::address(from)?;
        bcs::address(to)?;
        if octas == 0 {
            return Err("Aptos amount must be positive".into());
        }
        let (sequence, _) = self.fetch_account_info(from).await?;
        let (chain_id, _) = self.fetch_ledger_info().await?;
        if chain_id != u64::from(expected_chain_id) {
            return Err("Aptos endpoint network does not match the requested chain".into());
        }
        let gas_price = self.fetch_gas_price().await?;
        let expiration = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "clock before epoch")?
            .as_secs()
            .checked_add(600)
            .ok_or("expiration overflow")?;
        let prepared = prepare_transfer(
            from,
            to,
            octas,
            sequence,
            gas_price,
            10_000,
            expiration,
            expected_chain_id,
        )?;
        self.submit_signed_body(&prepared.sign(key)?).await
    }
    pub async fn submit_signed_body(&self, signed_json: &str) -> Result<AptosSendResult, String> {
        let body: Value = serde_json::from_str(signed_json)
            .map_err(|e| format!("invalid Aptos transaction: {e}"))?;
        crate::send::payload::before_submission(
            serde_json::json!({"signed_body_json":signed_json}).to_string(),
            "txid",
            None,
            None,
        )
        .await?;
        let response = self.post_val("/transactions", &body).await?;
        let txid = response["hash"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("Aptos submit: missing hash")?
            .to_string();
        let version = response["version"].as_str().and_then(|s| s.parse().ok());
        Ok(AptosSendResult {
            txid,
            version,
            signed_body_json: signed_json.into(),
        })
    }
}
