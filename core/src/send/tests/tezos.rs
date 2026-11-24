use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/tezos-vectors.json")).unwrap()
}

fn number(value: &serde_json::Value) -> u64 {
    value.as_str().unwrap().parse().unwrap()
}

fn limits(content: &serde_json::Value) -> TezosLimits {
    TezosLimits {
        fee: number(&content["fee"]),
        counter: number(&content["counter"]),
        gas_limit: number(&content["gas_limit"]),
        storage_limit: number(&content["storage_limit"]),
    }
}

/// The prepared transfer a vector's operations are, and the key its reveal
/// names, if it has one.
fn prepared(operation: &serde_json::Value) -> (PreparedTezosTransfer, Option<[u8; 32]>) {
    let contents = operation["contents"].as_array().unwrap();
    let transaction = contents.last().unwrap();
    assert_eq!(transaction["kind"], "transaction");
    let reveal = (contents.len() == 2).then(|| &contents[0]);
    let public_key = reveal.map(|reveal| {
        let key = reveal["public_key"].as_str().unwrap();
        decode_base58check(key, &prefix::EDPK, 32)
            .unwrap()
            .try_into()
            .unwrap()
    });
    let transfer = PreparedTezosTransfer {
        chain_id: "NetXdQprcVkpaWU".into(),
        protocol: "PsUshuai9QapM5TGj1JpuVGkdxz5GykdnEvS6Rh8SUVrARvZLCY".into(),
        branch: operation["branch"].as_str().unwrap().into(),
        expires_at_level: 1,
        source: transaction["source"].as_str().unwrap().into(),
        destination: transaction["destination"].as_str().unwrap().into(),
        amount: u128::from(number(&transaction["amount"])),
        reveal: reveal.map(limits),
        transaction: limits(transaction),
    };
    (transfer, public_key)
}

fn account_key(fixture: &serde_json::Value, address: &str) -> Ed25519Seed {
    let account = fixture["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["address"] == address)
        .unwrap();
    Ed25519Seed::from_hex(account["private_key"].as_str().unwrap()).unwrap()
}

#[test]
fn operations_forge_sign_and_hash_as_taquito_does() {
    let fixture = fixture();
    for operation in fixture["operations"].as_array().unwrap() {
        let name = operation["name"].as_str().unwrap();
        let (transfer, public_key) = prepared(operation);
        let key = account_key(&fixture, &transfer.source);
        let public_key = public_key.unwrap_or([0; 32]);
        assert_eq!(
            public_key == key.public_key(),
            transfer.reveal.is_some(),
            "{name}"
        );
        assert_eq!(
            hex::encode(transfer.forge(&key.public_key()).unwrap()),
            operation["forged"].as_str().unwrap(),
            "{name}"
        );
        let (signed, hash) = transfer.sign(Chain::Tezos, &key).unwrap();
        assert_eq!(
            hex::encode(&signed),
            operation["signed"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(hash, operation["hash"].as_str().unwrap(), "{name}");
    }
}

/// Operations the network included: forged again with the signature they
/// carry, each hashes to its own operation hash, and the Ed25519 one
/// verifies over the watermarked digest this module signs.
#[test]
fn mainnet_operations_forge_to_their_own_hashes() {
    let fixture = fixture();
    for operation in fixture["onchain"].as_array().unwrap() {
        let expected = operation["hash"].as_str().unwrap();
        let (transfer, public_key) = prepared(operation);
        let forged = transfer.forge(&public_key.unwrap_or([0; 32])).unwrap();
        assert_eq!(
            hex::encode(&forged),
            operation["forged"].as_str().unwrap(),
            "{expected}"
        );
        let signature = hex::decode(operation["signature_hex"].as_str().unwrap()).unwrap();
        let mut signed = forged.clone();
        signed.extend_from_slice(&signature);
        assert_eq!(
            base58check(&prefix::OPERATION, &blake2b::<32>(&[&signed])),
            expected
        );
        if let Some(public_key) = public_key {
            use ed25519_dalek::Verifier as _;
            let key = ed25519_dalek::VerifyingKey::from_bytes(&public_key).unwrap();
            let digest = blake2b::<32>(&[&[OPERATION_WATERMARK], &forged]);
            let signature = ed25519_dalek::Signature::from_slice(&signature).unwrap();
            key.verify(&digest, &signature).unwrap();
        }
    }
}

/// Whether `terms`, edited, are refused.
fn refused<'a>(
    mut terms: TezosTransferTerms<'a>,
    edit: impl FnOnce(&mut TezosTransferTerms<'a>),
) -> bool {
    edit(&mut terms);
    prepare_transfer(Chain::Tezos, terms).is_err()
}

fn terms<'a>(source: &'a str, destination: &'a str, reveal: bool) -> TezosTransferTerms<'a> {
    TezosTransferTerms {
        source,
        destination,
        amount: 1_500_000,
        protocol: "PsUshuai9QapM5TGj1JpuVGkdxz5GykdnEvS6Rh8SUVrARvZLCY",
        branch: "BLXuuBECtE1bYReydLQTSD1FgYwmgg6VbEBSRf6nHwxrhdNvff5",
        branch_level: 15_953_140,
        counter: 41,
        reveal_gas_limit: reveal.then_some(271),
        gas_limit: 2_269,
        storage_limit: 257,
    }
}

#[test]
fn a_prepared_transfer_pays_the_minimal_fee_and_its_margin() {
    let fixture = fixture();
    let source = fixture["accounts"][0]["address"].as_str().unwrap();
    let destination = fixture["recipients"]["tz3"].as_str().unwrap();
    let network = Chain::Tezos.tezos_network().unwrap();
    for reveal in [false, true] {
        let transfer = prepare_transfer(Chain::Tezos, terms(source, destination, reveal)).unwrap();
        let gas = 2_269 + if reveal { 271 } else { 0 };
        let size = transfer.forge(&[0; 32]).unwrap().len() as u64 + 64;
        let minimal = network.minimal_fee(gas, size).unwrap();
        assert_eq!(
            transfer.total_fee(),
            u128::from(minimal + 100),
            "reveal {reveal}"
        );
        assert_eq!(transfer.expires_at_level, 15_953_140 + 600);
        assert_eq!(transfer.transaction.counter, 42 + u64::from(reveal));
        assert_eq!(transfer.max_burn(Chain::Tezos).unwrap(), 257 * 250);
        assert_eq!(
            transfer.most_debited(Chain::Tezos).unwrap(),
            1_500_000 + u128::from(minimal + 100) + 257 * 250
        );
        if let Some(revealed) = transfer.reveal {
            assert_eq!(
                (revealed.counter, revealed.gas_limit, revealed.storage_limit),
                (42, 271, 0)
            );
            // The reveal pays for its own gas and its 62 bytes (tag, source,
            // a two-byte fee, counter, gas, storage, the tagged key and the
            // absent proof); the transfer pays the rest.
            assert_eq!(revealed.fee, (271u64 * 100 + 62 * 1_000).div_ceil(1_000));
        }
        // 0.1 mutez a unit of gas and a mutez a byte, over 100 mutez.
        assert_eq!(minimal, 100 + size + gas.div_ceil(10));
    }
}

#[test]
fn a_transfer_the_network_would_refuse_is_not_built() {
    let fixture = fixture();
    let source = fixture["accounts"][0]["address"].as_str().unwrap();
    let tz2 = fixture["recipients"]["tz2"].as_str().unwrap();
    let kt1 = fixture["recipients"]["kt1"].as_str().unwrap();
    let typo = {
        let mut typo = tz2.to_string();
        typo.replace_range(9..10, if &typo[9..10] == "a" { "b" } else { "a" });
        typo
    };
    assert!(prepare_transfer(Chain::Tezos, terms(source, kt1, false)).is_ok());
    let base = || terms(source, tz2, false);
    assert!(refused(base(), |t| t.amount = 0));
    assert!(refused(base(), |t| t.amount = u128::from(u64::MAX)));
    assert!(refused(base(), |t| t.destination = &typo));
    assert!(refused(base(), |t| t.destination = source.split_at(10).0));
    // Only a tz1 account signs here; a contract signs nothing.
    assert!(refused(base(), |t| t.source = tz2));
    assert!(refused(base(), |t| t.source = kt1));
    assert!(refused(base(), |t| {
        t.protocol = "PtTALLiNtPec7mE7yY4m3k26J8Qukef3E3ehzhfXgFZKGtDdAXu"
    }));
    assert!(refused(base(), |t| {
        t.branch = "BLXuuBECtE1bYReydLQTSD1FgYwmgg6VbEBSRf6nHwxrhdNvff6"
    }));
    assert!(refused(base(), |t| t.gas_limit = 0));
    assert!(refused(base(), |t| t.gas_limit = 1_040_001));
    assert!(refused(base(), |t| t.storage_limit = 60_001));
    // A batch's gas cannot exceed a block's.
    assert!(refused(base(), |t| {
        t.gas_limit = 1_040_000;
        t.reveal_gas_limit = Some(1);
    }));
    assert!(prepare_transfer(Chain::Ethereum, terms(source, tz2, false)).is_err());
}

#[test]
fn a_key_for_another_account_or_network_is_refused() {
    let fixture = fixture();
    let operation = &fixture["operations"][0];
    let (transfer, _) = prepared(operation);
    let key = account_key(&fixture, &transfer.source);
    let other = Ed25519Seed::from_hex(&"07".repeat(32)).unwrap();
    assert!(transfer.sign(Chain::Tezos, &other).is_err());
    assert!(transfer.sign(Chain::Solana, &key).is_err());
    let mut moved = transfer.clone();
    moved.chain_id = "NetXnHfVqm9iesp".into();
    assert!(moved.sign(Chain::Tezos, &key).is_err());
    let mut underpaid = transfer.clone();
    underpaid.transaction.fee = 100;
    assert!(underpaid.sign(Chain::Tezos, &key).is_err());
    let mut misordered = prepared(&fixture["operations"][1]).0;
    misordered.transaction.counter += 1;
    let unrevealed = account_key(&fixture, &misordered.source);
    assert!(misordered.sign(Chain::Tezos, &unrevealed).is_err());
}

#[test]
fn a_simulation_names_the_batch_as_it_will_be_signed() {
    let fixture = fixture();
    let (transfer, public_key) = prepared(&fixture["operations"][1]);
    let public_key = public_key.unwrap();
    let contents = transfer.contents(&public_key);
    assert_eq!(contents.len(), 2);
    let TezosContent::Reveal {
        public_key: revealed,
        counter,
        ..
    } = &contents[0]
    else {
        panic!("the reveal leads");
    };
    assert_eq!(
        revealed,
        fixture["accounts"][2]["public_key"].as_str().unwrap()
    );
    assert_eq!(*counter, transfer.reveal.unwrap().counter);
    let TezosContent::Transaction {
        amount,
        destination,
        storage_limit,
        ..
    } = &contents[1]
    else {
        panic!("the transaction follows");
    };
    assert_eq!(
        (u128::from(*amount), destination.as_str(), *storage_limit),
        (transfer.amount, transfer.destination.as_str(), 257)
    );
}
