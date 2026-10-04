//! Actual protocol identity routes metadata and transfer preparation.
use super::*;
use crate::send::SendExecutionRequest;
use crate::service::{ChainEndpoints, WalletService};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn request(chain_id: Chain, token_standard: Option<&str>) -> SendExecutionRequest {
    SendExecutionRequest {
        chain_id,
        wallet_id: "unopened-wallet".into(),
        password: None,
        to_address: "TD5gsCwxykWsLN9aPrq2TAfNjByuZKYp4E".into(),
        amount_str: "1".into(),
        contract_address: Some("1002000".into()),
        token_standard: token_standard.map(str::to_string),
        token_decimals: Some(6),
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: None,
        monero_priority: None,
        sign_only: true,
    }
}

fn service(server: &MockServer) -> std::sync::Arc<WalletService> {
    WalletService::new(vec![ChainEndpoints {
        chain_id: Chain::Tron,
        endpoints: vec![server.uri()],
        capabilities: crate::EndpointCapability::ALL.to_vec(),
    }])
    .unwrap()
}

#[test]
fn execution_validation_accepts_trc10_and_refuses_wrong_protocol_on_both_networks() {
    for chain in [Chain::Tron, Chain::TronNile] {
        for standard in [None, Some("TRC-10")] {
            validate_execution_amount(chain, &request(chain, standard)).unwrap();
        }
        assert!(validate_execution_amount(chain, &request(chain, Some("TRC-20"))).is_err());
    }
}

#[test]
fn a_native_send_cannot_carry_a_token_protocol() {
    let mut request = request(Chain::Tron, Some("TRC-20"));
    request.contract_address = None;
    request.token_decimals = None;
    assert!(
        validate_execution_amount(Chain::Tron, &request)
            .unwrap_err()
            .to_string()
            .contains("requires an identifier")
    );
}

#[tokio::test]
async fn wrong_protocol_build_and_execute_refuse_before_storage_keys_or_rpc() {
    let server = MockServer::start().await;
    let service = service(&server);
    for standard in [Some("TRC-20"), Some("unknown")] {
        assert!(
            service
                .build_send(request(Chain::Tron, standard))
                .await
                .is_err()
        );
        assert!(
            service
                .execute_send(request(Chain::Tron, standard))
                .await
                .is_err()
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn trc10_preview_and_metadata_use_real_token_precision_and_native_fee_budget() {
    let server = MockServer::start().await;
    let owner = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
    for (route, value) in [
        (
            "/wallet/getblockbynum",
            json!({"blockID":Chain::Tron.tron_genesis_block_id().unwrap()}),
        ),
        (
            "/wallet/getassetissuebyid",
            json!({"id":"1002000", "name":hex::encode("Test Legacy"), "abbr":hex::encode("T10"), "precision":2}),
        ),
        (
            "/wallet/getaccount",
            json!({"address":owner,"balance":10_000_000,"assetV2":[{"key":"1002000","value":2345}]}),
        ),
        (
            "/wallet/getchainparameters",
            json!({"chainParameter":[{"key":"getTransactionFee","value":1000},{"key":"getCreateAccountFee","value":100_000},{"key":"getCreateNewAccountFeeInSystemContract","value":1_000_000}]}),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(value))
            .mount(&server)
            .await;
    }
    let service = service(&server);
    assert_eq!(
        service
            .token_contract_decimals(Chain::Tron, "1002000")
            .await
            .unwrap(),
        Some(2)
    );
    let preview: serde_json::Value = serde_json::from_str(
        &service
            .fetch_tron_send_preview_json_on_chain(
                Chain::Tron,
                owner.into(),
                "T10".into(),
                "1002000".into(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(preview["max_sendable"], "23.45");
    assert_eq!(preview["estimated_fee_trx"], "1.1");
    assert_eq!(preview["fee_limit_sun"], 0);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() != "/wallet/triggerconstantcontract")
    );
}
