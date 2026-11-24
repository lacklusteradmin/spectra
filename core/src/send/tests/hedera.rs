use super::*;
use sha2::Digest as _;

/// A node transaction's hash, as the network and the SDK name it: SHA-384
/// of its `signedTransactionBytes`.
fn transaction_hash(transaction: &[u8]) -> Result<Vec<u8>, SendError> {
    let transaction = proto::Transaction::decode(transaction)
        .map_err(|_| SendError::invalid("Not a Hedera transaction"))?;
    Ok(sha2::Sha384::digest(&transaction.signed_transaction_bytes).to_vec())
}

fn vectors() -> Vec<serde_json::Value> {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/hedera-vectors.json")).unwrap();
    fixture["transfers"].as_array().unwrap().clone()
}

fn nodes(vector: &serde_json::Value) -> Vec<HederaAccountId> {
    vector["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| HederaAccountId::parse(Chain::Hedera, node.as_str().unwrap()).unwrap())
        .collect()
}

fn prepared(vector: &serde_json::Value) -> PreparedHederaTransfer {
    prepare_transfer(
        Chain::Hedera,
        HederaTransferTerms {
            payer: vector["payer"].as_str().unwrap(),
            public_key: vector["public_key"].as_str().unwrap(),
            recipient: vector["recipient"].as_str().unwrap(),
            amount: vector["amount"].as_str().unwrap().parse().unwrap(),
            memo: vector["memo"].as_str().unwrap(),
            max_fee: vector["max_fee"].as_str().unwrap().parse().unwrap(),
            nodes: &nodes(vector),
        },
    )
    .unwrap()
}

fn key(vector: &serde_json::Value) -> HederaPrivateKey {
    crate::derivation::hedera::stored_key(vector["private_key"].as_str().unwrap()).unwrap()
}

fn start(vector: &serde_json::Value) -> (i64, i32) {
    (
        vector["valid_start_seconds"].as_i64().unwrap(),
        vector["valid_start_nanos"].as_i64().unwrap() as i32,
    )
}

/// The SDK's `payer@seconds.nanos` as a mirror node writes it.
fn mirror_id(sdk: &str) -> String {
    let (payer, start) = sdk.split_once('@').unwrap();
    let (seconds, nanos) = start.split_once('.').unwrap();
    format!("{payer}-{seconds}-{nanos}")
}

#[test]
fn transfers_sign_to_the_bytes_the_hedera_sdk_signs() {
    for vector in vectors() {
        let transfer = prepared(&vector);
        let signed = transfer
            .sign(Chain::Hedera, &key(&vector), start(&vector))
            .unwrap();
        assert_eq!(
            signed.transaction_id,
            mirror_id(vector["transaction_id"].as_str().unwrap())
        );
        let per_node = vector["per_node"].as_array().unwrap();
        let split = node_transactions(&signed.transaction_list).unwrap();
        assert_eq!(split.len(), per_node.len());
        for (node, expected) in split.iter().zip(per_node) {
            assert_eq!(node.node.to_string(), expected["node"].as_str().unwrap());
            assert_eq!(
                hex::encode(
                    transfer
                        .body_bytes(
                            Chain::Hedera,
                            expected["node"].as_str().unwrap(),
                            start(&vector)
                        )
                        .unwrap()
                ),
                expected["body_bytes"].as_str().unwrap()
            );
            assert_eq!(
                hex::encode(&node.transaction),
                expected["transaction"].as_str().unwrap()
            );
            assert_eq!(node.transaction_id, signed.transaction_id);
            assert_eq!(
                hex::encode(transaction_hash(&node.transaction).unwrap()),
                expected["transaction_hash"].as_str().unwrap()
            );
        }
        // The whole list is the SDK's `toBytes()`: each node's Transaction
        // under field 1.
        let list = proto::TransactionList {
            transaction_list: per_node
                .iter()
                .map(|node| proto::Transaction {
                    signed_transaction_bytes: proto::SignedTransaction::decode(
                        hex::decode(node["signed_transaction"].as_str().unwrap())
                            .unwrap()
                            .as_slice(),
                    )
                    .unwrap()
                    .encode_to_vec(),
                })
                .collect(),
        };
        assert_eq!(signed.transaction_list, list.encode_to_vec());
    }
}

#[test]
fn a_key_the_transfer_was_not_built_for_is_refused() {
    let vectors = vectors();
    let transfer = prepared(&vectors[0]);
    // The ECDSA key of the second vector, and an unrelated Ed25519 key.
    assert!(
        transfer
            .sign(Chain::Hedera, &key(&vectors[1]), start(&vectors[0]))
            .is_err()
    );
    let other = crate::derivation::hedera::HederaPrivateKey::new(
        crate::derivation::hedera::HederaKeyScheme::Ed25519,
        zeroize::Zeroizing::new([7; 32]),
    )
    .unwrap();
    assert!(
        transfer
            .sign(Chain::Hedera, &other, start(&vectors[0]))
            .is_err()
    );
    assert!(
        transfer
            .sign(Chain::Ethereum, &key(&vectors[0]), start(&vectors[0]))
            .is_err()
    );
    // A valid start the network cannot read.
    assert!(
        transfer
            .sign(
                Chain::Hedera,
                &key(&vectors[0]),
                (1_795_522_566, 1_000_000_000)
            )
            .is_err()
    );
}

#[test]
fn a_transfer_the_network_would_refuse_is_not_built() {
    let vector = &vectors()[0];
    let public_key = vector["public_key"].as_str().unwrap();
    let three = nodes(vector);
    let terms = |payer, recipient, amount, memo, max_fee, nodes| HederaTransferTerms {
        payer,
        public_key,
        recipient,
        amount,
        memo,
        max_fee,
        nodes,
    };
    let long = "x".repeat(101);
    let full = "é".repeat(50);
    for (payer, recipient, amount, memo, max_fee, nodes) in [
        ("0.0.1234", "0.0.1234", 1, "", 1, &three[..]),
        ("0.0.1234", "0.0.98765", 0, "", 1, &three[..]),
        (
            "0.0.1234",
            "0.0.98765",
            u128::from(u64::MAX),
            "",
            1,
            &three[..],
        ),
        ("0.0.1234", "0.0.98765", 1, long.as_str(), 1, &three[..]),
        ("0.0.1234", "0.0.98765", 1, "a\0b", 1, &three[..]),
        ("0.0.1234", "0.0.98765", 1, "", 0, &three[..]),
        ("0.0.1234", "0.0.98765", 1, "", 1, &[][..]),
        ("0.0.1234", "0.0.98765-abcde", 1, "", 1, &three[..]),
        ("0.0.1234", "1.0.98765", 1, "", 1, &three[..]),
    ] {
        assert!(
            prepare_transfer(
                Chain::Hedera,
                terms(payer, recipient, amount, memo, max_fee, nodes)
            )
            .is_err(),
            "{payer} {recipient} {amount} {memo:?} {max_fee} {}",
            nodes.len()
        );
    }
    // 100 bytes of memo, a checksummed recipient written without it, and a
    // node named twice signed for once.
    let twice = [three[0], three[0]];
    let built = prepare_transfer(
        Chain::Hedera,
        terms("0.0.1234", "0.0.98765-lueab", 1, &full, 1, &twice),
    )
    .unwrap();
    assert_eq!(built.recipient, "0.0.98765");
    assert_eq!(built.nodes, ["0.0.3"]);
}

#[test]
fn a_list_of_mixed_transactions_is_refused() {
    let vectors = vectors();
    let first = prepared(&vectors[0])
        .sign(Chain::Hedera, &key(&vectors[0]), start(&vectors[0]))
        .unwrap();
    let other = prepared(&vectors[2])
        .sign(Chain::Hedera, &key(&vectors[2]), start(&vectors[2]))
        .unwrap();
    let mut mixed = proto::TransactionList::decode(first.transaction_list.as_slice()).unwrap();
    mixed.transaction_list.extend(
        proto::TransactionList::decode(other.transaction_list.as_slice())
            .unwrap()
            .transaction_list,
    );
    assert!(node_transactions(&mixed.encode_to_vec()).is_err());
    assert!(node_transactions(&[]).is_err());
    assert!(node_transactions(b"not protobuf").is_err());
}
