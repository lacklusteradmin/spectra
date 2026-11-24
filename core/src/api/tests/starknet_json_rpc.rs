use super::*;

/// What PublicNode's node (spec 0.10.3) answered on 2026-11-24 to a
/// simulated STRK transfer from a deployed Ready account.
fn live_estimate() -> Value {
    json!({
        "l1_gas_consumed": "0x0", "l1_gas_price": "0x396eef64a169",
        "l2_gas_consumed": "0x187540", "l2_gas_price": "0x6bc235206",
        "l1_data_gas_consumed": "0xc0", "l1_data_gas_price": "0xdb0b15d8e",
        "overall_fee": "0xa4c33f62e76a00", "unit": "FRI",
    })
}

#[test]
fn an_estimate_becomes_bounds_with_half_again_on_every_amount_and_price() {
    let estimate = fee_estimate(&live_estimate()).unwrap();
    assert_eq!(estimate.l2_gas, (0x187540, 0x6bc235206));
    let bounds = estimate.bounds(50).unwrap();
    assert_eq!(bounds.l1_gas.max_amount, 0);
    assert_eq!(bounds.l1_gas.max_price_per_unit, 0x396eef64a169 * 3 / 2);
    assert_eq!(bounds.l2_gas.max_amount, 0x187540 * 3 / 2);
    assert_eq!(bounds.l2_gas.max_price_per_unit, 0x6bc235206 * 3 / 2);
    assert_eq!(bounds.l1_data_gas.max_amount, 0xc0 * 3 / 2);
    // What the node said the transaction costs, and the most its bounds
    // let it charge: 2.25 times as much where nothing rounds.
    let overall = 0x187540u128 * 0x6bc235206 + 0xc0 * 0xdb0b15d8e;
    assert_eq!(overall, 0xa4c33f62e76a00);
    let ceiling = bounds.max_fee().unwrap();
    assert!(
        ceiling >= overall * 9 / 4 - 1 && ceiling <= overall * 9 / 4 + 1,
        "{ceiling}"
    );
    // A fee in another unit is ETH's, which a v3 transaction does not pay.
    let mut wei = live_estimate();
    wei["unit"] = json!("WEI");
    assert!(fee_estimate(&wei).is_err());
    let mut missing = live_estimate();
    missing["l2_gas_price"] = Value::Null;
    assert!(fee_estimate(&missing).is_err());
}

#[test]
fn a_status_is_confirmed_once_a_block_accepted_it() {
    for (answer, expected) in [
        (
            json!({"finality_status": "ACCEPTED_ON_L2", "execution_status": "SUCCEEDED"}),
            TransactionStatus::Confirmed {
                succeeded: true,
                block: None,
            },
        ),
        (
            json!({"finality_status": "ACCEPTED_ON_L1", "execution_status": "REVERTED", "failure_reason": "u256_sub Overflow"}),
            TransactionStatus::Confirmed {
                succeeded: false,
                block: None,
            },
        ),
        (
            json!({"finality_status": "REJECTED"}),
            TransactionStatus::Confirmed {
                succeeded: false,
                block: None,
            },
        ),
        (
            json!({"finality_status": "RECEIVED"}),
            TransactionStatus::Pending,
        ),
        (
            json!({"finality_status": "CANDIDATE"}),
            TransactionStatus::Pending,
        ),
        (
            json!({"finality_status": "PRE_CONFIRMED", "execution_status": "SUCCEEDED"}),
            TransactionStatus::Pending,
        ),
    ] {
        assert_eq!(transaction_status(&answer).unwrap(), expected, "{answer}");
    }
    assert!(transaction_status(&json!({"finality_status": "ACCEPTED_ON_L2"})).is_err());
    assert!(transaction_status(&json!({"finality_status": "LOST"})).is_err());
}

#[test]
fn an_error_object_is_an_answer_with_its_code() {
    // As PublicNode answered a counterfactual account's class on 2026-11-24.
    let missing = decode(
        "starknet_getClassHashAt",
        json!({"jsonrpc": "2.0", "error": {"code": 20, "message": "Contract not found"}, "id": 1}),
    )
    .unwrap()
    .unwrap_err();
    assert_eq!(missing.code, CONTRACT_NOT_FOUND);
    let refused = decode(
        "starknet_addInvokeTransaction",
        json!({"error": {"code": 55, "message": "Account validation failed", "data": "argent/invalid-owner-sig"}}),
    )
    .unwrap()
    .unwrap_err();
    assert!(
        refusal("starknet_addInvokeTransaction", &refused)
            .to_string()
            .contains("argent/invalid-owner-sig")
    );
    assert_eq!(
        decode("starknet_chainId", json!({"result": "0x534e5f4d41494e"}))
            .unwrap()
            .unwrap(),
        json!("0x534e5f4d41494e")
    );
    assert!(decode("starknet_chainId", json!({"id": 1})).is_err());
}

#[test]
fn a_u256_balance_is_read_up_to_u128() {
    let felt = |value: u128| Felt::from(value);
    // STRK held by a Ready account, as `balanceOf` answered.
    assert_eq!(
        u256_value(&[felt(0x319a5ebe1faa6d759d), Felt::ZERO]).unwrap(),
        0x319a5ebe1faa6d759d
    );
    assert_eq!(
        u256_value(&[felt(u128::MAX), Felt::ZERO]).unwrap(),
        u128::MAX
    );
    assert!(u256_value(&[Felt::ZERO, Felt::ONE]).is_err());
    assert!(u256_value(&[Felt::from_bytes_be_slice(&[1; 17]), Felt::ZERO]).is_err());
    assert!(u256_value(&[Felt::ONE]).is_err());
}

#[test]
fn a_felt_goes_on_the_wire_without_leading_zeros() {
    assert_eq!(rpc_felt(&Felt::ZERO), "0x0");
    assert_eq!(rpc_felt(&Felt::from(0x0abcu64)), "0xabc");
    assert_eq!(
        rpc_felt(
            &parse_felt("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d")
                .unwrap()
        ),
        "0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"
    );
}

#[tokio::test]
async fn a_node_on_another_network_is_refused() {
    let server = crate::service::send_stage_support::node(|_, body| {
        assert_eq!(body["method"], "starknet_chainId");
        // SN_SEPOLIA.
        Some(json!({"jsonrpc": "2.0", "id": 1, "result": "0x534e5f5345504f4c4941"}))
    })
    .await;
    let client = StarknetClient::new(Arc::new(vec![server.uri()]));
    assert!(client.verify_network(Chain::Starknet).await.is_err());
    assert!(client.verify_network(Chain::Ethereum).await.is_err());
}
