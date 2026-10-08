use super::*;

fn params() -> CardanoProtocolParams {
    CardanoProtocolParams {
        fee_per_byte: 44,
        fee_fixed: 155_381,
        coins_per_utxo_byte: 4310,
        max_tx_size: 16_384,
        max_value_size: 5000,
    }
}

fn test_key() -> ([u8; 64], [u8; 32]) {
    let (key, public, _) = crate::derivation::cardano::derive_cardano_base_keys(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "",
        "m/1852'/1815'/0'/0/0",
    )
    .unwrap();
    (key, public)
}

/// A mainnet address's bech32 from its bytes.
fn address(bytes_hex: &str) -> String {
    bech32::encode::<bech32::Bech32>(
        bech32::Hrp::parse("addr").unwrap(),
        &hex::decode(bytes_hex).unwrap(),
    )
    .unwrap()
}

fn assets(rows: &serde_json::Value) -> Vec<CardanoAssetAmount> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| CardanoAssetAmount {
            asset: CardanoAssetId::new(
                row["policy"].as_str().unwrap(),
                row["name"].as_str().unwrap(),
            )
            .unwrap()
            .identifier(),
            quantity: row["quantity"].as_str().unwrap().parse().unwrap(),
        })
        .collect()
}

fn number(value: &serde_json::Value) -> u64 {
    value
        .as_str()
        .map(|text| text.parse().unwrap())
        .unwrap_or_else(|| value.as_u64().unwrap())
}

#[test]
fn extended_witness_matches_independent_emurgo_transaction() {
    let vector: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/cardano-emurgo-witness.json"
    ))
    .unwrap();
    let (key, public) = test_key();
    assert_eq!(hex::encode(key), vector["privateKey"]);
    assert_eq!(hex::encode(public), vector["publicKey"]);
    let to = vector["address"].as_str().unwrap().to_string();
    let transaction = PreparedCardanoTransaction {
        inputs: vec![CardanoInput {
            tx_hash: "00".repeat(32),
            tx_index: 0,
            lovelace: 1_170_000,
            assets: vec![],
        }],
        outputs: vec![CardanoOutput {
            address: to,
            lovelace: 1_000_000,
            assets: vec![],
        }],
        fee: 170_000,
        ttl: 100,
    };
    assert_eq!(
        hex::encode(transaction.encode_body().unwrap()),
        vector["body"]
    );
    let hash = transaction.transaction_hash().unwrap();
    assert_eq!(hex::encode(hash), vector["hash"]);
    let signature = sign_extended(&key, &public, &hash).unwrap();
    assert_eq!(hex::encode(signature), vector["signature"]);
    ed25519_dalek::VerifyingKey::from_bytes(&public)
        .unwrap()
        .verify_strict(&hash, &ed25519_dalek::Signature::from_bytes(&signature))
        .unwrap();
    assert_eq!(
        transaction.sign(&key, &public).unwrap(),
        vector["transaction"]
    );
    assert!(sign_extended(&key, &[0; 32], &hash).is_err());
    let mut altered = key;
    altered[0] |= 1;
    assert!(sign_extended(&altered, &public, &hash).is_err());
}

/// Bodies, signed transactions, each output's minimum ADA and the minimum
/// fee, as the Cardano Serialization Library computes them.
#[test]
fn native_asset_transactions_match_csl() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/cardano-assets.json")).unwrap();
    let (key, public) = test_key();
    for case in fixture["cases"].as_array().unwrap() {
        let transaction = PreparedCardanoTransaction {
            inputs: case["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| CardanoInput {
                    tx_hash: input["tx_hash"].as_str().unwrap().into(),
                    tx_index: input["tx_index"].as_u64().unwrap() as u32,
                    lovelace: number(&input["lovelace"]),
                    assets: assets(&input["assets"]),
                })
                .collect(),
            outputs: case["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|output| CardanoOutput {
                    address: address(output["address"].as_str().unwrap()),
                    lovelace: number(&output["lovelace"]),
                    assets: assets(&output["assets"]),
                })
                .collect(),
            fee: number(&case["fee"]),
            ttl: number(&case["ttl"]),
        };
        let name = &case["name"];
        assert_eq!(
            hex::encode(transaction.encode_body().unwrap()),
            case["body"],
            "{name}"
        );
        assert_eq!(
            transaction.sign(&key, &public).unwrap(),
            case["transaction"],
            "{name}"
        );
        assert_eq!(
            transaction.minimum_fee(&params()).unwrap(),
            number(&case["min_fee"]),
            "{name}"
        );
        for (output, expected) in transaction
            .outputs
            .iter()
            .zip(case["outputs"].as_array().unwrap())
        {
            assert_eq!(
                minimum_ada(&output.address, &output.assets, &params()).unwrap(),
                number(&expected["min_ada"]),
                "{name}"
            );
        }
        transaction.conserves().unwrap();
    }
}

const SENDER: &str = "addr1vy8ac7qqy0vtulyl7wntmsxc6wex80gvcyjy33qffrhm7ss7lxrqp";

fn recipient() -> String {
    address(&format!("61{}", "22".repeat(28)))
}

fn token(name: &str) -> String {
    CardanoAssetId::new(&"aa".repeat(28), name)
        .unwrap()
        .identifier()
}

fn utxo(n: u8, lovelace: u64, held: &[(&str, u64)]) -> CardanoInput {
    CardanoInput {
        tx_hash: format!("{n:02x}").repeat(32),
        tx_index: 0,
        lovelace,
        assets: held
            .iter()
            .map(|(name, quantity)| CardanoAssetAmount {
                asset: token(name),
                quantity: *quantity,
            })
            .collect(),
    }
}

fn plan(
    utxos: &[CardanoInput],
    transfer: CardanoTransfer,
) -> Result<PreparedCardanoTransaction, SendError> {
    PreparedCardanoTransaction::plan(utxos, &params(), SENDER, &recipient(), &transfer, 100)
}

/// ADA held beside tokens can pay: the token-bearing input is spent and its
/// tokens come back as change, with the ADA to carry them.
#[test]
fn ada_sends_spend_token_bearing_inputs_and_return_their_tokens() {
    let utxos = [utxo(1, 3_000_000, &[("01", 5)]), utxo(2, 1_200_000, &[])];
    let transaction = plan(&utxos, CardanoTransfer::Ada(2_000_000)).unwrap();
    transaction.conserves().unwrap();
    assert_eq!(transaction.outputs[0].lovelace, 2_000_000);
    let change = &transaction.outputs[1];
    assert_eq!(change.address, SENDER);
    assert_eq!(
        change.assets,
        vec![CardanoAssetAmount {
            asset: token("01"),
            quantity: 5
        }]
    );
    assert!(change.lovelace >= minimum_ada(SENDER, &change.assets, &params()).unwrap());
    assert!(transaction.fee >= transaction.minimum_fee(&params()).unwrap());
    // ADA alone comes first; tokens are spent only when it is not enough.
    let enough = [utxo(1, 3_000_000, &[("01", 5)]), utxo(2, 9_000_000, &[])];
    let transaction = plan(&enough, CardanoTransfer::Ada(2_000_000)).unwrap();
    assert_eq!(transaction.inputs.len(), 1);
    assert!(transaction.inputs[0].assets.is_empty());
}

/// A token travels with its minimum ADA; the rest of every asset the inputs
/// held comes back.
#[test]
fn token_sends_carry_their_minimum_ada_and_conserve_assets() {
    let utxos = [
        utxo(1, 1_500_000, &[("01", 100), ("02", 7)]),
        utxo(2, 2_000_000, &[]),
        utxo(3, 1_400_000, &[("01", 30)]),
    ];
    let asset = CardanoAssetId::parse(&token("01")).unwrap();
    let transaction = plan(
        &utxos,
        CardanoTransfer::Asset {
            asset: asset.clone(),
            quantity: 120,
        },
    )
    .unwrap();
    transaction.conserves().unwrap();
    let sent = &transaction.outputs[0];
    assert_eq!(
        sent.assets,
        vec![CardanoAssetAmount {
            asset: token("01"),
            quantity: 120
        }]
    );
    assert_eq!(
        sent.lovelace,
        minimum_ada(&recipient(), &sent.assets, &params()).unwrap()
    );
    let change = &transaction.outputs[1];
    assert_eq!(
        change.assets,
        vec![
            CardanoAssetAmount {
                asset: token("01"),
                quantity: 10
            },
            CardanoAssetAmount {
                asset: token("02"),
                quantity: 7
            }
        ]
    );
    let terms = transaction.terms(1).unwrap();
    assert_eq!(
        (terms.debited.as_str(), terms.received.as_str()),
        ("12", "12")
    );
    assert_eq!(
        terms.carried_native,
        Some(crate::decimal::from_units(u128::from(sent.lovelace), 6))
    );
    // More than is held, and enough tokens without the ADA to carry them.
    let error = plan(
        &utxos,
        CardanoTransfer::Asset {
            asset: asset.clone(),
            quantity: 131,
        },
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("Insufficient token balance"),
        "{error}"
    );
    let poor = [utxo(1, 1_100_000, &[("01", 100)])];
    let error = plan(
        &poor,
        CardanoTransfer::Asset {
            asset,
            quantity: 50,
        },
    )
    .unwrap_err();
    assert!(matches!(error, SendError::InsufficientFunds(_)), "{error}");
}

/// Change too small to stand as an output joins the fee, and an ADA output
/// below the protocol minimum is refused.
#[test]
fn dust_joins_the_fee_and_small_outputs_are_refused() {
    let utxos = [utxo(1, 2_500_000, &[])];
    let transaction = plan(&utxos, CardanoTransfer::Ada(2_000_000)).unwrap();
    assert_eq!(transaction.outputs.len(), 1);
    assert_eq!(transaction.fee, 500_000);
    transaction.conserves().unwrap();
    let error = plan(&utxos, CardanoTransfer::Ada(800_000)).unwrap_err();
    assert!(error.to_string().contains("at least"), "{error}");
    assert!(plan(&utxos, CardanoTransfer::Ada(2_400_000)).is_err());
}

/// A changed value no longer balances: what signing refuses.
#[test]
fn an_unbalanced_transaction_is_refused() {
    let utxos = [utxo(1, 3_000_000, &[("01", 5)])];
    let mut transaction = plan(&utxos, CardanoTransfer::Ada(1_000_000)).unwrap();
    transaction.outputs[1].assets.clear();
    assert!(transaction.conserves().is_err());
    let (key, public) = test_key();
    assert!(transaction.sign(&key, &public).is_err());
    let mut transaction = plan(&utxos, CardanoTransfer::Ada(1_000_000)).unwrap();
    transaction.fee -= 1;
    assert!(transaction.clone().within_limits(&params()).is_err());
}
