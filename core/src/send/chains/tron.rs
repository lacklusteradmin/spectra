//! TRX/TRC-20: read a block reference, construct locally, sign locally, broadcast.
//! Wire schema: tronprotocol/protocol core/Tron.proto and contract/*.proto.
use crate::derivation::chains::tron::tron_base58_to_evm_hex;
use crate::fetch::chains::tron::{TronClient, TronSendResult};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) enum Transfer<'a> {
    Native {
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

/// Only the block reference is supplied by the node, never a transaction/hash.
pub(crate) struct BlockReference {
    pub number: u64,
    pub id: [u8; 32],
    pub timestamp_ms: u64,
}

pub(crate) struct PreparedTronTransfer {
    owner: [u8; 21],
    raw: Vec<u8>,
    body: Value,
}

fn address(value: &str) -> Result<[u8; 21], String> {
    let hex = tron_base58_to_evm_hex(value)?;
    let mut out = [0x41; 21];
    let bytes = hex::decode(hex).map_err(|_| "invalid Tron address")?;
    if bytes.len() != 20 {
        return Err("invalid Tron address length".into());
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
fn positive_i64(value: u64) -> Result<(), String> {
    if value == 0 || value > i64::MAX as u64 {
        return Err("Tron value must be positive and fit int64".into());
    }
    Ok(())
}

pub(crate) fn prepare_transfer(
    from: &str,
    transfer: Transfer<'_>,
    block: BlockReference,
) -> Result<PreparedTronTransfer, String> {
    let owner = address(from)?;
    positive_i64(block.timestamp_ms)?;
    if block.id[..8] != block.number.to_be_bytes() {
        return Err("Tron block id disagrees with block number".into());
    }
    let expiration = block
        .timestamp_ms
        .checked_add(60_000)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or("Tron expiration overflow")?;
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
        Transfer::Token {
            contract,
            to,
            amount,
            fee_limit,
        } => {
            if amount == 0 {
                return Err("Tron token amount must be positive".into());
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
    pub(crate) fn sign(mut self, key: &[u8]) -> Result<String, String> {
        use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
        use sha3::Keccak256;
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(key).map_err(|_| "invalid Tron signing key")?;
        let public = PublicKey::from_secret_key(&secp, &secret).serialize_uncompressed();
        if self.owner[1..] != Keccak256::digest(&public[1..])[12..] {
            return Err("Tron sender does not match signing key".into());
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

impl TronClient {
    async fn transfer_reference(&self) -> Result<BlockReference, String> {
        let block = self.post("/wallet/getnowblock", &json!({})).await?;
        let number = block
            .pointer("/block_header/raw_data/number")
            .and_then(Value::as_u64)
            .ok_or("missing Tron block number")?;
        let id = hex::decode(block["blockID"].as_str().ok_or("missing Tron block id")?)
            .map_err(|_| "invalid Tron block id")?
            .try_into()
            .map_err(|_| "Tron block id must be 32 bytes")?;
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "clock before epoch")?
            .as_millis()
            .try_into()
            .map_err(|_| "clock overflow")?;
        Ok(BlockReference {
            number,
            id,
            timestamp_ms,
        })
    }
    pub async fn sign_and_broadcast(
        &self,
        from: &str,
        to: &str,
        amount: u64,
        key: &[u8],
    ) -> Result<TronSendResult, String> {
        address(from)?;
        address(to)?;
        positive_i64(amount)?;
        let prepared = prepare_transfer(
            from,
            Transfer::Native { to, amount },
            self.transfer_reference().await?,
        )?;
        self.broadcast_raw(&prepared.sign(key)?).await
    }
    pub async fn sign_and_broadcast_trc20(
        &self,
        from: &str,
        contract: &str,
        to: &str,
        amount: u128,
        fee_limit: u64,
        key: &[u8],
    ) -> Result<TronSendResult, String> {
        address(from)?;
        address(contract)?;
        address(to)?;
        positive_i64(fee_limit)?;
        if amount == 0 {
            return Err("Tron token amount must be positive".into());
        }
        let prepared = prepare_transfer(
            from,
            Transfer::Token {
                contract,
                to,
                amount,
                fee_limit,
            },
            self.transfer_reference().await?,
        )?;
        self.broadcast_raw(&prepared.sign(key)?).await
    }
    pub async fn broadcast_raw(&self, signed_tx_json: &str) -> Result<TronSendResult, String> {
        let body: Value = serde_json::from_str(signed_tx_json)
            .map_err(|e| format!("invalid signed Tron transaction: {e}"))?;
        let raw = hex::decode(
            body["raw_data_hex"]
                .as_str()
                .ok_or("missing Tron raw bytes")?,
        )
        .map_err(|_| "invalid Tron raw bytes")?;
        let txid = hex::encode(Sha256::digest(&raw));
        if body["txID"].as_str() != Some(txid.as_str()) {
            return Err("Tron transaction hash mismatch".into());
        }
        let result = self.post("/wallet/broadcasttransaction", &body).await?;
        if result["result"].as_bool() != Some(true) {
            return Err(format!("Tron broadcast refused: {result}"));
        }
        Ok(TronSendResult {
            txid,
            signed_tx_json: signed_tx_json.into(),
        })
    }
}
