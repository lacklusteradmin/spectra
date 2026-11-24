use super::*;

const SOURCE: &str = "tz1PpZctTPYj3GjY1B9wtWJh4hgd3XMo1t3R";
const EMPTY: &str = "tz1TYiwwXZ9VkjirUW6e6sdtfTX1cnM5nmfb";

fn transaction(destination: &str) -> TezosContent {
    TezosContent::Transaction {
        source: SOURCE.into(),
        fee: 0,
        counter: 28_987_846,
        gas_limit: 1_040_000,
        storage_limit: 60_000,
        amount: 1,
        destination: destination.into(),
    }
}

/// A mainnet node's simulation of a 1-mutez transfer to an empty account,
/// read 2026-11-24 (protocol PsUshuai…).
fn allocating_answer() -> Value {
    json!({"contents": [{
        "kind": "transaction", "source": SOURCE, "fee": "0", "counter": "28987846",
        "gas_limit": "1040000", "storage_limit": "60000", "amount": "1", "destination": EMPTY,
        "metadata": {"operation_result": {
            "status": "applied",
            "balance_updates": [
                {"kind": "contract", "contract": SOURCE, "change": "-1", "origin": "block"},
                {"kind": "contract", "contract": EMPTY, "change": "1", "origin": "block"},
                {"kind": "contract", "contract": SOURCE, "change": "-64250", "origin": "block"},
                {"kind": "burned", "category": "storage fees", "change": "64250", "origin": "block"},
            ],
            "consumed_milligas": "2168788",
            "allocated_destination_contract": true,
        }},
    }], "signature": simulation_signature()})
}

#[test]
fn a_simulation_reads_the_gas_used_and_the_accounts_created() {
    let used = consumption_from(&allocating_answer(), &[transaction(EMPTY)]).unwrap();
    assert_eq!(
        used,
        [TezosConsumption {
            milligas: 2_168_788,
            paid_storage: 0,
            allocations: 1,
        }]
    );
    assert_eq!(used[0].gas(), 2_169);
}

#[test]
fn a_contract_calls_gas_and_storage_include_its_internal_operations() {
    let answer = json!({"contents": [{
        "kind": "transaction",
        "metadata": {
            "operation_result": {
                "status": "applied", "consumed_milligas": "5100250", "paid_storage_size_diff": "67",
            },
            "internal_operation_results": [{
                "kind": "transaction",
                "result": {
                    "status": "applied", "consumed_milligas": "2100000",
                    "allocated_destination_contract": true,
                },
            }],
        },
    }]});
    let used = consumption_from(
        &answer,
        &[transaction("KT1H5b7LxEExkFd2Tng77TfuWbM5aPvHstPr")],
    )
    .unwrap();
    assert_eq!(
        used,
        [TezosConsumption {
            milligas: 7_200_250,
            paid_storage: 67,
            allocations: 1,
        }]
    );
    assert_eq!(used[0].gas(), 7_201);
}

/// A mainnet node's answer to a transfer of more than the account holds.
#[test]
fn an_operation_the_network_would_not_apply_is_refused_with_its_errors() {
    let answer = json!({"contents": [{
        "kind": "transaction",
        "metadata": {"operation_result": {
            "status": "failed",
            "errors": [
                {"kind": "temporary", "id": "proto.025-PsUshuai.contract.balance_too_low",
                 "contract": SOURCE, "balance": "4412227070176", "amount": "999999999999999"},
                {"kind": "temporary", "id": "proto.025-PsUshuai.tez.subtraction_underflow",
                 "amounts": ["4412227070176", "999999999999999"]},
            ],
        }},
    }]});
    let refused = consumption_from(&answer, &[transaction(EMPTY)]).unwrap_err();
    assert!(matches!(refused, ApiError::Rejected(_)));
    assert!(refused.to_string().contains("balance_too_low"), "{refused}");

    // The reveal applied, the transfer behind it failed: the batch is refused.
    let reveal = TezosContent::Reveal {
        source: SOURCE.into(),
        fee: 0,
        counter: 1,
        gas_limit: 271,
        storage_limit: 0,
        public_key: "edpku94jEvWaJ5S1JaQei7xEbv2WAGK146aoshAYUmimvUqxyB148S".into(),
    };
    let batch = json!({"contents": [
        {"kind": "reveal", "metadata": {"operation_result": {"status": "backtracked", "consumed_milligas": "171000"}}},
        {"kind": "transaction", "metadata": {"operation_result": {
            "status": "failed",
            "errors": [{"kind": "permanent", "id": "proto.025-PsUshuai.michelson_v1.bad_contract_parameter"}],
        }}},
    ]});
    let refused = consumption_from(&batch, &[reveal, transaction(EMPTY)]).unwrap_err();
    assert!(
        refused.to_string().contains("bad_contract_parameter"),
        "{refused}"
    );
}

#[test]
fn an_answer_for_other_operations_is_refused() {
    let answer = allocating_answer();
    // Fewer operations than were asked about, or another kind.
    assert!(consumption_from(&answer, &[transaction(EMPTY), transaction(EMPTY)]).is_err());
    let reveal = TezosContent::Reveal {
        source: SOURCE.into(),
        fee: 0,
        counter: 1,
        gas_limit: 1,
        storage_limit: 0,
        public_key: String::new(),
    };
    assert!(consumption_from(&answer, &[reveal]).is_err());
    assert!(consumption_from(&json!({}), &[transaction(EMPTY)]).is_err());
}

/// `/context/contracts/{id}` for a funded, revealed account and for one
/// the chain does not hold, read from mainnet nodes 2026-11-24.
#[test]
fn an_account_reads_its_balance_counter_and_key() {
    let funded = json!({"balance": "4412227070176", "counter": "28987845", "revealed": true});
    assert_eq!(
        account_from(&funded).unwrap(),
        TezosAccount {
            balance: 4_412_227_070_176,
            counter: 28_987_845,
            revealed: true,
        }
    );
    let empty = json!({"balance": "0", "counter": "246598707", "revealed": false});
    assert_eq!(
        account_from(&empty).unwrap(),
        TezosAccount {
            balance: 0,
            counter: 246_598_707,
            revealed: false,
        }
    );
    assert!(account_from(&json!({"balance": "-1", "counter": "1", "revealed": true})).is_err());
    assert!(account_from(&json!({"balance": "1", "counter": "1"})).is_err());
}

#[test]
fn a_block_header_names_a_block_hash_and_level() {
    let header = json!({
        "protocol": "PsUshuai9QapM5TGj1JpuVGkdxz5GykdnEvS6Rh8SUVrARvZLCY",
        "chain_id": "NetXdQprcVkpaWU",
        "hash": "BMURKx3R8bQUpHZAruDcUgEMKCKxmXGKRiwGSJtkt2Lc3C3r2Vm",
        "level": 15953511, "proto": 25,
    });
    assert_eq!(
        block_from(&header).unwrap(),
        TezosBlock {
            hash: "BMURKx3R8bQUpHZAruDcUgEMKCKxmXGKRiwGSJtkt2Lc3C3r2Vm".into(),
            level: 15_953_511,
        }
    );
    // An operation hash is not a block's.
    let other = json!({"hash": "ootbVsgxdptx4MrpHNkRjLSkRTR5PbDeAH9XVVR9xc8N69UHFsm", "level": 1});
    assert!(block_from(&other).is_err());
}

#[test]
fn a_simulated_operation_is_written_as_a_node_reads_it() {
    let json = transaction(EMPTY).to_json();
    assert_eq!(json["kind"], "transaction");
    assert_eq!(json["counter"], "28987846");
    assert_eq!(json["amount"], "1");
    assert_eq!(json["destination"], EMPTY);
    // 64 zero bytes, as Taquito's estimate sends them.
    assert_eq!(
        simulation_signature(),
        "edsigtXomBKi5CTRf5cjATJWSyaRvhfYNHqSUGrn4SdbYRcGwQrUGjzEfQDTuqHhuA8b2d8NarZjz8TRf65WkpQmo423BtomS8Q"
    );
}

/// A node that refuses an operation before applying it answers HTTP 500 with
/// its errors; that is the network's answer, read once and shown by id.
#[tokio::test]
async fn a_simulation_the_node_refuses_names_its_errors() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(500).set_body_json(json!([{
            "kind": "temporary",
            "id": "proto.025-PsUshuai.contract.balance_too_low",
            "contract": SOURCE, "balance": "0", "amount": "1",
        }])))
        .expect(1)
        .mount(&server)
        .await;
    let client = TezosClient::new(Arc::new(vec![server.uri()]));
    let error = client
        .simulate(
            "NetXdQprcVkpaWU",
            "BLockGenesisGenesisGenesisGenesisGenesisf79b5d1CoW2",
            &[transaction(EMPTY)],
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ApiError::Rejected(_)), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("proto.025-PsUshuai.contract.balance_too_low"),
        "{error}"
    );
}
