use super::*;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn record(chain: Chain, api: EndpointApi, endpoint: String) -> AppCoreEndpointRecord {
    AppCoreEndpointRecord {
        id: "test".into(),
        chain_id: chain.str_id().into(),
        api: Some(api),
        endpoint,
        capabilities: vec!["history".into()],
        probe_url: None,
        explorer_label: None,
        tx_suffix: String::new(),
    }
}

#[test]
fn every_builtin_api_has_a_probe_on_its_own_origin() {
    for record in &crate::app_core::endpoint_catalog()
        .unwrap()
        .endpoint_records
    {
        if record.api.is_none() {
            continue;
        }
        let chain = Chain::from_str_id(&record.chain_id).unwrap();
        let checks = checks(chain, record).unwrap_or_else(|e| panic!("{}: {e}", record.id));
        assert!(!checks.is_empty());
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

#[tokio::test]
async fn evm_checks_reads_after_chain_identity_and_rejects_wrong_network() {
    let server = MockServer::start().await;
    let record = record(Chain::Ethereum, EndpointApi::EvmJsonRpc, server.uri());
    Mock::given(body_partial_json(json!({"method":"eth_chainId"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":"0x1"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(body_partial_json(json!({"method":"eth_blockNumber"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"error":{"code":-32046,"message":"Cannot fulfill request"}})),
        )
        .mount(&server)
        .await;
    let (checked, reachable, detail) = probe(Chain::Ethereum, &record).await;
    assert!(checked && !reachable);
    assert!(detail.contains("eth_blockNumber") && detail.contains("Cannot fulfill request"));
    server.reset().await;
    Mock::given(body_partial_json(json!({"method":"eth_chainId"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":"0x89"})))
        .mount(&server)
        .await;
    assert!(!probe(Chain::Ethereum, &record).await.1);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.body_json::<Value>().unwrap()["method"] == "eth_chainId")
    );
}

#[test]
fn rpc_requires_a_result_and_accepts_large_hex_balances() {
    let check = Check::rpc(
        "https://example.org",
        "eth_getBalance",
        json!([]),
        Response::RpcHex(None),
    );
    assert!(
        check
            .validate(
                Chain::Ethereum,
                &json!({"result":"0x100000000000000000000"})
            )
            .is_ok()
    );
    for response in [
        json!({}),
        json!({"result":null}),
        json!({"result":"not hex"}),
        json!({"result":"0x"}),
    ] {
        assert!(check.validate(Chain::Ethereum, &response).is_err());
    }
}

#[tokio::test]
async fn esplora_and_monero_use_the_protocol_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/blocks/tip/height"))
        .respond_with(ResponseTemplate::new(200).set_body_string("900000"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        probe(
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
    Mock::given(method("POST"))
        .and(path("/json_rpc"))
        .and(body_partial_json(json!({"method":"get_info"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"result":{"nettype":"mainnet","synchronized":true}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        probe(
            Chain::Monero,
            &record(Chain::Monero, EndpointApi::MoneroDaemonRpc, server.uri())
        )
        .await
        .1
    );
}

#[tokio::test]
async fn rest_probes_ignore_an_unrelated_probe_host_and_reject_html() {
    let server = MockServer::start().await;
    let mut record = record(
        Chain::Tron,
        EndpointApi::TrongridV1,
        format!("{}/v1/accounts", server.uri()),
    );
    record.probe_url = Some("https://unrelated.invalid/wallet/getnowblock".into());
    Mock::given(method("GET"))
        .and(path(format!("/v1/accounts/{ZERO_TRON}")))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>Unavailable</html>"))
        .mount(&server)
        .await;
    let (checked, reachable, _) = probe(Chain::Tron, &record).await;
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
    let (_, reachable, detail) = probe(
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

#[tokio::test]
async fn rosetta_posts_metadata_and_checks_the_response() {
    let server = MockServer::start().await;
    let record = record(Chain::Icp, EndpointApi::IcpRosetta, server.uri());
    Mock::given(method("POST"))
        .and(path("/network/list"))
        .and(body_partial_json(json!({"metadata":{}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"network_identifiers":[{"blockchain":"internet-computer","network":"test"}]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    assert!(probe(Chain::Icp, &record).await.1);
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let (_, reachable, detail) = probe(Chain::Icp, &record).await;
    assert!(!reachable && detail.contains("403"));
}
