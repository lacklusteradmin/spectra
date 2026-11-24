//! APT: construct the BCS signing message locally and sign it. Reads and
//! submission are `api::aptos_rest`.

use super::bcs;
use crate::send::error::SendError;
use crate::send::keys::Ed25519Seed;
use sha3::{Digest, Sha3_256};

/// A transaction built and ready to sign: its signing message (the
/// RawTransaction prefix hash and its BCS) and the terms that message holds
/// which the send stages check again before signing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedAptosTransfer {
    sender: [u8; 32],
    pub(crate) message: Vec<u8>,
    pub(crate) sequence: u64,
    pub(crate) max_gas: u64,
    pub(crate) gas_price: u64,
    pub(crate) expiration: u64,
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
/// `primary_fungible_store::transfer<Metadata>`.
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
    Ok(PreparedAptosTransfer {
        sender,
        message,
        sequence,
        max_gas,
        gas_price,
        expiration,
        staking_public_key: None,
    })
}
impl PreparedAptosTransfer {
    /// RawTransaction's chain id is its final BCS byte.
    pub(crate) fn chain_id(&self) -> Option<u8> {
        self.message.last().copied()
    }

    /// The SignedTransaction BCS: the RawTransaction the message signs, then
    /// an Ed25519 authenticator (0) with its public key and signature.
    fn signed_transaction(&self, public: &[u8], signature: &[u8]) -> Result<Vec<u8>, SendError> {
        let raw = self
            .message
            .get(32..)
            .filter(|_| self.message[..32] == Sha3_256::digest(b"APTOS::RawTransaction")[..])
            .ok_or_else(|| SendError::invalid("Invalid Aptos signing message"))?;
        let mut signed = raw.to_vec();
        signed.push(0);
        bcs::bytes(public, &mut signed);
        bcs::bytes(signature, &mut signed);
        Ok(signed)
    }

    /// The transaction as a simulation takes it: signed by `public` with an
    /// all-zero signature, which a node executes without checking.
    pub(crate) fn simulation(&self, public: &[u8; 32]) -> Result<Vec<u8>, SendError> {
        self.signed_transaction(public, &[0; 64])
    }

    /// The SignedTransaction BCS a node takes, and the hash it lands under:
    /// SHA3 of the `APTOS::Transaction` prefix, `Transaction::UserTransaction`
    /// (0) and the signed BCS.
    pub(crate) fn sign(self, key: &Ed25519Seed) -> Result<(Vec<u8>, String), SendError> {
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
        let signed = self.signed_transaction(&public, &key.sign(&self.message))?;
        let hash = Sha3_256::new()
            .chain_update(Sha3_256::digest(b"APTOS::Transaction"))
            .chain_update([0])
            .chain_update(&signed)
            .finalize();
        Ok((signed, format!("0x{}", hex::encode(hash))))
    }
}

/// The `delegation_pool` entry function a staking action calls.
pub(crate) fn delegation_function(
    action: crate::staking::StakingAction,
) -> Result<&'static str, SendError> {
    match action {
        crate::staking::StakingAction::Stake => Ok("add_stake"),
        crate::staking::StakingAction::Unstake => Ok("unlock"),
        crate::staking::StakingAction::Withdraw => Ok("withdraw"),
        _ => Err(SendError::invalid(
            "Aptos delegation rewards compound into stake",
        )),
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
    let function = delegation_function(action)?;
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
    Ok(PreparedAptosTransfer {
        sender,
        message,
        sequence,
        max_gas,
        gas_price,
        expiration,
        staking_public_key: None,
    })
}

/// A RawTransaction or SignedTransaction this module encodes, read back for a
/// test's loopback node: an entry-function call whose type arguments are
/// structs without type parameters.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct DecodedAptosTransaction {
    pub sender: [u8; 32],
    pub function: String,
    pub arguments: Vec<Vec<u8>>,
    pub max_gas: u64,
    pub gas_price: u64,
    /// The Ed25519 public key and signature of a signed transaction.
    pub authenticator: Option<([u8; 32], Vec<u8>)>,
}

#[cfg(test)]
pub(crate) fn decode(bytes: &[u8]) -> DecodedAptosTransaction {
    struct Reader<'a>(&'a [u8]);
    impl<'a> Reader<'a> {
        fn take(&mut self, n: usize) -> &'a [u8] {
            let (out, rest) = self.0.split_at(n);
            self.0 = rest;
            out
        }
        fn byte(&mut self) -> u8 {
            self.take(1)[0]
        }
        fn u64(&mut self) -> u64 {
            u64::from_le_bytes(self.take(8).try_into().unwrap())
        }
        fn uleb(&mut self) -> usize {
            let (mut value, mut shift) = (0usize, 0);
            loop {
                let byte = self.byte();
                value |= usize::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    return value;
                }
                shift += 7;
            }
        }
        fn vector(&mut self) -> &'a [u8] {
            let len = self.uleb();
            self.take(len)
        }
        fn text(&mut self) -> String {
            String::from_utf8(self.vector().to_vec()).unwrap()
        }
    }
    let mut r = Reader(bytes);
    let sender: [u8; 32] = r.take(32).try_into().unwrap();
    r.u64(); // sequence
    assert_eq!(r.byte(), 2, "an entry function");
    let address = hex::encode(r.take(32));
    let address = address.trim_start_matches('0');
    let function = format!("0x{address}::{}::{}", r.text(), r.text());
    for _ in 0..r.uleb() {
        assert_eq!(r.byte(), 7, "a struct type argument");
        r.take(32);
        r.vector();
        r.vector();
        assert_eq!(r.uleb(), 0, "no type parameters");
    }
    let arguments = (0..r.uleb()).map(|_| r.vector().to_vec()).collect();
    let max_gas = r.u64();
    let gas_price = r.u64();
    r.take(9); // expiration, chain id
    let authenticator = (!r.0.is_empty()).then(|| {
        assert_eq!(r.byte(), 0, "an Ed25519 authenticator");
        let public: [u8; 32] = r.vector().try_into().unwrap();
        (public, r.vector().to_vec())
    });
    DecodedAptosTransaction {
        sender,
        function,
        arguments,
        max_gas,
        gas_price,
        authenticator,
    }
}

#[cfg(test)]
mod token_tests {
    use super::*;
    use serde_json::Value;
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
            let (signed, hash) = prepared.sign(&key).unwrap();
            assert_eq!(hash, vector["transaction_hash"].as_str().unwrap());
            assert_eq!(
                hex::encode(&signed[signed.len() - 64..]),
                vector["signature"].as_str().unwrap()
            );
        }
    }
}
