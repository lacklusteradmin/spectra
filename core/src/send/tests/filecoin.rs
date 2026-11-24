use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/filecoin-vectors.json"
    ))
    .unwrap()
}

fn number(value: &serde_json::Value) -> u64 {
    value.as_u64().unwrap()
}

fn digits(value: &serde_json::Value) -> u128 {
    value.as_str().unwrap().parse().unwrap()
}

/// The message a vector describes, as Lotus writes it.
fn prepared(vector: &serde_json::Value) -> PreparedFilecoinMessage {
    let message = &vector["message"];
    prepare_transfer(
        Chain::Filecoin,
        FilecoinTransferTerms {
            sender: message["From"].as_str().unwrap(),
            recipient: message["To"].as_str().unwrap(),
            value: digits(&message["Value"]),
            method: number(&message["Method"]),
            nonce: number(&message["Nonce"]),
            gas: FilecoinGas {
                limit: number(&message["GasLimit"]),
                fee_cap: digits(&message["GasFeeCap"]),
                premium: digits(&message["GasPremium"]),
            },
        },
    )
    .unwrap()
}

fn key(vector: &serde_json::Value) -> SecretKey {
    SecretKey::from_slice(&hex::decode(vector["private_key"].as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn messages_serialize_hash_and_sign_as_filecoin_signing_tools_does() {
    for vector in fixture()["messages"].as_array().unwrap() {
        let transfer = prepared(vector);
        let cbor = transfer.message_bytes(Chain::Filecoin).unwrap();
        assert_eq!(hex::encode(&cbor), vector["cbor"].as_str().unwrap());
        let (cid_bytes, cid) = crate::derivation::filecoin::message_cid(&cbor);
        assert_eq!(
            hex::encode(cid_bytes),
            vector["cid_bytes"].as_str().unwrap()
        );
        assert_eq!(cid, vector["cid"].as_str().unwrap());
        let (signed, signed_cid) = transfer.sign(Chain::Filecoin, &key(vector)).unwrap();
        assert_eq!(
            hex::encode(&signed.signature),
            vector["signature"].as_str().unwrap()
        );
        assert_eq!(signed_cid, vector["signed_cid"].as_str().unwrap());
        // What Lotus is handed is the message signed.
        let message = &vector["message"];
        assert_eq!(signed.message.to, message["To"].as_str().unwrap());
        assert_eq!(signed.message.value, message["Value"].as_str().unwrap());
        assert_eq!(signed.message.nonce, number(&message["Nonce"]));
        assert_eq!(signed.message.method, number(&message["Method"]));
        assert_eq!(
            signed.message.gas_premium,
            message["GasPremium"].as_str().unwrap()
        );
    }
}

#[test]
fn a_mainnet_message_has_the_cids_the_chain_gave_it() {
    let mainnet = &fixture()["mainnet"];
    let transfer = prepared(mainnet);
    let cbor = transfer.message_bytes(Chain::Filecoin).unwrap();
    assert_eq!(hex::encode(&cbor), mainnet["cbor"].as_str().unwrap());
    assert_eq!(
        crate::derivation::filecoin::message_cid(&cbor).1,
        mainnet["cid"].as_str().unwrap()
    );
    // Its signature, over BLAKE2b-256 of the CID's bytes, recovers the key
    // its sender's address hashes.
    let signature = hex::decode(mainnet["signature"].as_str().unwrap()).unwrap();
    let (cid, _) = crate::derivation::filecoin::message_cid(&cbor);
    let recoverable = secp256k1::ecdsa::RecoverableSignature::from_compact(
        &signature[..64],
        secp256k1::ecdsa::RecoveryId::from_i32(i32::from(signature[64])).unwrap(),
    )
    .unwrap();
    let digest = crate::derivation::primitives::blake2b::<32>(&[&cid]);
    let signer = Secp256k1::verification_only()
        .recover_ecdsa(&Message::from_digest(digest), &recoverable)
        .unwrap();
    assert_eq!(
        crate::derivation::filecoin::address_from_public_key(Chain::Filecoin, &signer).unwrap(),
        transfer.sender
    );
    // The signed message's CID is the one it was mined under.
    assert_eq!(
        transfer.signed_cid(Chain::Filecoin, &signature).unwrap(),
        mainnet["signed_cid"].as_str().unwrap()
    );
}

#[test]
fn a_key_for_another_account_is_refused() {
    let vector = &fixture()["messages"][0];
    let transfer = prepared(vector);
    let other = SecretKey::from_slice(&[7; 32]).unwrap();
    assert!(transfer.sign(Chain::Filecoin, &other).is_err());
}

#[test]
fn a_transfer_the_network_would_refuse_or_misdeliver_is_not_built() {
    let fixture = fixture();
    let sender = fixture["accounts"][0]["address"].as_str().unwrap();
    let recipient = fixture["accounts"][1]["address"].as_str().unwrap();
    let f410 = fixture["recipients"][5]["address"].as_str().unwrap();
    let f0 = fixture["recipients"][0]["address"].as_str().unwrap();
    let gas = FilecoinGas {
        limit: 1_526_328,
        fee_cap: 101_213,
        premium: 100_022,
    };
    let terms = |sender, recipient, value, method, gas| FilecoinTransferTerms {
        sender,
        recipient,
        value,
        method,
        nonce: 1,
        gas,
    };
    let gas_with = |limit, fee_cap, premium| FilecoinGas {
        limit,
        fee_cap,
        premium,
    };
    for (sender, recipient, value, method, gas) in [
        // Only a key's own account signs.
        (f410, recipient, 1, METHOD_SEND, gas),
        (f0, recipient, 1, METHOD_SEND, gas),
        // A key's account takes a bare send, an Ethereum-style one InvokeEVM.
        (sender, recipient, 1, METHOD_INVOKE_EVM, gas),
        (sender, f410, 1, METHOD_SEND, gas),
        (sender, f0, 1, 2, gas),
        (sender, recipient, 0, METHOD_SEND, gas),
        // Gas the network refuses, and more than 0.07 FIL of it.
        (sender, recipient, 1, METHOD_SEND, gas_with(0, 1, 0)),
        (
            sender,
            recipient,
            1,
            METHOD_SEND,
            gas_with(10_000_000_001, 1, 0),
        ),
        (sender, recipient, 1, METHOD_SEND, gas_with(1, 0, 0)),
        (sender, recipient, 1, METHOD_SEND, gas_with(1, 1, 2)),
        (
            sender,
            recipient,
            1,
            METHOD_SEND,
            gas_with(1_000_000, 70_000_000_001, 0),
        ),
        (
            sender,
            "t1qode47ievxlxzk6z2viuovedabmn3tq6t57uqhq",
            1,
            METHOD_SEND,
            gas,
        ),
    ] {
        assert!(
            prepare_transfer(
                Chain::Filecoin,
                terms(sender, recipient, value, method, gas)
            )
            .is_err(),
            "{sender} {recipient} {value} {method} {gas:?}"
        );
    }
    let at_most = gas_with(1_000_000, 70_000_000_000, 0);
    let built = prepare_transfer(
        Chain::Filecoin,
        terms(sender, f0, 5, METHOD_INVOKE_EVM, at_most),
    )
    .unwrap();
    assert_eq!(built.maximum_fee(), Some(MAX_FEE));
    assert_eq!(built.maximum_cost(), Some(MAX_FEE + 5));
}

#[test]
fn a_transfer_calls_the_method_its_recipient_takes_value_with() {
    use crate::derivation::filecoin::FilecoinProtocol::*;
    assert_eq!(transfer_method(Secp256k1, false), METHOD_SEND);
    assert_eq!(transfer_method(Bls, false), METHOD_SEND);
    assert_eq!(transfer_method(Delegated, false), METHOD_INVOKE_EVM);
    assert_eq!(transfer_method(Id, false), METHOD_SEND);
    assert_eq!(transfer_method(Id, true), METHOD_INVOKE_EVM);
    assert_eq!(transfer_method(Actor, true), METHOD_INVOKE_EVM);
}

#[test]
fn amounts_are_big_integers_of_their_fewest_bytes() {
    assert_eq!(big_int(0), Vec::<u8>::new());
    assert_eq!(big_int(1), vec![0, 1]);
    assert_eq!(big_int(256), vec![0, 1, 0]);
    assert_eq!(big_int(u128::MAX), [vec![0], vec![0xff; 16]].concat());
}
