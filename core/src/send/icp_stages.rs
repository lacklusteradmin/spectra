//! Locally constructed ICP ledger `send_pb` calls and Rosetta envelopes.
//! Wire definitions: dfinity/ic rs/rosetta-api/icp/{models,convert}.rs and
//! rs/ledger_suite/icp/proto/ic_ledger/pb/v1/types.proto.

use crate::api::icp_rosetta::{IcpClient, network_identifier as network};
use crate::send::error::SendError;
use crate::send::keys::Ed25519Seed;
use crate::{derivation::icp::*, registry::Chain};
use ciborium::Value as Cbor;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedIcpTransaction {
    pub sender: String,
    pub recipient: String,
    pub amount: u64,
    pub fee: u64,
    pub memo: u64,
    pub created_at_time_ns: u64,
    pub ingress_expiry_ns: u64,
    pub ledger_canister: String,
    pub argument_hex: String,
}

fn leb(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (n & 127) as u8;
        n >>= 7;
        out.push(byte | if n == 0 { 0 } else { 128 });
        if n == 0 {
            return out;
        }
    }
}
fn message(field: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![(field << 3) | 2];
    out.extend(leb(bytes.len() as u64));
    out.extend(bytes);
    out
}
fn integer(n: u64) -> Vec<u8> {
    if n == 0 {
        Vec::new()
    } else {
        let mut out = vec![8];
        out.extend(leb(n));
        out
    }
}
/// A text-keyed map in canonical CBOR key order (RFC 7049 §3.9): shorter keys
/// first, then bytewise.
fn map(mut entries: Vec<(&str, Cbor)>) -> Cbor {
    entries.sort_by(|(a, _), (b, _)| (a.len(), a).cmp(&(b.len(), b)));
    Cbor::Map(
        entries
            .into_iter()
            .map(|(k, v)| (Cbor::Text(k.into()), v))
            .collect(),
    )
}
fn encode(value: &Cbor) -> Result<Vec<u8>, SendError> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).map_err(SendError::invalid)?;
    Ok(out)
}
fn text(s: &str) -> Cbor {
    Cbor::Text(s.into())
}
fn bytes(b: &[u8]) -> Cbor {
    Cbor::Bytes(b.to_vec())
}

/// IC representation-independent request identifier, independent of CBOR key order.
fn request_hash(value: &Cbor) -> Result<[u8; 32], SendError> {
    let data = match value {
        Cbor::Text(s) => s.as_bytes().to_vec(),
        Cbor::Bytes(b) => b.clone(),
        Cbor::Integer(n) => leb(u64::try_from(*n)
            .map_err(|_| SendError::Invalid("Invalid IC unsigned integer".into()))?),
        Cbor::Array(items) => items
            .iter()
            .map(request_hash)
            .collect::<Result<Vec<_>, _>>()?
            .concat(),
        Cbor::Map(entries) => {
            let mut pairs = entries
                .iter()
                .map(|(k, v)| Ok((request_hash(k)?, request_hash(v)?)))
                .collect::<Result<Vec<_>, SendError>>()?;
            pairs.sort_unstable();
            pairs
                .into_iter()
                .flat_map(|(k, v)| [k, v].concat())
                .collect()
        }
        _ => return Err(SendError::Invalid("Unsupported IC request value".into())),
    };
    Ok(Sha256::digest(data).into())
}
fn envelope(content: Cbor, key: &Ed25519Seed) -> Result<Cbor, SendError> {
    let mut signing = b"\x0aic-request".to_vec();
    signing.extend(request_hash(&content)?);
    let sig = key.sign(&signing);
    Ok(map(vec![
        ("content", content),
        ("sender_pubkey", bytes(&public_key_der(&key.public_key()))),
        ("sender_sig", bytes(&sig)),
    ]))
}

impl PreparedIcpTransaction {
    pub(crate) fn transaction_hash(&self) -> Result<String, SendError> {
        // Ledger hashes packed CBOR, not the ingress request ID. Packed field
        // indices and Transfer variant index follow icp_ledger::Transaction.
        fn packed(mut entries: Vec<(u64, Cbor)>) -> Cbor {
            entries.sort_by_key(|(k, _)| *k);
            Cbor::Map(
                entries
                    .into_iter()
                    .map(|(k, v)| (Cbor::Integer(k.into()), v))
                    .collect(),
            )
        }
        let transfer = packed(vec![
            (0, text(&self.sender)),
            (1, text(&self.recipient)),
            (2, packed(vec![(0, Cbor::Integer(self.amount.into()))])),
            (3, packed(vec![(0, Cbor::Integer(self.fee.into()))])),
        ]);
        let transaction = packed(vec![
            (0, packed(vec![(2, transfer)])),
            (1, Cbor::Integer(self.memo.into())),
            (
                2,
                packed(vec![(0, Cbor::Integer(self.created_at_time_ns.into()))]),
            ),
        ]);
        Ok(hex::encode(Sha256::digest(encode(&transaction)?)))
    }

    fn argument(&self) -> Result<Vec<u8>, SendError> {
        let to = validate_account(&self.recipient)?;
        let mut arg = message(1, &integer(self.memo));
        arg.extend(message(2, &message(1, &integer(self.amount))));
        arg.extend(message(3, &integer(self.fee)));
        arg.extend(message(5, &message(1, &to)));
        arg.extend(message(7, &integer(self.created_at_time_ns)));
        Ok(arg)
    }
    pub(crate) fn sign(&self, key: &Ed25519Seed) -> Result<String, SendError> {
        let sender = principal(&key.public_key());
        if hex::encode(account_from_principal(&sender)) != self.sender {
            return Err(SendError::Invalid(
                "ICP sender does not match signing key".into(),
            ));
        }
        let arg = self.argument()?;
        if hex::encode(&arg) != self.argument_hex
            || self.ledger_canister != Chain::Icp.icp_ledger_id()?
        {
            return Err(SendError::Invalid(
                "ICP reviewed transfer content changed".into(),
            ));
        }
        let content = map(vec![
            ("request_type", text("call")),
            ("canister_id", bytes(&hex::decode(&self.ledger_canister)?)),
            ("method_name", text("send_pb")),
            ("arg", bytes(&arg)),
            ("sender", bytes(&sender)),
            (
                "ingress_expiry",
                Cbor::Integer(self.ingress_expiry_ns.into()),
            ),
        ]);
        let id = request_hash(&content)?;
        let read = map(vec![
            ("request_type", text("read_state")),
            ("sender", bytes(&sender)),
            (
                "ingress_expiry",
                Cbor::Integer(self.ingress_expiry_ns.into()),
            ),
            (
                "paths",
                Cbor::Array(vec![Cbor::Array(vec![
                    bytes(b"request_status"),
                    bytes(&id),
                ])]),
            ),
        ]);
        let pair = map(vec![
            ("update", envelope(content, key)?),
            ("read_state", envelope(read, key)?),
        ]);
        let signed = map(vec![(
            "requests",
            Cbor::Array(vec![Cbor::Array(vec![
                text("TRANSACTION"),
                Cbor::Array(vec![pair]),
            ])]),
        )]);
        let raw = encode(&signed)?;
        Ok(
            json!({"network_identifier":network(),"signed_transaction":hex::encode(raw)})
                .to_string(),
        )
    }
}

pub(crate) async fn prepare_transfer(
    client: &IcpClient,
    sender: &str,
    recipient: &str,
    amount: u64,
) -> Result<PreparedIcpTransaction, SendError> {
    validate_account(sender)?;
    validate_account(recipient)?;
    client.verify_network().await?;
    let fee = u64::try_from(
        Chain::Icp
            .static_fee_units()
            .ok_or_else(|| SendError::Invalid("Missing ICP ledger fee".into()))?,
    )
    .map_err(|_| SendError::Invalid("ICP fee overflow".into()))?;
    let operations = json!([
        {"operation_identifier":{"index":0},"type":"TRANSACTION","account":{"address":sender},"amount":{"value":format!("-{amount}"),"currency":{"symbol":"ICP","decimals":8}}},
        {"operation_identifier":{"index":1},"type":"TRANSACTION","account":{"address":recipient},"amount":{"value":amount.to_string(),"currency":{"symbol":"ICP","decimals":8}}},
        {"operation_identifier":{"index":2},"type":"FEE","account":{"address":sender},"amount":{"value":format!("-{fee}"),"currency":{"symbol":"ICP","decimals":8}}}
    ]);
    let pre: Value = client
        .rosetta_post(
            "/construction/preprocess",
            &json!({"network_identifier":network(),"operations":operations}),
        )
        .await?;
    let options = pre
        .get("options")
        .ok_or_else(|| SendError::Invalid("Missing ICP construction options".into()))?;
    let meta: Value = client
        .rosetta_post(
            "/construction/metadata",
            &json!({"network_identifier":network(),"options":options}),
        )
        .await?;
    let fees = meta["suggested_fee"]
        .as_array()
        .ok_or_else(|| SendError::Invalid("Missing ICP suggested fee".into()))?;
    if fees.len() != 1
        || fees[0]["value"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            != Some(fee)
        || fees[0]["currency"] != json!({"symbol":"ICP","decimals":8})
    {
        return Err(SendError::Invalid(
            "ICP ledger fee changed or is unsupported".into(),
        ));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(SendError::invalid)?
        .as_nanos();
    let now =
        u64::try_from(now).map_err(|_| SendError::Invalid("ICP timestamp overflow".into()))?;
    let mut prepared = PreparedIcpTransaction {
        sender: sender.to_lowercase(),
        recipient: recipient.to_lowercase(),
        amount,
        fee,
        memo: rand::random(),
        created_at_time_ns: now,
        ingress_expiry_ns: now
            .checked_add(240_000_000_000)
            .ok_or_else(|| SendError::Invalid("ICP expiry overflow".into()))?,
        ledger_canister: Chain::Icp.icp_ledger_id()?.into(),
        argument_hex: String::new(),
    };
    prepared.argument_hex = hex::encode(prepared.argument()?);
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_envelopes_bind_only_the_reviewed_transfer_and_status_path() {
        let key = Ed25519Seed::from_hex(&"01".repeat(32)).unwrap();
        let sender = hex::encode(account_from_principal(&principal(&key.public_key())));
        let mut p = PreparedIcpTransaction {
            sender,
            recipient: "807077e900000000000000000000000000000000000000000000000000000000".into(),
            amount: 123,
            fee: 10_000,
            memo: 42,
            created_at_time_ns: 123456789,
            ingress_expiry_ns: 234567890,
            ledger_canister: Chain::Icp.icp_ledger_id().unwrap().into(),
            argument_hex: String::new(),
        };
        p.argument_hex = hex::encode(p.argument().unwrap());
        let payload: Value = serde_json::from_str(&p.sign(&key).unwrap()).unwrap();
        let signed: Cbor = ciborium::from_reader(
            hex::decode(payload["signed_transaction"].as_str().unwrap())
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        fn get<'a>(map: &'a [(Cbor, Cbor)], key: &str) -> &'a Cbor {
            &map.iter().find(|(k, _)| *k == text(key)).unwrap().1
        }
        let Cbor::Map(root) = signed else { panic!() };
        let Cbor::Array(requests) = get(&root, "requests") else {
            panic!()
        };
        let Cbor::Array(request) = &requests[0] else {
            panic!()
        };
        assert_eq!(request[0], text("TRANSACTION"));
        let Cbor::Array(pairs) = &request[1] else {
            panic!()
        };
        let Cbor::Map(pair) = &pairs[0] else { panic!() };
        for kind in ["update", "read_state"] {
            let Cbor::Map(envelope) = get(pair, kind) else {
                panic!()
            };
            assert_eq!(envelope.len(), 3);
            let content = get(envelope, "content");
            let Cbor::Bytes(signature) = get(envelope, "sender_sig") else {
                panic!()
            };
            let mut message = b"\x0aic-request".to_vec();
            message.extend(request_hash(content).unwrap());
            ed25519_dalek::VerifyingKey::from_bytes(&key.public_key())
                .unwrap()
                .verify_strict(
                    &message,
                    &ed25519_dalek::Signature::from_slice(signature).unwrap(),
                )
                .unwrap();
        }
        p.amount += 1;
        assert!(p.sign(&key).unwrap_err().to_string().contains("changed"));
        p.amount -= 1;
        p.sender = p.recipient.clone();
        assert!(
            p.sign(&key)
                .unwrap_err()
                .to_string()
                .contains("signing key")
        );
    }

    /// The signed envelope, byte for byte: canonical CBOR key order and
    /// deterministic Ed25519. Captured from the previous CBOR library, so a
    /// library change that reorders or re-encodes anything fails here.
    #[test]
    fn signed_envelope_bytes_are_pinned() {
        let key = Ed25519Seed::from_hex(&"01".repeat(32)).unwrap();
        let sender = hex::encode(account_from_principal(&principal(&key.public_key())));
        let mut p = PreparedIcpTransaction {
            sender,
            recipient: "807077e900000000000000000000000000000000000000000000000000000000".into(),
            amount: 123,
            fee: 10_000,
            memo: 42,
            created_at_time_ns: 123456789,
            ingress_expiry_ns: 234567890,
            ledger_canister: Chain::Icp.icp_ledger_id().unwrap().into(),
            argument_hex: String::new(),
        };
        p.argument_hex = hex::encode(p.argument().unwrap());
        let payload: Value = serde_json::from_str(&p.sign(&key).unwrap()).unwrap();
        let raw = hex::decode(payload["signed_transaction"].as_str().unwrap()).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(raw)),
            "66a46f5fec7970295086a1e145237b2dbfcd877e9cfb5b90d02a49223d6e49e1"
        );
    }

    #[test]
    fn official_ledger_transaction_hash_vector() {
        // dfinity/ic rs/ledger_suite/icp/src/lib.rs::tests::transaction_hash.
        let p = PreparedIcpTransaction {
            sender: "e7a879ea563d273c46dd28c1584eaa132fad6f3e316615b3eb657d067f3519b5".into(),
            recipient: "207ec07185bedd0f2176ec2760057b8b7bc619a94d60e70fbc91af322a9f7e93".into(),
            amount: 11_541_900_000,
            fee: 10_000,
            memo: 5_432_845_643_782_906_771,
            created_at_time_ns: 1_621_901_572_293_430_780,
            ingress_expiry_ns: 0,
            ledger_canister: Chain::Icp.icp_ledger_id().unwrap().into(),
            argument_hex: String::new(),
        };
        assert_eq!(
            p.transaction_hash().unwrap(),
            "be31664ef154456aec5df2e4acc7f23a715ad8ea33ad9dbcbb7e6e90bc5a8b8f"
        );
    }
}
