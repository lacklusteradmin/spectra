//! PSBTs against bitcoinjs-lib's (`multisig-psbt.json`, from
//! scripts/generate-multisig-psbt-vectors.cjs).

use super::*;
use std::str::FromStr;

struct Case {
    policy: MultisigPolicy,
    vector: serde_json::Value,
    phrases: Vec<String>,
}

fn case() -> Case {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/multisig-psbt.json")).unwrap();
    let network = &fixture["networks"][0];
    let keys: Vec<String> = network["cosigners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "[{}/{}]{}/<0;1>/*",
                c["fingerprint"].as_str().unwrap(),
                c["origin"].as_str().unwrap(),
                c["xpub"].as_str().unwrap()
            )
        })
        .collect();
    let descriptor = format!("wsh(sortedmulti(2,{}))", keys.join(","));
    Case {
        policy: MultisigPolicy::parse(Chain::Bitcoin, &descriptor).unwrap(),
        vector: network["psbt"].clone(),
        phrases: fixture["phrases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect(),
    }
}

impl Case {
    fn psbt(&self, key: &str) -> Psbt {
        decode(self.vector[key].as_str().unwrap()).unwrap()
    }
    fn account(&self, cosigner: usize) -> Xpriv {
        self.policy
            .cosigner_of_phrase(Chain::Bitcoin, &self.phrases[cosigner], "")
            .unwrap()
            .1
    }
}

/// bitcoinjs-lib's PSBT reads as the account's: both inputs at their
/// places, the payment and the change, the fee; the one built here from the
/// same coins is the same transaction.
#[test]
fn a_bitcoinjs_psbt_reviews_as_the_accounts_and_builds_identically() {
    let case = case();
    let reviewed = review(&case.policy, Chain::Bitcoin, &case.psbt("unsigned")).unwrap();
    let places: Vec<Place> = reviewed.inputs.iter().map(|input| input.place).collect();
    assert_eq!(places, vec![(0, 0), (1, 0)]);
    assert_eq!(reviewed.inputs[0].value, 100_000);
    assert_eq!(
        reviewed.outputs[0].address,
        case.vector["recipient"]["address"]
    );
    assert_eq!(reviewed.outputs[0].change, None);
    assert_eq!(reviewed.outputs[1].change, Some((1, 1)));
    assert_eq!(reviewed.fee, 1000);
    assert!(!reviewed.complete);
    assert_eq!(reviewed.txid, case.vector["txid"].as_str().unwrap());

    let inputs: Vec<(OutPoint, u64, Place)> = case.vector["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| {
            (
                OutPoint::from_str(&format!(
                    "{}:{}",
                    input["txid"].as_str().unwrap(),
                    input["vout"]
                ))
                .unwrap(),
                input["value"].as_u64().unwrap(),
                (
                    input["branch"].as_u64().unwrap() as u32,
                    input["index"].as_u64().unwrap() as u32,
                ),
            )
        })
        .collect();
    let recipient = Address::from_str(case.vector["recipient"]["address"].as_str().unwrap())
        .unwrap()
        .assume_checked()
        .script_pubkey();
    let built = build(
        &case.policy,
        &inputs,
        &[(recipient, 120_000)],
        Some(((1, 1), 29_000)),
    )
    .unwrap();
    assert_eq!(
        hex::encode(bitcoin::consensus::serialize(&built.unsigned_tx)),
        case.vector["unsigned_tx"].as_str().unwrap()
    );
    assert_eq!(
        review(&case.policy, Chain::Bitcoin, &built).unwrap(),
        reviewed
    );
}

/// Signed here as A, combined with bitcoinjs-lib's signature as B in either
/// order, the finished transaction is bitcoinjs-lib's byte for byte, and
/// the size estimate bounds it.
#[test]
fn a_and_b_finish_bitcoinjs_transaction_byte_for_byte() {
    let case = case();
    let mut first = case.psbt("unsigned");
    let signed = sign(&case.policy, Chain::Bitcoin, &mut first, &case.account(0)).unwrap();
    assert_eq!(signed.signed_by(), vec![0]);
    assert!(finalize(&case.policy, Chain::Bitcoin, &first).is_err());
    let combined = combine(
        &case.policy,
        Chain::Bitcoin,
        &mut first,
        case.psbt("signed_by_b"),
    )
    .unwrap();
    assert!(combined.complete);
    assert_eq!(combined.signed_by(), vec![0, 1]);
    let mut second = case.psbt("signed_by_b");
    sign(&case.policy, Chain::Bitcoin, &mut second, &case.account(0)).unwrap();
    for psbt in [first, second] {
        let tx = finalize(&case.policy, Chain::Bitcoin, &psbt).unwrap();
        assert_eq!(
            hex::encode(bitcoin::consensus::serialize(&tx)),
            case.vector["final_a_b"].as_str().unwrap()
        );
        let estimate = estimate_vsize(
            &case.policy,
            2,
            tx.output.iter().map(|o| o.script_pubkey.len()),
        );
        assert!(
            estimate >= tx.vsize() as u64 && estimate <= tx.vsize() as u64 + 2,
            "{estimate} {}",
            tx.vsize()
        );
    }
}

/// An input that pays another script or names no key origin of the
/// account, a witness script another place's, an output passing off a
/// payment as change, a signature that does not verify, and a copy of
/// another transaction are refused; nothing is signed over them.
#[test]
fn foreign_inputs_mismatched_scripts_and_changed_transactions_are_refused() {
    let case = case();
    let refused = |psbt: &Psbt, expected: &str| {
        let error = review(&case.policy, Chain::Bitcoin, psbt)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(expected), "{error}");
        let mut copy = psbt.clone();
        assert!(sign(&case.policy, Chain::Bitcoin, &mut copy, &case.account(0)).is_err());
    };
    let mut foreign = case.psbt("unsigned");
    foreign.inputs[0]
        .witness_utxo
        .as_mut()
        .unwrap()
        .script_pubkey = script_at(&case.policy, (0, 5)).unwrap().0;
    refused(&foreign, "script is not the wallet's");
    let mut unnamed = case.psbt("unsigned");
    unnamed.inputs[1].bip32_derivation.clear();
    refused(&unnamed, "does not belong to this wallet");
    let mut mismatched = case.psbt("unsigned");
    mismatched.inputs[0].witness_script = Some(case.policy.witness_script((0, 1)).unwrap());
    refused(&mismatched, "script is not the wallet's");
    let mut false_change = case.psbt("unsigned");
    false_change.outputs[0].bip32_derivation = false_change.outputs[1].bip32_derivation.clone();
    refused(&false_change, "claims to be the wallet's change");
    let mut forged = case.psbt("signed_by_b");
    let (key, signature) = forged.inputs[0]
        .partial_sigs
        .iter()
        .next()
        .map(|(k, s)| (*k, *s))
        .unwrap();
    let other = case.psbt("signed_by_b").inputs[1]
        .partial_sigs
        .values()
        .next()
        .copied()
        .unwrap();
    assert_ne!(signature, other);
    forged.inputs[0].partial_sigs.insert(key, other);
    refused(&forged, "not a valid one");

    let mut ours = case.psbt("unsigned");
    let mut changed = case.psbt("signed_by_b");
    changed.unsigned_tx.output[0].value = Amount::from_sat(119_000);
    let error = combine(&case.policy, Chain::Bitcoin, &mut ours, changed)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("another transaction"), "{error}");
    assert!(
        review(&case.policy, Chain::Bitcoin, &ours)
            .unwrap()
            .signed_by()
            .is_empty()
    );
}
