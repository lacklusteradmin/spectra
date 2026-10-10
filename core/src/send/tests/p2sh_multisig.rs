//! P2SH multisig spends on Bitcoin Cash and Dogecoin against bitcoinjs-lib's,
//! bitcore-lib-cash's and ecash-lib's (`p2sh-multisig.json`, from
//! scripts/generate-p2sh-multisig-vectors.cjs).

use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/p2sh-multisig.json")).unwrap()
}

fn network<'a>(fixture: &'a serde_json::Value, chain: &str) -> &'a serde_json::Value {
    fixture["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|network| network["chain"] == chain)
        .unwrap()
}

fn policy(network: &serde_json::Value, chain: Chain) -> MultisigPolicy {
    let keys: Vec<String> = network["cosigners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cosigner| {
            format!(
                "[{}/{}]{}/<0;1>/*",
                cosigner["fingerprint"].as_str().unwrap(),
                cosigner["origin"].as_str().unwrap(),
                cosigner["xpub"].as_str().unwrap()
            )
        })
        .collect();
    MultisigPolicy::parse(
        chain,
        &format!(
            "sh(sortedmulti({},{}))",
            network["threshold"],
            keys.join(",")
        ),
    )
    .unwrap()
}

fn place(value: &serde_json::Value) -> Place {
    (
        value["branch"].as_u64().unwrap() as u32,
        value["index"].as_u64().unwrap() as u32,
    )
}

fn spend(
    policy: &MultisigPolicy,
    chain: Chain,
    vector: &serde_json::Value,
    inputs: &serde_json::Value,
) -> P2shSpend {
    let inputs: Vec<(OutPoint, u64, Place)> = inputs
        .as_array()
        .unwrap()
        .iter()
        .map(|input| {
            (
                OutPoint {
                    txid: input["txid"].as_str().unwrap().parse().unwrap(),
                    vout: input["vout"].as_u64().unwrap() as u32,
                },
                input["value"].as_u64().unwrap(),
                place(input),
            )
        })
        .collect();
    let recipient = &vector["recipient"];
    build(
        policy,
        chain,
        &inputs,
        &[(
            ScriptBuf::from(hex::decode(recipient["script_pubkey"].as_str().unwrap()).unwrap()),
            recipient["value"].as_u64().unwrap(),
        )],
        Some((
            place(&vector["change"]),
            vector["change"]["value"].as_u64().unwrap(),
        )),
    )
    .unwrap()
}

fn cosigners(
    fixture: &serde_json::Value,
    policy: &MultisigPolicy,
    chain: Chain,
) -> Vec<(usize, Xpriv)> {
    fixture["phrases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|phrase| {
            policy
                .cosigner_of_phrase(chain, phrase.as_str().unwrap(), "")
                .unwrap()
        })
        .collect()
}

/// Every network's addresses are the vectors', on each branch.
#[test]
fn addresses_are_the_vectors() {
    let fixture = fixture();
    for (id, chain) in [
        ("bitcoin-cash", Chain::BitcoinCash),
        ("bitcoin-cash-testnet", Chain::BitcoinCashTestnet),
        ("dogecoin", Chain::Dogecoin),
        ("dogecoin-testnet", Chain::DogecoinTestnet),
    ] {
        let network = network(&fixture, id);
        let policy = policy(network, chain);
        for address in network["addresses"].as_array().unwrap() {
            assert_eq!(
                policy.address(chain, place(address)).unwrap(),
                address["address"].as_str().unwrap(),
                "{id}"
            );
            assert_eq!(
                hex::encode(policy.witness_script(place(address)).unwrap().as_bytes()),
                address["redeem_script"].as_str().unwrap()
            );
        }
    }
}

/// Each spend's transaction, digests and RFC 6979 signatures are the
/// vectors'; Bitcoin Cash's PSBTs are BCHN's byte for byte and read back,
/// Dogecoin's partially signed transactions Dogecoin Core's, padded or not;
/// the finished transactions are the vectors'.
#[test]
fn spends_sign_and_travel_as_each_network_does() {
    let fixture = fixture();
    for (id, chain) in [
        ("bitcoin-cash", Chain::BitcoinCash),
        ("dogecoin", Chain::Dogecoin),
    ] {
        let network = network(&fixture, id);
        let vector = &network["transaction"];
        let policy = policy(network, chain);
        let unsigned = spend(&policy, chain, vector, &vector["inputs"]);
        assert_eq!(
            hex::encode(bitcoin::consensus::serialize(&unsigned.transaction)),
            vector["unsigned_tx"].as_str().unwrap(),
            "{id}"
        );
        for (index, input) in vector["inputs"].as_array().unwrap().iter().enumerate() {
            let redeem = policy.witness_script(place(input)).unwrap();
            assert_eq!(
                hex::encode(
                    sighash(
                        chain,
                        &unsigned.transaction,
                        index,
                        &redeem,
                        input["value"].as_u64().unwrap()
                    )
                    .unwrap()
                ),
                input["sighash"].as_str().unwrap()
            );
        }
        let keys = cosigners(&fixture, &policy, chain);
        let signed = |order: &[usize]| {
            let mut spend = unsigned.clone();
            for phrase in order {
                let (cosigner, account) = &keys[*phrase];
                sign(&policy, chain, &mut spend, *cosigner, account).unwrap();
            }
            spend
        };
        for (index, input) in vector["inputs"].as_array().unwrap().iter().enumerate() {
            let all = signed(&[0, 1, 2]);
            for signature in input["signatures"].as_array().unwrap() {
                let cosigner = signature["cosigner"].as_u64().unwrap() as usize;
                assert_eq!(
                    all.inputs[index].signatures[&keys[cosigner].0],
                    signature["signature"].as_str().unwrap()
                );
            }
        }
        let by_p1 = signed(&[1]);
        let both = signed(&[1, 0]);
        let holdings_inputs = unsigned.inputs.clone();
        let outpoints: Vec<OutPoint> = unsigned
            .transaction
            .input
            .iter()
            .map(|input| input.previous_output)
            .collect();
        let unspent = |outpoint: &OutPoint| {
            outpoints
                .iter()
                .position(|own| own == outpoint)
                .map(|index| (holdings_inputs[index].value, holdings_inputs[index].place))
        };
        let change_script = policy.script_pubkey(place(&vector["change"])).unwrap();
        let change_place = place(&vector["change"]);
        let change = |script: &ScriptBuf| (*script == change_script).then_some(change_place);
        let holdings = Holdings {
            unspent: &unspent,
            change: &change,
        };
        if chain == Chain::BitcoinCash {
            assert_eq!(
                encode(&policy, chain, &unsigned).unwrap(),
                vector["psbt"]["unsigned"].as_str().unwrap()
            );
            assert_eq!(
                encode(&policy, chain, &by_p1).unwrap(),
                vector["psbt"]["signed_by_p1"].as_str().unwrap()
            );
            assert_eq!(
                decode(
                    &policy,
                    chain,
                    vector["psbt"]["signed_by_p1"].as_str().unwrap(),
                    &holdings
                )
                .unwrap(),
                by_p1
            );
            let previous = &vector["full_previous"];
            let full = spend(&policy, chain, vector, &previous["inputs"]);
            for text in [
                &previous["psbt_unsigned"],
                &previous["psbt_unsigned_ctxout"],
            ] {
                assert_eq!(
                    decode(&policy, chain, text.as_str().unwrap(), &holdings).unwrap(),
                    full
                );
            }
            let mut full_signed = full.clone();
            for phrase in [0, 1] {
                let (cosigner, account) = &keys[phrase];
                sign(&policy, chain, &mut full_signed, *cosigner, account).unwrap();
            }
            assert_eq!(
                hex::encode(bitcoin::consensus::serialize(
                    &finalize(&policy, &full_signed).unwrap()
                )),
                previous["final_p0_p1"].as_str().unwrap()
            );
        } else {
            assert_eq!(
                encode(&policy, chain, &by_p1).unwrap(),
                vector["signed_by_p1"].as_str().unwrap()
            );
            assert_eq!(
                encode(&policy, chain, &signed(&[0])).unwrap(),
                vector["signed_by_p0"].as_str().unwrap()
            );
            for text in [&vector["signed_by_p1"], &vector["signed_by_p1_padded"]] {
                assert_eq!(
                    decode(&policy, chain, text.as_str().unwrap(), &holdings).unwrap(),
                    by_p1
                );
            }
            assert_eq!(
                decode(
                    &policy,
                    chain,
                    &encode(&policy, chain, &unsigned).unwrap(),
                    &holdings
                )
                .unwrap(),
                unsigned
            );
        }
        let finished = finalize(&policy, &both).unwrap();
        assert_eq!(
            hex::encode(bitcoin::consensus::serialize(&finished)),
            vector["final_p0_p1"].as_str().unwrap()
        );
        let review = review(&policy, chain, &both).unwrap();
        assert!(review.complete && review.fee == vector["fee"].as_u64().unwrap());
        assert_eq!(review.txid, vector["txid"].as_str().unwrap());
        assert!(!super::review(&policy, chain, &by_p1).unwrap().complete);
        let mut joined = by_p1.clone();
        combine(&policy, chain, &mut joined, signed(&[0])).unwrap();
        assert_eq!(finalize(&policy, &joined).unwrap(), finished);
        assert!(sign(&policy, chain, &mut joined, keys[1].0, &keys[1].1).is_err());
        let estimate = estimate_size(
            &policy,
            finished.input.len(),
            finished
                .output
                .iter()
                .map(|output| output.script_pubkey.len())
                .collect(),
        );
        assert!(estimate >= bitcoin::consensus::serialize(&finished).len());
        // Bitcoin Cash's digest commits to the amount; Dogecoin's does not,
        // which is why its amounts come from the network.
        let mut tampered = by_p1.clone();
        tampered.inputs[0].value += 1;
        assert_eq!(
            super::review(&policy, chain, &tampered).is_err(),
            chain == Chain::BitcoinCash
        );
    }
}

/// A Dogecoin transaction carries neither its inputs' amounts nor their
/// places, which the wallet supplies from what it holds: an input that is
/// not among the wallet's unspent outputs is refused, and so is one whose
/// scriptSig carries another script than the wallet's at the place the
/// wallet holds that output.
#[test]
fn a_dogecoin_spend_of_coins_the_wallet_does_not_hold_there_is_refused() {
    let fixture = fixture();
    let network = network(&fixture, "dogecoin");
    let vector = &network["transaction"];
    let policy = policy(network, Chain::Dogecoin);
    let unsigned = spend(&policy, Chain::Dogecoin, vector, &vector["inputs"]);
    let held: Vec<(OutPoint, u64, Place)> = unsigned
        .transaction
        .input
        .iter()
        .zip(&unsigned.inputs)
        .map(|(txin, input)| (txin.previous_output, input.value, input.place))
        .collect();
    let signed_by_p1 = vector["signed_by_p1"].as_str().unwrap();
    let no_change = |_: &ScriptBuf| None;
    let read = |unspent: &dyn Fn(&OutPoint) -> Option<(u64, Place)>| {
        decode(
            &policy,
            Chain::Dogecoin,
            signed_by_p1,
            &Holdings {
                unspent,
                change: &no_change,
            },
        )
        .map(drop)
        .map_err(|error| error.to_string())
    };
    let holding = |outpoint: &OutPoint| {
        held.iter()
            .find(|(own, _, _)| own == outpoint)
            .map(|(_, value, place)| (*value, *place))
    };
    assert_eq!(read(&holding), Ok(()));
    let first_not_held = |outpoint: &OutPoint| {
        (*outpoint != held[0].0)
            .then(|| holding(outpoint))
            .flatten()
    };
    assert_eq!(
        read(&first_not_held),
        Err("Input 0 is not one of this wallet's unspent outputs.".into())
    );
    let first_elsewhere = |outpoint: &OutPoint| {
        holding(outpoint).map(|(value, place)| {
            (
                value,
                if *outpoint == held[0].0 {
                    (0, 1)
                } else {
                    place
                },
            )
        })
    };
    assert_eq!(
        read(&first_elsewhere),
        Err("Input 0's script is not the wallet's script at its place.".into())
    );
}
