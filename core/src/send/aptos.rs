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
    pub(crate) staking_public_key: Option<[u8; 32]>,
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
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
    prepare_token_transfer(
        from,
        to,
        amount,
        sequence,
        gas_price,
        max_gas,
        expiration,
        chain_id,
        "0x1::aptos_coin::AptosCoin",
    )
}

/// Coin types call `coin::transfer<T>`; fungible-asset metadata objects call
/// `primary_fungible_store::transfer<Metadata>`. Both payloads are encoded
/// locally so the signed BCS commits to the same asset as the reviewed JSON.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_token_transfer(
    from: &str,
    to: &str,
    amount: u64,
    sequence: u64,
    gas_price: u64,
    max_gas: u64,
    expiration: u64,
    chain_id: u8,
    identifier: &str,
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
    let (module, coin_type, arguments) = if identifier.contains("::") {
        (
            "coin",
            identifier,
            vec![recipient.to_vec(), amount.to_le_bytes().to_vec()],
        )
    } else {
        (
            "primary_fungible_store",
            "0x1::fungible_asset::Metadata",
            vec![
                bcs::address(identifier)?.to_vec(),
                recipient.to_vec(),
                amount.to_le_bytes().to_vec(),
            ],
        )
    };
    let parts: Vec<_> = coin_type.split("::").collect();
    let valid_name = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    if parts.len() != 3 || !valid_name(parts[1]) || !valid_name(parts[2]) {
        return Err(SendError::invalid("Invalid Aptos token type"));
    }
    bcs::bytes(module.as_bytes(), &mut message);
    bcs::bytes(b"transfer", &mut message);
    message.push(1); // one type argument
    message.push(7); // TypeTag::Struct
    message.extend_from_slice(&bcs::address(parts[0])?);
    bcs::bytes(parts[1].as_bytes(), &mut message);
    bcs::bytes(parts[2].as_bytes(), &mut message);
    message.push(0);
    bcs::uleb(arguments.len(), &mut message);
    for argument in &arguments {
        bcs::bytes(argument, &mut message);
    }
    message.extend_from_slice(&max_gas.to_le_bytes());
    message.extend_from_slice(&gas_price.to_le_bytes());
    message.extend_from_slice(&expiration.to_le_bytes());
    message.push(chain_id);
    let json_arguments = if identifier.contains("::") {
        vec![format!("0x{}", hex::encode(recipient)), amount.to_string()]
    } else {
        vec![
            format!("0x{}", hex::encode(bcs::address(identifier)?)),
            format!("0x{}", hex::encode(recipient)),
            amount.to_string(),
        ]
    };
    let body = json!({"sender":format!("0x{}",hex::encode(sender)),"sequence_number":sequence.to_string(),"max_gas_amount":max_gas.to_string(),"gas_unit_price":gas_price.to_string(),"expiration_timestamp_secs":expiration.to_string(),"payload":{"type":"entry_function_payload","function":format!("0x1::{module}::transfer"),"type_arguments":[coin_type],"arguments":json_arguments}});
    Ok(PreparedAptosTransfer {
        sender,
        message,
        body,
        staking_public_key: None,
    })
}
impl PreparedAptosTransfer {
    /// RawTransaction's chain id is its final BCS byte; the submission JSON
    /// intentionally omits it as required by the REST transaction schema.
    pub(crate) fn chain_id(&self) -> Option<u8> {
        self.message.last().copied()
    }

    pub(crate) fn sign(mut self, key: &Ed25519Seed) -> Result<(String, String), SendError> {
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
        let signature = key.sign(&self.message);
        let raw = self
            .message
            .get(32..)
            .filter(|_| self.message[..32] == Sha3_256::digest(b"APTOS::RawTransaction")[..])
            .ok_or_else(|| SendError::invalid("Invalid Aptos signing message"))?;
        // Transaction::UserTransaction(0), followed by SignedTransaction BCS:
        // RawTransaction, Ed25519 authenticator(0), byte-vector public/signature.
        let mut transaction = vec![0];
        transaction.extend_from_slice(raw);
        transaction.push(0);
        bcs::bytes(&public, &mut transaction);
        bcs::bytes(&signature, &mut transaction);
        let hash = Sha3_256::new()
            .chain_update(Sha3_256::digest(b"APTOS::Transaction"))
            .chain_update(transaction)
            .finalize();
        self.body["signature"] = json!({"type":"ed25519_signature","public_key":format!("0x{}",hex::encode(public)),"signature":format!("0x{}",hex::encode(signature))});
        Ok((self.body.to_string(), format!("0x{}", hex::encode(hash))))
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_delegation(
    from: &str,
    pool: &str,
    amount: u64,
    sequence: u64,
    gas_price: u64,
    max_gas: u64,
    expiration: u64,
    chain_id: u8,
    action: crate::staking::StakingAction,
) -> Result<PreparedAptosTransfer, SendError> {
    let function = match action {
        crate::staking::StakingAction::Stake => "add_stake",
        crate::staking::StakingAction::Unstake => "unlock",
        crate::staking::StakingAction::Withdraw => "withdraw",
        _ => {
            return Err(SendError::invalid(
                "Aptos delegation rewards compound into stake",
            ));
        }
    };
    if amount == 0 || gas_price == 0 || max_gas == 0 || expiration == 0 {
        return Err(SendError::invalid(
            "Invalid Aptos delegation amount, gas or expiration",
        ));
    }
    let sender = bcs::address(from)?;
    let pool = bcs::address(pool)?;
    let mut message = Sha3_256::digest(b"APTOS::RawTransaction").to_vec();
    message.extend_from_slice(&sender);
    message.extend_from_slice(&sequence.to_le_bytes());
    message.push(2);
    message.extend_from_slice(&bcs::address("0x1")?);
    bcs::bytes(b"delegation_pool", &mut message);
    bcs::bytes(function.as_bytes(), &mut message);
    message.push(0); // no type arguments
    message.push(2);
    bcs::bytes(&pool, &mut message);
    bcs::bytes(&amount.to_le_bytes(), &mut message);
    message.extend_from_slice(&max_gas.to_le_bytes());
    message.extend_from_slice(&gas_price.to_le_bytes());
    message.extend_from_slice(&expiration.to_le_bytes());
    message.push(chain_id);
    let body = json!({"sender":format!("0x{}",hex::encode(sender)),"sequence_number":sequence.to_string(),"max_gas_amount":max_gas.to_string(),"gas_unit_price":gas_price.to_string(),"expiration_timestamp_secs":expiration.to_string(),"payload":{"type":"entry_function_payload","function":format!("0x1::delegation_pool::{function}"),"type_arguments":[],"arguments":[format!("0x{}",hex::encode(pool)),amount.to_string()]}});
    Ok(PreparedAptosTransfer {
        sender,
        message,
        body,
        staking_public_key: None,
    })
}

#[cfg(test)]
mod token_tests {
    use super::*;
    #[test]
    fn token_transfers_match_official_sdk_signing_vectors() {
        let fixtures: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/token-send-vectors.json"))
                .unwrap();
        let key = Ed25519Seed::from_hex(&hex::encode([1u8; 32])).unwrap();
        for vector in fixtures["aptos"].as_array().unwrap() {
            let prepared = prepare_token_transfer(
                vector["sender"].as_str().unwrap(),
                &format!("0x{}", "22".repeat(32)),
                123456789,
                7,
                100,
                10000,
                1800000000,
                1,
                vector["asset"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(
                hex::encode(&prepared.message),
                vector["message"].as_str().unwrap()
            );
            let (body, hash) = prepared.sign(&key).unwrap();
            assert_eq!(hash, vector["transaction_hash"].as_str().unwrap());
            let signed: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(
                signed["signature"]["signature"].as_str().unwrap(),
                format!("0x{}", vector["signature"].as_str().unwrap())
            );
        }
    }
}
