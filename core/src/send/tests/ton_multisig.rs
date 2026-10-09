//! TON multisig v2 data, orders and messages against @ton/core's
//! (`ton-multisig.json`, from scripts/generate-ton-multisig-vectors.cjs).

use super::*;
use crate::derivation::ton::cell_from_boc;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/ton-multisig.json")).unwrap()
}

fn cell(value: &serde_json::Value) -> Cell {
    cell_from_boc(&hex::decode(value["boc_hex"].as_str().unwrap()).unwrap()).unwrap()
}

fn hash(value: &serde_json::Value) -> String {
    value["hash"].as_str().unwrap().to_string()
}

fn raw(text: &str) -> TonAccount {
    let (workchain, account) = text.split_once(':').unwrap();
    (
        workchain.parse().unwrap(),
        hex::decode(account).unwrap().try_into().unwrap(),
    )
}

fn transfer(fixture: &serde_json::Value) -> TonTransferOrder {
    let order = &fixture["order"];
    TonTransferOrder {
        destination: raw(order["destination"]["raw"].as_str().unwrap()),
        bounce: order["destination"]["bounce"].as_bool().unwrap(),
        nanotons: order["value"].as_str().unwrap().parse().unwrap(),
        comment: order["comment"].as_str().map(str::to_string),
    }
}

/// The multisig's data reads as its signers, threshold and next number;
/// order addresses derive as the contract derives them.
#[test]
fn multisig_data_and_order_addresses_are_the_contracts() {
    let fixture = fixture();
    let signers: Vec<TonAccount> = fixture["signers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|signer| raw(signer["raw"].as_str().unwrap()))
        .collect();
    for (name, next) in [("data", 0), ("data_at_next_order_seqno", 5)] {
        let data = cell(&fixture["multisig"][name]);
        assert_eq!(
            hex::encode(data.hash_depth().0),
            hash(&fixture["multisig"][name])
        );
        assert_eq!(
            MultisigData::parse(&data).unwrap(),
            MultisigData {
                next_order_seqno: next,
                threshold: 2,
                signers: signers.clone(),
                proposers: 1,
                allow_arbitrary_seqno: false,
            }
        );
    }
    let multisig = raw(fixture["multisig"]["raw"].as_str().unwrap());
    for vector in fixture["order_addresses"].as_array().unwrap() {
        assert_eq!(
            order_address(&multisig, vector["seqno"].as_u64().unwrap()).unwrap(),
            raw(vector["raw"].as_str().unwrap())
        );
    }
    let fixed = &fixture["fixed_signers"];
    for vector in fixed["order_addresses"].as_array().unwrap() {
        assert_eq!(
            order_address(
                &raw(fixed["raw"].as_str().unwrap()),
                vector["seqno"].as_u64().unwrap()
            )
            .unwrap(),
            raw(vector["raw"].as_str().unwrap())
        );
    }
}

/// The order, its proposal and an approval are @ton/core's; the order
/// decodes back, and another one does not pass for a transfer.
#[test]
fn orders_and_bodies_encode_as_ton_core_does() {
    let fixture = fixture();
    let transfer = transfer(&fixture);
    let order = transfer.order().unwrap();
    assert_eq!(
        hex::encode(order.hash_depth().0),
        hash(&fixture["order"]["cell"])
    );
    assert!(cell(&fixture["order"]["cell"]) == order);
    assert_eq!(TonTransferOrder::decode(&order).unwrap(), transfer);
    let signer = &fixture["new_order"]["signer"];
    let proposal = new_order(
        signer["query_id"].as_u64().unwrap(),
        signer["order_seqno"].as_u64().unwrap(),
        signer["index"].as_u64().unwrap() as u8,
        fixture["order"]["expiration_date"].as_u64().unwrap(),
        order.clone(),
    )
    .unwrap();
    assert_eq!(hex::encode(proposal.hash_depth().0), hash(signer));
    let vector = &fixture["approve"];
    let approval = approve(
        vector["query_id"].as_u64().unwrap(),
        vector["signer_index"].as_u64().unwrap() as u8,
    )
    .unwrap();
    assert_eq!(hex::encode(approval.hash_depth().0), hash(vector));

    let plain = TonTransferOrder {
        comment: None,
        bounce: true,
        ..transfer.clone()
    };
    assert_eq!(
        TonTransferOrder::decode(&plain.order().unwrap()).unwrap(),
        plain
    );
    let long = TonTransferOrder {
        comment: Some("x".repeat(300)),
        ..transfer.clone()
    };
    assert_eq!(
        TonTransferOrder::decode(&long.order().unwrap()).unwrap(),
        long
    );
    // A proposal is no order; nor is a transfer under another key.
    assert!(TonTransferOrder::decode(&proposal).is_err());
    let mut action = Cell::default();
    action.reference(order.clone()).unwrap();
    let other_key = crate::derivation::ton_cell::single_entry_dictionary(8, 1, action).unwrap();
    assert!(TonTransferOrder::decode(&other_key).is_err());
}

/// An order's state reads as the contract keeps it: uninitialized, then
/// with its approvals by index.
#[test]
fn order_state_reads_its_approvals() {
    let fixture = fixture();
    let multisig = raw(fixture["multisig"]["raw"].as_str().unwrap());
    let signers: Vec<TonAccount> = fixture["signers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|signer| raw(signer["raw"].as_str().unwrap()))
        .collect();
    let order = transfer(&fixture).order().unwrap();
    let mut head = Cell::default();
    head.address(multisig.0, &multisig.1)
        .unwrap()
        .uint(0, 64)
        .unwrap()
        .uint(0, 64)
        .unwrap()
        .uint(0, 64)
        .unwrap()
        .uint(5, 64)
        .unwrap();
    assert_eq!(OrderState::parse(&head, &multisig, 5).unwrap(), None);
    assert!(OrderState::parse(&head, &multisig, 6).is_err());
    let signers_cell = cell(&fixture["multisig"]["signers_cell"]);
    let mut state = head.clone();
    state
        .uint(2, 8)
        .unwrap()
        .uint(0, 1)
        .unwrap()
        .reference(signers_cell)
        .unwrap()
        .uint(0, 64)
        .unwrap()
        .uint(0, 64)
        .unwrap()
        .uint(0, 64)
        .unwrap()
        .uint(0b101, 64)
        .unwrap()
        .uint(2, 8)
        .unwrap()
        .uint(1_762_592_000, 48)
        .unwrap()
        .reference(order.clone())
        .unwrap();
    assert_eq!(
        OrderState::parse(&state, &multisig, 5).unwrap(),
        Some(OrderState {
            threshold: 2,
            sent_for_execution: false,
            signers,
            approvals: vec![0, 2],
            expiration: 1_762_592_000,
            order,
        })
    );
}

/// The multisig's code (`build/Multisig.compiled.json` of
/// multisig-contract-v2 9a4b13df, its Order code a library reference)
/// hashes to the code hash an account must carry.
#[test]
fn multisig_code_hashes_to_the_contracts() {
    let code = cell_from_boc(include_bytes!(
        "../../../tests/fixtures/ton-multisig-code.boc"
    ))
    .unwrap();
    assert_eq!(hex::encode(code.hash_depth().0), MULTISIG_CODE_HASH);
}
