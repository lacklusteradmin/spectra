//! Unsupported token protocols are refused before metadata, storage or keys.
use super::*;
use crate::send::SendExecutionRequest;
use crate::service::{ChainEndpoints, WalletService};
use wiremock::MockServer;

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

fn assert_trc10_refusal(error: SpectraBridgeError) {
    let message = error.to_string();
    assert!(message.contains("TRC-10"), "{message}");
    assert!(message.contains("not supported"), "{message}");
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
fn execution_validation_refuses_explicit_and_inferred_trc10_on_both_networks() {
    for chain in [Chain::Tron, Chain::TronNile] {
        for standard in [None, Some("TRC-10")] {
            assert_trc10_refusal(
                validate_execution_amount(chain, &request(chain, standard)).unwrap_err(),
            );
        }
    }
}

#[test]
fn a_native_send_cannot_carry_a_token_protocol() {
    let mut request = request(Chain::Tron, Some("TRC-20"));
    request.contract_address = None;
    request.token_decimals = None;
    let message = validate_execution_amount(Chain::Tron, &request)
        .unwrap_err()
        .to_string();
    assert!(message.contains("requires an identifier"), "{message}");
}

#[tokio::test]
async fn direct_build_and_execute_refuse_trc10_before_storage_keys_or_rpc() {
    let server = MockServer::start().await;
    let service = service(&server);
    for standard in [None, Some("TRC-10")] {
        assert_trc10_refusal(
            service
                .build_send(request(Chain::Tron, standard))
                .await
                .unwrap_err(),
        );
        assert_trc10_refusal(
            service
                .execute_send(request(Chain::Tron, standard))
                .await
                .unwrap_err(),
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn trc10_preview_refuses_before_rpc() {
    let server = MockServer::start().await;
    let service = service(&server);
    assert_trc10_refusal(
        service
            .fetch_tron_send_preview_json(
                "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH".into(),
                "T10".into(),
                "1002000".into(),
            )
            .await
            .unwrap_err(),
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn trc10_metadata_refuses_before_rpc() {
    let server = MockServer::start().await;
    let service = service(&server);
    assert_trc10_refusal(
        service
            .token_contract_decimals(Chain::Tron, "1002000")
            .await
            .unwrap_err(),
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
