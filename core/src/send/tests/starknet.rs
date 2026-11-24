use super::*;
use crate::api::starknet_json_rpc::ResourceBound;

fn vectors() -> Vec<serde_json::Value> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/starknet-vectors.json"
    ))
    .unwrap();
    fixture["transfers"].as_array().unwrap().clone()
}

fn felt(text: &str) -> Felt {
    parse_felt(text).unwrap()
}

fn quantity(value: &serde_json::Value) -> u128 {
    u128::from_str_radix(value.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

fn bounds(value: &serde_json::Value) -> ResourceBounds {
    let bound = |name: &str| ResourceBound {
        max_amount: u64::try_from(quantity(&value[name]["max_amount"])).unwrap(),
        max_price_per_unit: quantity(&value[name]["max_price_per_unit"]),
    };
    ResourceBounds {
        l1_gas: bound("l1_gas"),
        l2_gas: bound("l2_gas"),
        l1_data_gas: bound("l1_data_gas"),
    }
}

fn prepared(vector: &serde_json::Value) -> PreparedStarknetTransfer {
    prepare_transfer(
        Chain::Starknet,
        StarknetTransferTerms {
            sender: vector["sender"].as_str().unwrap(),
            token: vector["token"].as_str().unwrap(),
            recipient: vector["recipient"].as_str().unwrap(),
            amount: vector["amount"].as_str().unwrap().parse().unwrap(),
            nonce: vector["nonce"].as_u64().unwrap(),
            resource_bounds: bounds(&vector["resource_bounds"]),
            deployment: vector["deployment"].as_object().map(|deployment| {
                (
                    felt(vector["public_key"].as_str().unwrap()),
                    bounds(&deployment["resource_bounds"]),
                )
            }),
        },
    )
    .unwrap()
}

fn key(vector: &serde_json::Value) -> Felt {
    felt(vector["private_key"].as_str().unwrap())
}

fn strings(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect()
}

fn rpc(felts: &[Felt]) -> Vec<String> {
    felts
        .iter()
        .map(crate::api::starknet_json_rpc::rpc_felt)
        .collect()
}

#[test]
fn transfers_hash_and_sign_as_starknet_js_signs_them() {
    for vector in vectors() {
        let name = vector["name"].as_str().unwrap();
        let transfer = prepared(&vector);
        assert_eq!(
            rpc(&transfer.calldata()),
            strings(&vector["calldata"]),
            "{name}"
        );
        assert_eq!(
            crate::derivation::starknet::felt_hex(&transfer.invoke_hash()),
            vector["transaction_hash"].as_str().unwrap(),
            "{name}"
        );
        let signed = transfer.sign(Chain::Starknet, &key(&vector)).unwrap();
        assert_eq!(
            rpc(&signed.invoke.signature),
            strings(&vector["signature"]),
            "{name}"
        );
        // The request body, field for field, as starknet.js's RPC channel
        // writes it.
        assert_eq!(
            signed.invoke.request_body(false),
            vector["invoke_transaction"],
            "{name}"
        );
        match vector["deployment"].as_object() {
            None => {
                assert!(signed.deploy_account.is_none(), "{name}");
                assert!(transfer.deploy_hash().is_none());
            }
            Some(deployment) => {
                let signed_deployment = signed.deploy_account.as_ref().unwrap();
                assert_eq!(
                    crate::derivation::starknet::felt_hex(&signed_deployment.hash),
                    deployment["transaction_hash"].as_str().unwrap(),
                    "{name}"
                );
                assert_eq!(
                    rpc(&signed_deployment.transaction.signature),
                    strings(&deployment["signature"])
                );
                assert_eq!(
                    signed_deployment.transaction.request_body(false),
                    deployment["deploy_account_transaction"],
                    "{name}"
                );
            }
        }
    }
}

#[test]
fn the_most_both_transactions_can_cost_is_their_bounds_together() {
    let vector = &vectors()[1];
    let transfer = prepared(vector);
    let deployment = transfer.deployment.as_ref().unwrap();
    let sum = |bounds: &ResourceBounds| {
        [bounds.l1_gas, bounds.l2_gas, bounds.l1_data_gas]
            .iter()
            .map(|bound| u128::from(bound.max_amount) * bound.max_price_per_unit)
            .sum::<u128>()
    };
    assert_eq!(
        transfer.max_fee().unwrap(),
        sum(&transfer.resource_bounds) + sum(&deployment.resource_bounds)
    );
    let mut overflowing = transfer.clone();
    overflowing.resource_bounds.l2_gas = ResourceBound {
        max_amount: u64::MAX,
        max_price_per_unit: u128::MAX,
    };
    assert!(overflowing.max_fee().is_err());
}

#[test]
fn a_key_for_another_account_or_network_is_refused() {
    let vectors = vectors();
    let transfer = prepared(&vectors[0]);
    // The key of the account it pays.
    assert!(transfer.sign(Chain::Starknet, &key(&vectors[1])).is_err());
    assert!(transfer.sign(Chain::Ethereum, &key(&vectors[0])).is_err());
    let mut moved = transfer.clone();
    moved.chain_id = "SN_SEPOLIA".into();
    assert!(moved.sign(Chain::Starknet, &key(&vectors[0])).is_err());
    // A deployment naming another key than the one that signs.
    let mut deploying = prepared(&vectors[1]);
    deploying.deployment.as_mut().unwrap().public_key =
        crate::derivation::starknet::public_key(&key(&vectors[0]));
    assert!(deploying.sign(Chain::Starknet, &key(&vectors[1])).is_err());
    // A deployment of another class would deploy another account.
    let mut other_class = prepared(&vectors[1]);
    other_class.deployment.as_mut().unwrap().class_hash =
        felt("0x029927c8af6bccf3f6fda035981e765a7bdbf18a2dc0d630494f8758aa908e2b");
    assert!(
        other_class
            .sign(Chain::Starknet, &key(&vectors[1]))
            .is_err()
    );
}

#[test]
fn a_transfer_the_network_would_refuse_is_not_built() {
    let vectors = vectors();
    let vector = &vectors[1];
    let sender = vector["sender"].as_str().unwrap();
    let token = vector["token"].as_str().unwrap();
    let recipient = vector["recipient"].as_str().unwrap();
    let own_key = felt(vector["public_key"].as_str().unwrap());
    let other_key = felt(vectors[0]["public_key"].as_str().unwrap());
    let terms = |recipient, amount, nonce, deployment| StarknetTransferTerms {
        sender,
        token,
        recipient,
        amount,
        nonce,
        resource_bounds: ResourceBounds::ZERO,
        deployment,
    };
    for (recipient, amount, nonce, deployment) in [
        // Zero, an address past the bound, an EIP-55 address whose case is
        // not the Starknet checksum.
        (recipient, 0, 1, None),
        (
            "0x0800000000000000000000000000000000000000000000000000000000000000",
            1,
            1,
            None,
        ),
        ("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed", 1, 1, None),
        // A deployment whose key does not own the sender, and a transfer
        // behind a deployment at a nonce other than one.
        (recipient, 1, 1, Some((other_key, ResourceBounds::ZERO))),
        (recipient, 1, 0, Some((own_key, ResourceBounds::ZERO))),
        (recipient, 1, 2, Some((own_key, ResourceBounds::ZERO))),
    ] {
        assert!(
            prepare_transfer(Chain::Starknet, terms(recipient, amount, nonce, deployment)).is_err(),
            "{recipient} {amount} {nonce}"
        );
    }
    assert!(
        prepare_transfer(
            Chain::Starknet,
            terms(recipient, 1, 1, Some((own_key, ResourceBounds::ZERO)))
        )
        .is_ok()
    );
    assert!(prepare_transfer(Chain::Cardano, terms(recipient, 1, 1, None)).is_err());
}

#[test]
fn an_estimate_carries_zero_bounds_and_a_signature_on_the_curve() {
    let vector = &vectors()[1];
    let transfer = prepared(vector);
    let (deploy, invoke) = transfer.unsigned(ResourceBounds::ZERO, ResourceBounds::ZERO);
    let body = invoke.request_body(true);
    assert_eq!(body["version"], "0x100000000000000000000000000000003");
    assert_eq!(body["nonce"], "0x1");
    assert_eq!(body["resource_bounds"]["l2_gas"]["max_amount"], "0x0");
    assert_eq!(body["signature"].as_array().unwrap().len(), 2);
    let deploy = deploy.unwrap().request_body(true);
    assert_eq!(deploy["version"], "0x100000000000000000000000000000003");
    assert_eq!(deploy["contract_address_salt"], body_salt(vector));
}

fn body_salt(vector: &serde_json::Value) -> String {
    crate::api::starknet_json_rpc::rpc_felt(&felt(vector["public_key"].as_str().unwrap()))
}
