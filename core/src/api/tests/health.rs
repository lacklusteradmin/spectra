use super::*;
use crate::catalog::endpoints::EndpointRecord;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn record(chain: Chain, api: EndpointApi, endpoint: String) -> EndpointRecord {
    EndpointRecord {
        id: "test".into(),
        chain_id: chain,
        api,
        endpoint,
        capabilities: vec![EndpointCapability::History],
    }
}

/// Every built-in endpoint is checked: asked its network, or, where its API
/// cannot say, read through that API on its own origin.
#[test]
fn every_built_in_endpoint_is_checked_on_its_own_origin() {
    for record in &crate::catalog::endpoints::catalog().records {
        if crate::api::network::asks_network(record.chain_id, record.api) {
            continue;
        }
        let checks = reads(&endpoint(record), &record.capabilities);
        assert!(!checks.is_empty(), "{} is never checked", record.id);
        for check in checks {
            assert_eq!(
                reqwest::Url::parse(&check.url).unwrap().origin(),
                reqwest::Url::parse(&record.endpoint).unwrap().origin(),
                "{}",
                record.id
            );
        }
    }
}

fn replica_status_bytes(status: ciborium::Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(
        &ciborium::Value::Tag(
            55799,
            Box::new(ciborium::Value::Map(vec![(
                ciborium::Value::Text("replica_health_status".into()),
                status,
            )])),
        ),
        &mut bytes,
    )
    .unwrap();
    bytes
}

#[tokio::test]
async fn icp_replica_health_reads_cbor_status_on_the_configured_origin() {
    let server = MockServer::start().await;
    let record = record(
        Chain::Icp,
        EndpointApi::IcpReplica,
        format!("{}/replica/", server.uri()),
    );
    // Tagged CBOR and an untagged map both occur in IC agent transports.
    let tagged = replica_status_bytes(ciborium::Value::Text("healthy".into()));
    for bytes in [tagged.clone(), tagged[3..].to_vec()] {
        Mock::given(method("GET"))
            .and(path("/replica/api/v2/status"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(bytes, "application/cbor"))
            .expect(1)
            .mount(&server)
            .await;
        let (checked, healthy, detail) = probe_record(Chain::Icp, &record).await;
        assert!(checked && healthy, "{detail}");
        server.verify().await;
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().all(|request| request.body.is_empty()));
        server.reset().await;
    }
}

#[tokio::test]
async fn icp_replica_health_refuses_unhealthy_and_malformed_statuses() {
    use ciborium::Value as Cbor;
    let server = MockServer::start().await;
    let record = record(Chain::Icp, EndpointApi::IcpReplica, server.uri());
    let healthy = replica_status_bytes(Cbor::Text("healthy".into()));
    let mut trailing = healthy.clone();
    trailing.push(0);
    let mut duplicate = Vec::new();
    ciborium::into_writer(
        &Cbor::Map(vec![
            (
                Cbor::Text("replica_health_status".into()),
                Cbor::Text("healthy".into()),
            ),
            (
                Cbor::Text("replica_health_status".into()),
                Cbor::Text("starting".into()),
            ),
        ]),
        &mut duplicate,
    )
    .unwrap();
    let mut cases = [
        "starting",
        "waiting_for_certified_state",
        "waiting_for_root_delegation",
        "certified_state_behind",
        "unknown",
        "Healthy",
    ]
    .into_iter()
    .map(|status| replica_status_bytes(Cbor::Text(status.into())))
    .collect::<Vec<_>>();
    cases.extend([
        replica_status_bytes(Cbor::Bool(true)),
        vec![0xa0], // A valid map without the required health field.
        vec![0x80], // An array instead of a status record.
        healthy[..healthy.len() - 1].to_vec(),
        trailing,
        duplicate,
        b"<html>Unavailable</html>".to_vec(),
        br#"{"replica_health_status":"healthy"}"#.to_vec(),
    ]);
    for bytes in cases {
        Mock::given(method("GET"))
            .and(path("/api/v2/status"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(bytes, "application/cbor"))
            .expect(2)
            .mount(&server)
            .await;
        let (checked, healthy, detail) = probe_record(Chain::Icp, &record).await;
        assert!(checked && !healthy, "{detail}");
        server.verify().await;
        server.reset().await;
    }
    Mock::given(method("GET"))
        .and(path("/api/v2/status"))
        .respond_with(ResponseTemplate::new(503).set_body_raw(healthy, "application/cbor"))
        .expect(2)
        .mount(&server)
        .await;
    let (checked, healthy, detail) = probe_record(Chain::Icp, &record).await;
    assert!(checked && !healthy && detail.contains("503"), "{detail}");
    server.verify().await;
}

#[tokio::test]
async fn aptos_indexer_health_uses_the_configured_api_and_proves_network_identity() {
    let server = MockServer::start().await;
    let record = record(
        Chain::Aptos,
        EndpointApi::AptosIndexer,
        format!("{}/api/graphql", server.uri()),
    );
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_partial_json(
            json!({"query":"query { ledger_infos { chain_id } }"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data":{"ledger_infos":[{"chain_id":1}]}
        })))
        .expect(2)
        .mount(&server)
        .await;
    assert!(probe_record(Chain::Aptos, &record).await.1);
    let (checked, reachable, detail) = probe_record(Chain::AptosTestnet, &record).await;
    assert!(checked && !reachable);
    assert!(detail.contains("chain id 1, expected 2"), "{detail}");
    server.verify().await;
}

#[tokio::test]
async fn peercoin_health_verifies_network_and_precision_on_the_configured_origin() {
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        for invalid_field in [None, Some("network"), Some("precision"), Some("coin")] {
            let server = MockServer::start().await;
            let mut identity = json!({
                "blockbook": {
                    "coin": if chain.is_testnet() { "Peercoin Testnet" } else { "Peercoin" },
                    "decimals": 6
                },
                "backend": {
                    "chain": if chain.is_testnet() { "testnet" } else { "livenet" },
                    "blocks": 900_000
                }
            });
            match invalid_field {
                Some("network") => {
                    identity["backend"]["chain"] = json!(if chain.is_testnet() {
                        "livenet"
                    } else {
                        "testnet"
                    });
                }
                Some("precision") => identity["blockbook"]["decimals"] = json!(8),
                Some("coin") => identity["blockbook"]["coin"] = json!("Bitcoin"),
                None => {}
                _ => unreachable!(),
            }
            Mock::given(method("GET"))
                .and(path("/peercoin/api/v2"))
                .respond_with(ResponseTemplate::new(200).set_body_json(identity))
                .mount(&server)
                .await;
            let (checked, healthy, detail) = probe_record(
                chain,
                &record(
                    chain,
                    EndpointApi::Blockbook,
                    format!("{}/peercoin", server.uri()),
                ),
            )
            .await;
            assert!(checked, "{detail}");
            assert_eq!(healthy, invalid_field.is_none(), "{detail}");
            let requests = server.received_requests().await.unwrap();
            assert!(!requests.is_empty());
            assert!(requests.iter().all(|request| {
                request.method == "GET" && request.url.path() == "/peercoin/api/v2"
            }));
        }
    }
}

/// An EVM node passes on its chain id alone, and another network's fails.
#[tokio::test]
async fn evm_passes_on_its_chain_id_and_another_networks_fails() {
    let server = MockServer::start().await;
    let record = record(Chain::Ethereum, EndpointApi::EvmJsonRpc, server.uri());
    Mock::given(body_partial_json(json!({"method":"eth_chainId"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":"0x1"})))
        .expect(1)
        .mount(&server)
        .await;
    let (checked, reachable, detail) = probe_record(Chain::Ethereum, &record).await;
    assert!(
        checked && reachable && detail == "network verified",
        "{detail}"
    );
    server.verify().await;
    server.reset().await;
    Mock::given(body_partial_json(json!({"method":"eth_chainId"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":"0x89"})))
        .mount(&server)
        .await;
    let (checked, reachable, detail) = probe_record(Chain::Ethereum, &record).await;
    assert!(
        checked && !reachable && detail.contains("wrong network"),
        "{detail}"
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.body_json::<Value>().unwrap()["method"] == "eth_chainId")
    );
}

#[tokio::test]
async fn polkadot_health_refuses_a_relay_node_before_other_reads() {
    let server = MockServer::start().await;
    Mock::given(body_partial_json(
        json!({"method":"chain_getBlockHash","params":[0]}),
    ))
    .respond_with(ResponseTemplate::new(200).set_body_json(
        json!({"result":"0x91b171bb158e2d3848fa23a9f1c25182fb8e20313b2c1eb49219da7a70ce90c3"}),
    ))
    .expect(1)
    .mount(&server)
    .await;
    let (_, healthy, detail) = probe_record(
        Chain::Polkadot,
        &record(Chain::Polkadot, EndpointApi::SubstrateJsonRpc, server.uri()),
    )
    .await;
    assert!(!healthy && detail.contains("wrong Substrate"));
}

#[tokio::test]
async fn esplora_and_monero_are_asked_their_network_on_the_protocol_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/block-height/1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                "00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048",
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        probe_record(
            Chain::Bitcoin,
            &record(
                Chain::Bitcoin,
                EndpointApi::Esplora,
                format!("{}/api", server.uri())
            )
        )
        .await
        .1
    );
    // A daemon on another network fails before anything else is read.
    Mock::given(method("POST"))
        .and(path("/get_info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"nettype":"stagenet","synchronized":true})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (checked, reachable, detail) = probe_record(
        Chain::Monero,
        &record(Chain::Monero, EndpointApi::MoneroDaemonRpc, server.uri()),
    )
    .await;
    assert!(
        checked && !reachable && detail.contains("wrong network"),
        "{detail}"
    );
}

/// A Blockbook endpoint is probed at its `/api/v2` status, once, and is
/// healthy only when that reports the height it is at; an answer without
/// one is unreachable, and the detail says what came back.
#[tokio::test]
async fn blockbook_is_probed_at_its_status_and_must_report_its_height() {
    let server = MockServer::start().await;
    let record = record(Chain::DashTestnet, EndpointApi::Blockbook, server.uri());
    Mock::given(method("GET"))
        .and(path("/api/v2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"blockbook": {"bestHeight": 123}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (checked, reachable, detail) = probe_record(Chain::DashTestnet, &record).await;
    assert!(checked && reachable, "{detail}");
    for answer in [
        json!({"error": "Internal server error"}),
        json!({"blockbook": {"coin": "Dash Testnet"}}),
    ] {
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(answer.clone()))
            .mount(&server)
            .await;
        let (checked, reachable, detail) = probe_record(Chain::DashTestnet, &record).await;
        assert!(checked && !reachable, "{answer}");
        assert!(detail.contains(&answer.to_string()), "{detail}");
    }
}

#[tokio::test]
async fn rest_probes_reject_html() {
    let server = MockServer::start().await;
    let record = record(
        Chain::Tron,
        EndpointApi::TrongridV1,
        format!("{}/v1/accounts", server.uri()),
    );
    Mock::given(method("GET"))
        .and(path(format!("/v1/accounts/{ZERO_TRON}")))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>Unavailable</html>"))
        .mount(&server)
        .await;
    let (checked, reachable, _) = probe_record(Chain::Tron, &record).await;
    assert!(checked && !reachable);
    assert!(!server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn history_rejects_http_success_with_an_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/etherscan"))
        .and(query_param("action", "txlist"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status":"0","message":"chain not supported","result":null})),
        )
        .mount(&server)
        .await;
    let (_, reachable, detail) = probe_record(
        Chain::Berachain,
        &record(
            Chain::Berachain,
            EndpointApi::Blockscout,
            format!("{}/etherscan", server.uri()),
        ),
    )
    .await;
    assert!(!reachable && detail.contains("chain not supported"));
}

/// An ICP Rosetta node passes only listing the ICP ledger, and a refusal
/// is reported with its status.
#[tokio::test]
async fn rosetta_must_list_the_icp_ledger() {
    let server = MockServer::start().await;
    let record = record(Chain::Icp, EndpointApi::IcpRosetta, server.uri());
    for (identifier, verified) in [
        (crate::api::icp_rosetta::network_identifier(), true),
        (
            json!({"blockchain":"Internet Computer","network":"test"}),
            false,
        ),
    ] {
        Mock::given(method("POST"))
            .and(path("/network/list"))
            .and(body_partial_json(json!({"metadata":{}})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"network_identifiers":[identifier]})),
            )
            .mount(&server)
            .await;
        assert_eq!(probe_record(Chain::Icp, &record).await.1, verified);
        server.reset().await;
    }
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let (_, reachable, detail) = probe_record(Chain::Icp, &record).await;
    assert!(!reachable && detail.contains("403"), "{detail}");
}

/// An Algorand node and indexer pass only naming mainnet's genesis id and
/// hash, as Nodely's answered on 2026-11-24.
#[tokio::test]
async fn an_algorand_endpoint_passes_only_on_mainnets_genesis() {
    let mainnet = json!({
        "genesis-id": "mainnet-v1.0",
        "genesis-hash": "wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=",
        "last-round": 67257603,
        "min-fee": 1000,
        "fee": 0,
    });
    let mut testnet = mainnet.clone();
    testnet["genesis-id"] = json!("testnet-v1.0");
    testnet["genesis-hash"] = json!("SGO1GKSzyE7IEPItTxCByw9x8FmnrCDexi9/cOUJOiI=");
    let mut renamed = mainnet.clone();
    renamed["genesis-hash"] = testnet["genesis-hash"].clone();
    for (api, route) in [
        (EndpointApi::Algod, "/v2/transactions/params"),
        (EndpointApi::AlgorandIndexer, "/v2/blocks/0"),
    ] {
        for (answer, verified) in [(&mainnet, true), (&testnet, false), (&renamed, false)] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(200).set_body_json(answer))
                .mount(&server)
                .await;
            let (_, reachable, detail) =
                probe_record(Chain::Algorand, &record(Chain::Algorand, api, server.uri())).await;
            assert_eq!(reachable, verified, "{api:?} {detail}");
        }
    }
}

/// An endpoint whose API cannot name its network passes on its API's reads
/// and says it was not asked.
#[tokio::test]
async fn an_endpoint_that_cannot_name_its_network_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"blocks": 900_000})))
        .mount(&server)
        .await;
    let (checked, reachable, detail) = probe_record(
        Chain::Decred,
        &record(Chain::Decred, EndpointApi::Insight, server.uri()),
    )
    .await;
    assert!(checked && reachable, "{detail}");
    assert!(detail.contains("cannot name its network"), "{detail}");
}

/// A catalog record's endpoint and what it is used for, as the service hands
/// them over.
async fn probe_record(chain: Chain, record: &EndpointRecord) -> (bool, bool, String) {
    probe(chain, &endpoint(record), &record.capabilities).await
}

fn endpoint(record: &EndpointRecord) -> Endpoint {
    Endpoint {
        api: record.api,
        url: record.endpoint.clone(),
    }
}
