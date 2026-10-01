//! APT: construct the BCS signing message locally and sign it. Reads and
//! submission are `api::aptos_rest`.

use super::bcs;
use crate::send::error::SendError;
use crate::send::keys::Ed25519Seed;
use serde_json::{Value, json};
use sha3::{Digest, Sha3_256};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedAptosTransfer {
    sender: [u8; 32],
    pub(crate) message: Vec<u8>,
    pub(crate) body: Value,
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
) -> Result<PreparedAptosTransfer, SendError> {
    let sender = bcs::address(from)?;
    let recipient = bcs::address(to)?;
    if amount == 0 || gas_price == 0 || max_gas == 0 || expiration == 0 {
        return Err(SendError::Invalid(
            "invalid Aptos amount, gas or expiration".into(),
        ));
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
    pub(crate) fn sign(mut self, key: &Ed25519Seed) -> Result<String, SendError> {
        let public = key.public_key();
        let address: [u8; 32] = Sha3_256::new()
            .chain_update(public)
            .chain_update([0])
            .finalize()
            .into();
        if self.sender != address {
            return Err(SendError::Invalid(
                "Aptos sender does not match signing seed".into(),
            ));
        }
        self.body["signature"] = json!({"type":"ed25519_signature","public_key":format!("0x{}",hex::encode(public)),"signature":format!("0x{}",hex::encode(key.sign(&self.message)))});
        Ok(self.body.to_string())
    }
}
