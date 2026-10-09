//! Cardano native-script spends against the Cardano Serialization
//! Library's (`cardano-multisig.json`, from
//! scripts/generate-cardano-multisig-vectors.cjs).

use super::*;
use crate::registry::Chain;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/cardano-multisig.json"
    ))
    .unwrap()
}

fn script(fixture: &serde_json::Value, name: &str) -> NativeScript {
    NativeScript::parse(&fixture["scripts"][name]["json"].to_string()).unwrap()
}

fn number(value: &serde_json::Value) -> u64 {
    value
        .as_u64()
        .unwrap_or_else(|| value.as_str().unwrap().parse().unwrap())
}

/// Every script's CBOR, hash and addresses are CSL's; each phrase's
/// CIP-1854 key is its cosigner's.
#[test]
fn scripts_and_cosigner_keys_are_csls() {
    let fixture = fixture();
    for name in ["s1", "s2", "s3"] {
        let script = script(&fixture, name);
        let vector = &fixture["scripts"][name];
        assert_eq!(
            hex::encode(script.cbor()),
            vector["cbor"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(hex::encode(script.hash()), vector["hash"].as_str().unwrap());
        for (chain, id) in [
            (Chain::Cardano, "cardano"),
            (Chain::CardanoPreprod, "cardano-preprod"),
        ] {
            assert_eq!(
                script.address(chain).unwrap(),
                vector["addresses"][id]["address"].as_str().unwrap()
            );
        }
        assert_eq!(NativeScript::parse(&script.canonical()).unwrap(), script);
    }
    for cosigner in fixture["cosigners"].as_array().unwrap() {
        let keys =
            crate::derivation::cardano::cosigner_keys(cosigner["phrase"].as_str().unwrap(), "", 1)
                .unwrap();
        assert_eq!(
            hex::encode(*keys[0].0),
            cosigner["private_key"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(keys[0].1),
            cosigner["public_key"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(keys[0].2),
            cosigner["key_hash"].as_str().unwrap()
        );
    }
    let s2 = script(&fixture, "s2");
    assert_eq!(s2.validity(), (None, Some(90_000_000)));
    for text in [
        r#"{"type": "sig", "keyHash": "00"}"#,
        r#"{"type": "atLeast", "required": 3, "scripts": [{"type": "sig", "keyHash": "32000809732f4d5b9a53e2eec996e7c79a24963068ffbf3565318e9a"}]}"#,
        r#"{"type": "after", "slot": 5}"#,
        r#"{"type": "mofn"}"#,
    ] {
        assert!(NativeScript::parse(text).is_err(), "{text}");
    }
}

/// Each spend's body, hash, witnesses and signed transactions are CSL's,
/// built and signed here; the transactions decode back, and the script is
/// satisfied by its signers and only then.
#[test]
fn spends_sign_and_assemble_as_csl_does() {
    let fixture = fixture();
    let keys: Vec<_> = fixture["cosigners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cosigner| {
            crate::derivation::cardano::cosigner_keys(cosigner["phrase"].as_str().unwrap(), "", 1)
                .unwrap()
                .remove(0)
        })
        .collect();
    for vector in fixture["transactions"].as_array().unwrap() {
        let script = script(&fixture, vector["script"].as_str().unwrap());
        let prepared = PreparedCardanoTransaction {
            inputs: vector["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| CardanoInput {
                    tx_hash: input["tx_hash"].as_str().unwrap().into(),
                    tx_index: input["tx_index"].as_u64().unwrap() as u32,
                    lovelace: number(&input["lovelace"]),
                    assets: Vec::new(),
                })
                .collect(),
            outputs: vector["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|output| CardanoOutput {
                    address: output["address"].as_str().unwrap().into(),
                    lovelace: number(&output["lovelace"]),
                    assets: Vec::new(),
                })
                .collect(),
            fee: number(&vector["fee"]),
            ttl: number(&vector["ttl"]),
            script: Some(spend(&script, None)),
        };
        assert_eq!(
            hex::encode(prepared.body().unwrap()),
            vector["body"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(prepared.transaction_hash().unwrap()),
            vector["hash"].as_str().unwrap()
        );
        let hash = prepared.transaction_hash().unwrap();
        for signed in vector["signed"].as_array().unwrap() {
            let witnesses: Vec<Witness> = signed["signers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| {
                    let index = name.as_str().unwrap()[1..].parse::<usize>().unwrap();
                    let (private, public, _) = &keys[index];
                    let signature =
                        crate::send::cardano::sign_extended(private, public, &hash).unwrap();
                    assert_eq!(
                        hex::encode(signature),
                        vector["witnesses"][name.as_str().unwrap()]["signature"]
                            .as_str()
                            .unwrap()
                    );
                    (*public, signature)
                })
                .collect();
            assert_eq!(
                hex::encode(prepared.witness_set(&witnesses).unwrap()),
                signed["witness_set"].as_str().unwrap()
            );
            let raw = prepared.encode_signed(&witnesses).unwrap();
            assert_eq!(hex::encode(&raw), signed["transaction"].as_str().unwrap());
            let decoded = decode(&raw, &script).unwrap();
            assert_eq!(decoded.witnesses, witnesses);
            assert_eq!(
                decoded.prepared(&script, &prepared.inputs, &raw).unwrap(),
                prepared
            );
            let keys = signed_keys(&prepared, &script, &witnesses).unwrap();
            assert_eq!(
                script.satisfied(&keys, None, prepared.ttl),
                witnesses.len() >= 2,
                "{:?}",
                signed["signers"]
            );
        }
    }
}

/// Refused: another script's transaction, an input the account does not
/// hold, a witness over another body, a key twice; and a `before` script is
/// not satisfied past its slot.
#[test]
fn foreign_spends_and_witnesses_are_refused() {
    let fixture = fixture();
    let vector = &fixture["transactions"][0];
    let s1 = script(&fixture, "s1");
    let raw = hex::decode(vector["signed"][0]["transaction"].as_str().unwrap()).unwrap();
    assert!(decode(&raw, &script(&fixture, "s3")).is_err());
    let decoded = decode(&raw, &s1).unwrap();
    assert!(decoded.prepared(&s1, &[], &raw).is_err());
    let held = vec![CardanoInput {
        tx_hash: "aa".repeat(32),
        tx_index: 0,
        lovelace: 10_000_000,
        assets: Vec::new(),
    }];
    let prepared = decoded.prepared(&s1, &held, &raw).unwrap();
    let mut other = prepared.clone();
    other.fee += 1;
    assert!(signed_keys(&other, &s1, &decoded.witnesses).is_err());
    let twice = vec![decoded.witnesses[0], decoded.witnesses[0]];
    assert!(signed_keys(&prepared, &s1, &twice).is_err());
    let mut tampered = raw.clone();
    *tampered.last_mut().unwrap() = 0xf5;
    assert!(decode(&tampered, &s1).is_err());
    let s2 = script(&fixture, "s2");
    let both = s2.key_hashes();
    assert!(s2.satisfied(&both, None, 90_000_000));
    assert!(!s2.satisfied(&both, None, 90_000_001));
}
