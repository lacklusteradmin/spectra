use super::*;
use std::sync::{Arc, Mutex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

#[derive(Clone)]
pub(crate) struct NodeState {
    pub genesis: String,
    pub member: bool,
    pub member_migration: bool,
    pub pool_migration: bool,
    pub pending_slash: u128,
    pub fee: u128,
    pub nonce: u32,
    pub free: u128,
}
pub(crate) async fn node() -> (SubstrateClient, Arc<Mutex<NodeState>>, MockServer) {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/polkadot-staking-vectors.json"
    ))
    .unwrap();
    let state = Arc::new(Mutex::new(NodeState {
        genesis: Chain::Polkadot.substrate_genesis_hash().unwrap().into(),
        member: true,
        member_migration: false,
        pool_migration: false,
        pending_slash: 0,
        fee: 1_000_000,
        nonce: 7,
        free: 1_000_000_000_000_000,
    }));
    let server = MockServer::start().await;
    let live = state.clone();
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let state = live.lock().unwrap();
            let result = match body["method"].as_str().unwrap() {
                "chain_getBlockHash" if body["params"][0] == 0 => json!(state.genesis),
                "chain_getBlockHash" | "chain_getFinalizedHead" => {
                    json!(format!("0x{}", "11".repeat(32)))
                }
                "chain_getHeader" => json!({"number":"0x64"}),
                "state_getRuntimeVersion" => {
                    json!({"specVersion":2_005_000,"transactionVersion":15})
                }
                "state_getMetadata" => json!(format!(
                    "0x{}",
                    hex::encode(crate::api::substrate_json_rpc::tests::fixture(
                        Chain::Polkadot
                    ))
                )),
                "state_getKeysPaged" => json!([fixture["state"]["pool"]["key"]]),
                "state_getStorage" => {
                    let key = body["params"][0].as_str().unwrap();
                    if key.starts_with("0x26aa394eea5630e07c48ae0c9558cef7b99d") {
                        let mut bytes = vec![0; 16];
                        bytes.extend(state.free.to_le_bytes());
                        bytes.extend(vec![0; 48]);
                        json!(format!("0x{}", hex::encode(bytes)))
                    } else {
                        let item = fixture["state"]
                            .as_object()
                            .unwrap()
                            .iter()
                            .find(|(_, row)| row["key"] == key)
                            .unwrap_or_else(|| panic!("unexpected storage key {key}"));
                        if item.0 == "member" && !state.member {
                            Value::Null
                        } else {
                            item.1["hex"].clone()
                        }
                    }
                }
                "state_call" => {
                    let input = decode_hex(body["params"][1].as_str().unwrap()).unwrap();
                    let bytes = match body["params"][0].as_str().unwrap() {
                        "NominationPoolsApi_points_to_balance" => {
                            let (_, points) = <(u32, u128)>::decode(&mut input.as_slice()).unwrap();
                            ratio(points, 1_800_000_000_000, 2_000_000_000_000)
                                .unwrap()
                                .encode()
                        }
                        "NominationPoolsApi_balance_to_points" => {
                            let (_, amount) = <(u32, u128)>::decode(&mut input.as_slice()).unwrap();
                            (amount * 10 / 9).encode()
                        }
                        "NominationPoolsApi_pending_rewards" => Some(30_000_000_000u128).encode(),
                        "NominationPoolsApi_member_needs_delegate_migration" => {
                            state.member_migration.encode()
                        }
                        "NominationPoolsApi_pool_needs_delegate_migration" => {
                            state.pool_migration.encode()
                        }
                        "NominationPoolsApi_member_pending_slash" => state.pending_slash.encode(),
                        method => panic!("unexpected state_call {method}"),
                    };
                    json!(format!("0x{}", hex::encode(bytes)))
                }
                "system_accountNextIndex" => json!(state.nonce),
                "payment_queryInfo" => json!({"partialFee":state.fee.to_string()}),
                name => panic!("unexpected RPC {name}"),
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":result}))
        })
        .mount(&server)
        .await;
    (
        SubstrateClient::new(Arc::new(vec![server.uri()])),
        state,
        server,
    )
}

#[tokio::test]
async fn pool_snapshot_proves_network_and_keeps_slashed_chunks_and_rewards_exact() {
    let (client, state, server) = node().await;
    let snapshot = client.nomination_pool_snapshot(&[7; 32]).await.unwrap();
    assert_eq!(snapshot.minimum_join, 10_000_000_000);
    let member = snapshot.member.unwrap();
    assert_eq!(member.active_balance, 900_000_000_000);
    assert_eq!(member.pending_rewards, 30_000_000_000);
    assert_eq!(member.withdrawable_balance, 90_000_000_000);
    assert_eq!(member.unbonding_balance, 45_000_000_000);
    assert_eq!(member.next_unlock_era, Some(20));
    assert_eq!(member.pool_id, 7);
    let pools = client.fetch_nomination_pools().await.unwrap();
    assert_eq!(pools[0].name, "Spectra Pool");
    assert_eq!(pools[0].commission, Some(0.1));
    assert_eq!(pools[0].active_balance, 1_800_000_000_000);
    let before = server.received_requests().await.unwrap().len();
    state.lock().unwrap().genesis = format!("0x{}", "ff".repeat(32));
    assert!(client.nomination_pool_snapshot(&[7; 32]).await.is_err());
    let requests = server.received_requests().await.unwrap();
    assert!(requests[before..].iter().all(|request| {
        serde_json::from_slice::<Value>(&request.body).unwrap()["method"] == "chain_getBlockHash"
    }));
}
#[tokio::test]
async fn missing_membership_is_an_empty_position_after_network_proof() {
    let (client, state, _server) = node().await;
    state.lock().unwrap().member = false;
    assert!(
        client
            .nomination_pool_snapshot(&[7; 32])
            .await
            .unwrap()
            .member
            .is_none()
    );
}
