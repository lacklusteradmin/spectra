use super::*;

fn vectors() -> Vec<serde_json::Value> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/algorand-vectors.json"
    ))
    .unwrap();
    fixture["transfers"].as_array().unwrap().clone()
}

fn number(vector: &serde_json::Value, field: &str) -> u64 {
    vector[field].as_str().unwrap().parse().unwrap()
}

fn terms(vector: &serde_json::Value) -> AlgorandPaymentTerms<'_> {
    AlgorandPaymentTerms {
        sender: vector["sender"].as_str().unwrap(),
        receiver: vector["receiver"].as_str().unwrap(),
        amount: vector["amount"].as_str().unwrap().parse().unwrap(),
        fee: number(vector, "fee"),
        first_valid: number(vector, "first_valid"),
        last_valid: number(vector, "last_valid"),
    }
}

fn key(vector: &serde_json::Value) -> AlgorandKey {
    match vector["kind"].as_str().unwrap() {
        "mnemonic" => AlgorandKey::from_bytes(
            &*crate::derivation::algorand::seed_from_mnemonic(vector["mnemonic"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap(),
        _ => {
            AlgorandKey::from_bytes(&hex::decode(vector["extended_key"].as_str().unwrap()).unwrap())
                .unwrap()
        }
    }
}

#[test]
fn payments_encode_sign_and_name_as_algosdk_does() {
    for vector in vectors() {
        let txid = vector["txid"].as_str().unwrap();
        let payment = prepare_payment(Chain::Algorand, terms(&vector)).unwrap();
        assert_eq!(payment.genesis_id, vector["genesis_id"].as_str().unwrap());
        assert_eq!(
            hex::encode(payment.transaction_bytes()),
            vector["transaction"].as_str().unwrap(),
            "{txid}"
        );
        assert_eq!(
            hex::encode(payment.bytes_to_sign()),
            vector["bytes_to_sign"].as_str().unwrap()
        );
        assert_eq!(payment.txid(), txid);
        let (signed, signed_txid) = payment.sign(Chain::Algorand, &key(&vector)).unwrap();
        assert_eq!(
            hex::encode(signed),
            vector["signed"].as_str().unwrap(),
            "{txid}"
        );
        assert_eq!(signed_txid, txid);
    }
}

#[test]
fn a_fee_per_byte_is_priced_as_algosdk_prices_it() {
    let mut priced = 0;
    for vector in vectors().iter().filter(|v| v["fee_per_byte"].is_string()) {
        let fee = fee_for(
            Chain::Algorand,
            terms(vector),
            number(vector, "fee_per_byte"),
            number(vector, "min_fee"),
        )
        .unwrap();
        assert_eq!(fee, number(vector, "fee"), "{}", vector["txid"]);
        priced += 1;
    }
    // One above the minimum, one at it.
    assert_eq!(priced, 2);
}

#[test]
fn a_payment_the_network_would_refuse_is_not_built() {
    let vector = &vectors()[0];
    let base = || terms(vector);
    for (why, terms) in [
        (
            "zero amount",
            AlgorandPaymentTerms {
                amount: 0,
                ..base()
            },
        ),
        (
            "past u64",
            AlgorandPaymentTerms {
                amount: u128::from(u64::MAX) + 1,
                ..base()
            },
        ),
        ("no fee", AlgorandPaymentTerms { fee: 0, ..base() }),
        (
            "round zero",
            AlgorandPaymentTerms {
                first_valid: 0,
                ..base()
            },
        ),
        (
            "window past 1,000 rounds",
            AlgorandPaymentTerms {
                last_valid: base().first_valid + 1_001,
                ..base()
            },
        ),
        (
            "window ending before it starts",
            AlgorandPaymentTerms {
                last_valid: base().first_valid - 1,
                ..base()
            },
        ),
        (
            "another network's address",
            AlgorandPaymentTerms {
                receiver: "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7",
                ..base()
            },
        ),
    ] {
        assert!(prepare_payment(Chain::Algorand, terms).is_err(), "{why}");
    }
    assert!(
        prepare_payment(
            Chain::Algorand,
            AlgorandPaymentTerms {
                last_valid: base().first_valid + 1_000,
                ..base()
            }
        )
        .is_ok()
    );
    assert!(prepare_payment(Chain::Ethereum, base()).is_err());
}

#[test]
fn a_key_for_another_account_or_network_is_refused() {
    let vectors = vectors();
    let payment = prepare_payment(Chain::Algorand, terms(&vectors[0])).unwrap();
    // Another account's mnemonic, and an ARC-52 key that is not the sender's.
    assert!(payment.sign(Chain::Algorand, &key(&vectors[1])).is_err());
    assert!(payment.sign(Chain::Algorand, &key(&vectors[3])).is_err());
    assert!(AlgorandKey::from_bytes(&[7; 33]).is_err());
    let mut moved = payment.clone();
    moved.genesis_hash[0] ^= 1;
    assert!(moved.sign(Chain::Algorand, &key(&vectors[0])).is_err());
    let mut renamed = payment.clone();
    renamed.genesis_id = "testnet-v1.0".into();
    assert!(renamed.sign(Chain::Algorand, &key(&vectors[0])).is_err());
}
