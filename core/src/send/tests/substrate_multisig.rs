//! Substrate multisig accounts and approvals against polkadot.js's
//! (`substrate-multisig.json`, from
//! scripts/generate-substrate-multisig-vectors.cjs).

use super::*;
use crate::derivation::substrate_multisig::SubstrateMultisig;
use crate::registry::Chain;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/substrate-multisig.json"
    ))
    .unwrap()
}

fn bytes32(value: &serde_json::Value) -> [u8; 32] {
    hex::decode(value.as_str().unwrap().trim_start_matches("0x"))
        .unwrap()
        .try_into()
        .unwrap()
}

fn unhex(value: &serde_json::Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap().trim_start_matches("0x")).unwrap()
}

fn policy(fixture: &serde_json::Value, chain: Chain, threshold: u16) -> SubstrateMultisig {
    let id = if chain == Chain::Polkadot {
        "polkadot"
    } else {
        "substrate"
    };
    let signatories: Vec<_> = fixture["signatories"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .map(|signatory| signatory[id].clone())
        .collect();
    SubstrateMultisig::parse(
        chain,
        &serde_json::json!({"threshold": threshold, "signatories": signatories}).to_string(),
    )
    .unwrap()
}

/// The account of each threshold is polkadot.js's, at each network's
/// prefix, whatever order its signatories are named in; refused: a
/// threshold of one or above the signatories, a signatory twice, another
/// network's address.
#[test]
fn accounts_derive_as_polkadot_js_does() {
    let fixture = fixture();
    for threshold in [2u16, 3] {
        let vector = &fixture["multisig"][format!("threshold_{threshold}")];
        for (chain, id) in [
            (Chain::Polkadot, "polkadot"),
            (Chain::Bittensor, "substrate"),
        ] {
            let policy = policy(&fixture, chain, threshold);
            assert_eq!(policy.account_id(), bytes32(&vector["account_id"]));
            assert_eq!(policy.address(), vector[id].as_str().unwrap());
            assert_eq!(
                SubstrateMultisig::parse(chain, &policy.canonical()).unwrap(),
                policy
            );
        }
    }
    let sorted: Vec<[u8; 32]> = fixture["sorted_order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|index| {
            bytes32(&fixture["signatories"][index.as_u64().unwrap() as usize]["public_key"])
        })
        .collect();
    assert_eq!(policy(&fixture, Chain::Polkadot, 2).signatories, sorted);
    let address = |index: usize, id: &str| {
        fixture["signatories"][index][id]
            .as_str()
            .unwrap()
            .to_string()
    };
    for (threshold, signatories) in [
        (1, vec![address(0, "polkadot"), address(1, "polkadot")]),
        (3, vec![address(0, "polkadot"), address(1, "polkadot")]),
        (2, vec![address(0, "polkadot"), address(0, "polkadot")]),
        (2, vec![address(0, "polkadot"), address(1, "substrate")]),
    ] {
        let text =
            serde_json::json!({"threshold": threshold, "signatories": signatories}).to_string();
        assert!(
            SubstrateMultisig::parse(Chain::Polkadot, &text).is_err(),
            "{text}"
        );
    }
}

/// The transfer, its hash and every approval are polkadot.js's on Asset Hub
/// and on Bittensor; the transfer decodes back, and nothing else does.
#[test]
fn calls_encode_as_polkadot_js_does() {
    let fixture = fixture();
    let max_weight = (
        fixture["max_weight"]["ref_time"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        fixture["max_weight"]["proof_size"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    let timepoint = (
        fixture["timepoint"]["height"].as_u64().unwrap() as u32,
        fixture["timepoint"]["index"].as_u64().unwrap() as u32,
    );
    for runtime in ["polkadot-asset-hub", "bittensor"] {
        let vector = &fixture["runtimes"][runtime];
        let indices = (
            vector["balances"]["pallet_index"].as_u64().unwrap() as u8,
            vector["balances"]["calls"]["transfer_keep_alive"]
                .as_u64()
                .unwrap() as u8,
        );
        let pallet = MultisigPallet {
            index: vector["multisig"]["pallet_index"].as_u64().unwrap() as u8,
            as_multi: vector["multisig"]["calls"]["as_multi"].as_u64().unwrap() as u8,
            approve_as_multi: vector["multisig"]["calls"]["approve_as_multi"]
                .as_u64()
                .unwrap() as u8,
            deposit_base: 0,
            deposit_factor: 0,
            max_signatories: 100,
        };
        let transfer = MultisigTransfer {
            recipient: bytes32(&vector["inner"]["dest"]),
            amount: vector["inner"]["value"]
                .as_u64()
                .map(u128::from)
                .unwrap_or_else(|| vector["inner"]["value"].as_str().unwrap().parse().unwrap()),
        };
        let call = transfer_call(indices, &transfer);
        assert_eq!(call, unhex(&vector["inner"]["hex"]));
        let hash = call_hash(&call);
        assert_eq!(hash, bytes32(&vector["inner"]["call_hash"]));
        assert_eq!(decode_transfer(&call, indices).unwrap(), transfer);
        let others = |name: &str| -> Vec<[u8; 32]> {
            vector["calls"][name]["other_signatories"]
                .as_array()
                .unwrap()
                .iter()
                .map(bytes32)
                .collect()
        };
        assert_eq!(
            as_multi(
                &pallet,
                2,
                &others("as_multi_first"),
                None,
                &call,
                max_weight
            ),
            unhex(&vector["calls"]["as_multi_first"]["hex"])
        );
        assert_eq!(
            as_multi(
                &pallet,
                2,
                &others("as_multi_final"),
                Some(timepoint),
                &call,
                max_weight
            ),
            unhex(&vector["calls"]["as_multi_final"]["hex"])
        );
        // The vector caps the weight an approval by hash never dispatches;
        // Spectra's caps none.
        let mut approve = unhex(&vector["calls"]["approve_as_multi_first"]["hex"]);
        approve.truncate(approve.len() - 8);
        approve.extend([0, 0]);
        assert_eq!(
            approve_as_multi(&pallet, 2, &others("approve_as_multi_first"), None, &hash),
            approve
        );

        let mut other = call.clone();
        other[0] ^= 1;
        assert!(decode_transfer(&other, indices).is_err());
        let mut longer = call.clone();
        longer.push(0);
        assert!(decode_transfer(&longer, indices).is_err());
        assert!(
            decode_transfer(
                &transfer_call(
                    indices,
                    &MultisigTransfer {
                        amount: 0,
                        ..transfer
                    }
                ),
                indices
            )
            .is_err()
        );
    }
}
