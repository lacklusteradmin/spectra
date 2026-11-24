use super::*;

fn vectors() -> Vec<serde_json::Value> {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/cosmos-vectors.json")).unwrap();
    fixture["transfers"].as_array().unwrap().clone()
}

fn number(vector: &serde_json::Value, field: &str) -> u64 {
    match &vector[field] {
        serde_json::Value::String(text) => text.parse().unwrap(),
        value => value.as_u64().unwrap(),
    }
}

fn prepared(vector: &serde_json::Value) -> (Chain, PreparedCosmosTransfer) {
    let chain = Chain::parse(vector["chain"].as_str().unwrap()).unwrap();
    let transfer = prepare_transfer(
        chain,
        CosmosTransferTerms {
            sender: vector["sender"].as_str().unwrap(),
            recipient: vector["recipient"].as_str().unwrap(),
            amount: vector["amount"].as_str().unwrap().parse().unwrap(),
            memo: vector["memo"].as_str().unwrap(),
            account_number: number(vector, "account_number"),
            sequence: number(vector, "sequence"),
            gas_limit: number(vector, "gas_limit"),
            fee: vector["fee"].as_str().unwrap().parse().unwrap(),
            timeout_height: number(vector, "timeout_height"),
        },
    )
    .unwrap();
    (chain, transfer)
}

fn key(vector: &serde_json::Value) -> SecretKey {
    SecretKey::from_slice(&hex::decode(vector["private_key"].as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn transfers_sign_to_the_bytes_and_hash_cosmjs_signs() {
    for vector in vectors() {
        let (chain, transfer) = prepared(&vector);
        assert_eq!(
            transfer.chain_id,
            vector["chain_id"].as_str().unwrap(),
            "{chain}"
        );
        assert_eq!(transfer.denom, vector["denom"].as_str().unwrap(), "{chain}");
        assert_eq!(
            hex::encode(transfer.body_bytes()),
            vector["body_bytes"].as_str().unwrap(),
            "{chain}"
        );
        let (raw, hash) = transfer.sign(chain, &key(&vector)).unwrap();
        assert_eq!(
            hex::encode(&raw),
            vector["tx_raw"].as_str().unwrap(),
            "{chain}"
        );
        assert_eq!(hash, vector["tx_hash"].as_str().unwrap(), "{chain}");
    }
}

#[test]
fn a_key_for_another_account_or_network_is_refused() {
    let vectors = vectors();
    let (chain, transfer) = prepared(&vectors[0]);
    let other = SecretKey::from_slice(&[7; 32]).unwrap();
    assert!(transfer.sign(chain, &other).is_err());
    // The same transfer presented as another network's.
    assert!(transfer.sign(Chain::Celestia, &key(&vectors[0])).is_err());
    let mut moved = transfer.clone();
    moved.chain_id = "cosmoshub-3".into();
    assert!(moved.sign(chain, &key(&vectors[0])).is_err());
}

#[test]
fn a_transfer_the_network_would_refuse_is_not_built() {
    let vector = &vectors()[0];
    let (chain, _) = prepared(vector);
    let sender = vector["sender"].as_str().unwrap();
    let recipient = vector["recipient"].as_str().unwrap();
    let terms = |recipient, amount, memo, gas_limit, timeout_height| CosmosTransferTerms {
        sender,
        recipient,
        amount,
        memo,
        account_number: 1,
        sequence: 1,
        gas_limit,
        fee: 1,
        timeout_height,
    };
    let limit = chain.memo_text_limit().unwrap();
    let long = "1".repeat(limit + 1);
    // The recipient's own account under another network's prefix.
    let osmosis = crate::derivation::cosmos::encode_address(
        Chain::Osmosis,
        &crate::derivation::cosmos::decode_address(chain, recipient).unwrap(),
    )
    .unwrap();
    for (recipient, amount, memo, gas, timeout) in [
        (osmosis.as_str(), 1, "", 1, 1),
        (recipient, 0, "", 1, 1),
        (recipient, 1, long.as_str(), 1, 1),
        (recipient, 1, "", 0, 1),
        (recipient, 1, "", 1, 0),
    ] {
        assert!(
            prepare_transfer(chain, terms(recipient, amount, memo, gas, timeout)).is_err(),
            "{recipient} {amount} {gas} {timeout}"
        );
    }
    assert!(prepare_transfer(chain, terms(recipient, 1, &"1".repeat(limit), 1, 1)).is_ok());
}

#[test]
fn a_simulation_carries_no_key_and_one_empty_signature() {
    let (_, transfer) = prepared(&vectors()[0]);
    let simulation = transfer.simulation_bytes();
    let raw = proto::TxRaw::decode(simulation.as_slice()).unwrap();
    assert_eq!(raw.body_bytes, transfer.body_bytes());
    assert_eq!(raw.signatures, vec![Vec::<u8>::new()]);
    let auth = proto::AuthInfo::decode(raw.auth_info_bytes.as_slice()).unwrap();
    assert!(auth.signer_infos[0].public_key.is_none());
    assert_eq!(auth.signer_infos[0].sequence, transfer.sequence);
}
