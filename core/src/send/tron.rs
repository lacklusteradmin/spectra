//! TRX/TRC-10/TRC-20: construct and sign locally against a block reference that
//! `api::tron_http` reads.
//! Wire schema: tronprotocol/protocol core/Tron.proto and contract/*.proto.

use crate::api::tron_http::BlockReference;
use crate::derivation::tron::tron_base58_to_evm_hex;
use crate::send::error::SendError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The most energy a TRC-20 transfer may burn, in sun: the `fee_limit` every
/// token transfer is signed with.
pub(crate) const TRC20_FEE_LIMIT_SUN: u64 = 100_000_000;
/// What a TRC-20 transfer usually burns when the account has no energy, in
/// sun: the preview's static estimate, under the limit above.
pub(crate) const TRC20_TYPICAL_FEE_SUN: u64 = 15_000_000;

pub(crate) enum Transfer<'a> {
    Native {
        to: &'a str,
        amount: u64,
    },
    Trc10 {
        asset_id: &'a str,
        to: &'a str,
        amount: u64,
    },
    Token {
        contract: &'a str,
        to: &'a str,
        amount: u128,
        fee_limit: u64,
    },
}

#[cfg(test)]
mod trc10_vectors {
    use super::*;

    #[test]
    fn trc10_raw_hash_and_signature_match_official_tronweb() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/trc10-send-vectors.json"))
                .unwrap();
        let owner = fixture["owner"].as_str().unwrap();
        let to = fixture["receiver"].as_str().unwrap();
        let key = hex::decode(fixture["key"].as_str().unwrap()).unwrap();
        let block = || BlockReference {
            number: fixture["block"]["number"].as_u64().unwrap(),
            id: hex::decode(fixture["block"]["id"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
            timestamp_ms: fixture["block"]["timestamp_ms"].as_u64().unwrap(),
        };
        for vector in fixture["vectors"].as_array().unwrap() {
            let prepared = prepare_transfer(
                owner,
                Transfer::Trc10 {
                    asset_id: vector["asset_id"].as_str().unwrap(),
                    to,
                    amount: vector["amount"].as_str().unwrap().parse().unwrap(),
                },
                block(),
            )
            .unwrap();
            assert!(prepared.bandwidth_bytes().unwrap() < 512);
            assert!(prepared.body["raw_data"].get("fee_limit").is_none());
            let signed: Value = serde_json::from_str(&prepared.sign(&key).unwrap()).unwrap();
            assert_eq!(signed["raw_data_hex"], vector["raw"]);
            assert_eq!(signed["txID"], vector["txid"]);
            assert_eq!(
                hex::decode(signed["signature"][0].as_str().unwrap()).unwrap(),
                hex::decode(vector["signature"].as_str().unwrap()).unwrap()
            );
        }
        for (asset_id, amount, receiver) in [
            ("1002000", 0, to),
            ("1002000", i64::MAX as u64 + 1, to),
            ("1002000", 1, owner),
            ("001002000", 1, to),
            ("a1002000", 1, to),
        ] {
            assert!(
                prepare_transfer(
                    owner,
                    Transfer::Trc10 {
                        asset_id,
                        to: receiver,
                        amount
                    },
                    block()
                )
                .is_err()
            );
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedTronTransfer {
    owner: [u8; 21],
    pub(crate) raw: Vec<u8>,
    pub(crate) body: Value,
}

fn address(value: &str) -> Result<[u8; 21], SendError> {
    let hex = tron_base58_to_evm_hex(value)?;
    let mut out = [0x41; 21];
    let bytes = hex::decode(hex).map_err(|_| SendError::Invalid("invalid Tron address".into()))?;
    if bytes.len() != 20 {
        return Err(SendError::Invalid("invalid Tron address length".into()));
    }
    out[1..].copy_from_slice(&bytes);
    Ok(out)
}
fn varint(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let b = (n & 127) as u8;
        n >>= 7;
        out.push(b | if n == 0 { 0 } else { 128 });
        if n == 0 {
            break;
        }
    }
}
fn bytes(field: u64, value: &[u8], out: &mut Vec<u8>) {
    varint(field * 8 + 2, out);
    varint(value.len() as u64, out);
    out.extend_from_slice(value);
}
fn integer(field: u64, value: u64, out: &mut Vec<u8>) {
    if value != 0 {
        varint(field * 8, out);
        varint(value, out);
    }
}
fn positive_i64(value: u64) -> Result<(), SendError> {
    if value == 0 || value > i64::MAX as u64 {
        return Err(SendError::Invalid(
            "Tron value must be positive and fit int64".into(),
        ));
    }
    Ok(())
}

pub(crate) fn prepare_transfer(
    from: &str,
    transfer: Transfer<'_>,
    block: BlockReference,
) -> Result<PreparedTronTransfer, SendError> {
    let owner = address(from)?;
    positive_i64(block.timestamp_ms)?;
    if block.id[..8] != block.number.to_be_bytes() {
        return Err(SendError::Invalid(
            "Tron block id disagrees with block number".into(),
        ));
    }
    let expiration = block
        .timestamp_ms
        .checked_add(60_000)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(|| SendError::Invalid("Tron expiration overflow".into()))?;
    let mut value = Vec::new();
    bytes(1, &owner, &mut value);
    let (kind, name, parameter, fee) = match transfer {
        Transfer::Native { to, amount } => {
            positive_i64(amount)?;
            let to = address(to)?;
            bytes(2, &to, &mut value);
            integer(3, amount, &mut value);
            (
                1,
                "TransferContract",
                json!({"owner_address":hex::encode(owner),"to_address":hex::encode(to),"amount":amount}),
                0,
            )
        }
        Transfer::Trc10 {
            asset_id,
            to,
            amount,
        } => {
            crate::api::tron_http::validate_asset_id(asset_id)
                .map_err(|error| SendError::Invalid(error.to_string().into()))?;
            positive_i64(amount)?;
            let to = address(to)?;
            if owner == to {
                return Err(SendError::Invalid(
                    "TRC-10 cannot transfer to its sender".into(),
                ));
            }
            // TransferAssetContract's field layout differs from TransferContract.
            value.clear();
            bytes(1, asset_id.as_bytes(), &mut value);
            bytes(2, &owner, &mut value);
            bytes(3, &to, &mut value);
            integer(4, amount, &mut value);
            (
                2,
                "TransferAssetContract",
                json!({"asset_name":hex::encode(asset_id),"owner_address":hex::encode(owner),"to_address":hex::encode(to),"amount":amount}),
                0,
            )
        }
        Transfer::Token {
            contract,
            to,
            amount,
            fee_limit,
        } => {
            if amount == 0 {
                return Err(SendError::Invalid(
                    "Tron token amount must be positive".into(),
                ));
            }
            positive_i64(fee_limit)?;
            let contract = address(contract)?;
            let to = address(to)?;
            let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
            data.extend_from_slice(&[0; 12]);
            data.extend_from_slice(&to[1..]);
            data.extend_from_slice(&[0; 16]);
            data.extend_from_slice(&amount.to_be_bytes());
            bytes(2, &contract, &mut value);
            bytes(4, &data, &mut value);
            (
                31,
                "TriggerSmartContract",
                json!({"owner_address":hex::encode(owner),"contract_address":hex::encode(contract),"data":hex::encode(data)}),
                fee_limit,
            )
        }
    };
    let type_url = format!("type.googleapis.com/protocol.{name}");
    let mut any = Vec::new();
    bytes(1, type_url.as_bytes(), &mut any);
    bytes(2, &value, &mut any);
    let mut contract = Vec::new();
    integer(1, kind, &mut contract);
    bytes(2, &any, &mut contract);
    let ref_bytes = &block.number.to_be_bytes()[6..];
    let ref_hash = &block.id[8..16];
    let mut raw = Vec::new();
    bytes(1, ref_bytes, &mut raw);
    bytes(4, ref_hash, &mut raw);
    integer(8, expiration, &mut raw);
    bytes(11, &contract, &mut raw);
    integer(14, block.timestamp_ms, &mut raw);
    integer(18, fee, &mut raw);
    let mut raw_json = json!({"ref_block_bytes":hex::encode(ref_bytes),"ref_block_hash":hex::encode(ref_hash),"expiration":expiration,"timestamp":block.timestamp_ms,"contract":[{"type":name,"parameter":{"type_url":type_url,"value":parameter}}]});
    if fee != 0 {
        raw_json["fee_limit"] = json!(fee);
    }
    let body = json!({"visible":false,"raw_data":raw_json,"raw_data_hex":hex::encode(&raw),"txID":hex::encode(Sha256::digest(&raw))});
    Ok(PreparedTronTransfer { owner, raw, body })
}

impl PreparedTronTransfer {
    /// Protobuf Transaction with one 65-byte signature, plus java-tron's
    /// MAX_RESULT_SIZE_IN_TX bandwidth reservation. No TRC-10 VM energy.
    pub(crate) fn bandwidth_bytes(&self) -> Result<u64, SendError> {
        let mut encoded = Vec::new();
        bytes(1, &self.raw, &mut encoded);
        bytes(2, &[0; 65], &mut encoded);
        u64::try_from(encoded.len() + 64)
            .map_err(|_| SendError::Invalid("Tron transaction size overflow".into()))
    }

    pub(crate) fn sign(mut self, key: &[u8]) -> Result<String, SendError> {
        use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
        use sha3::Keccak256;
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(key)
            .map_err(|_| SendError::Invalid("invalid Tron signing key".into()))?;
        let public = PublicKey::from_secret_key(&secp, &secret).serialize_uncompressed();
        if self.owner[1..] != Keccak256::digest(&public[1..])[12..] {
            return Err(SendError::Invalid(
                "Tron sender does not match signing key".into(),
            ));
        }
        let hash: [u8; 32] = Sha256::digest(&self.raw).into();
        let (recovery, signature) = secp
            .sign_ecdsa_recoverable(&Message::from_digest(hash), &secret)
            .serialize_compact();
        let mut sig = signature.to_vec();
        sig.push(recovery.to_i32() as u8 + 27);
        self.body["signature"] = json!([hex::encode(sig)]);
        Ok(self.body.to_string())
    }
}
