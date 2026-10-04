use super::*;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};

async fn service(server: &MockServer) -> std::sync::Arc<WalletService> {
    WalletService::new(vec![ChainEndpoints {
        chain_id: Chain::WorldChain,
        capabilities: vec![EndpointCapability::Verification],
        endpoints: vec![server.uri()],
    }])
    .unwrap()
}

async fn mount_receipt(server: &MockServer, status: &str, l1: serde_json::Value) {
    Mock::given(body_partial_json(
        json!({"method":"eth_getTransactionReceipt"}),
    ))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "jsonrpc":"2.0","id":1,"result":{
            "blockNumber":"0x7","status":status,"gasUsed":"0x5208",
            "effectiveGasPrice":"0x2","l1Fee":l1
        }
    })))
    .mount(server)
    .await;
}

#[tokio::test]
async fn rollup_actual_cost_uses_mined_block_oracle_for_success_and_revert() {
    for status in ["0x1", "0x0"] {
        let server = MockServer::start().await;
        mount_receipt(&server, status, json!("0x64")).await;
        Mock::given(body_partial_json(json!({
            "method":"eth_call","params":[{
                "to":"0x420000000000000000000000000000000000000F",
                "data":format!("0x275aedd2{:064x}",21000)
            },"0x7"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc":"2.0","id":1,"result":format!("0x{:064x}",253)
        })))
        .expect(1)
        .mount(&server)
        .await;
        let receipt = service(&server)
            .await
            .evm_transaction_status(Chain::WorldChain, "0x11".into())
            .await
            .unwrap()
            .unwrap();
        assert!(receipt.is_confirmed);
        assert_eq!(receipt.is_failed, status == "0x0");
        let cost = receipt.cost.unwrap();
        assert_eq!(cost.network_fee, "0.000000000000042353");
        assert_eq!(cost.l1_data_fee.as_deref(), Some("0.0000000000000001"));
        assert_eq!(cost.operator_fee.as_deref(), Some("0.000000000000000253"));
        assert_eq!(cost.validated_for_chain(Chain::WorldChain), Some(cost));
        server.verify().await;
    }
}

#[tokio::test]
async fn historical_pre_operator_receipt_has_zero_operator_charge() {
    let server = MockServer::start().await;
    mount_receipt(&server, "0x1", json!("0x64")).await;
    Mock::given(body_partial_json(json!({"method":"eth_call"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"not deployed"}
        })))
        .mount(&server)
        .await;
    Mock::given(body_partial_json(
        json!({"method":"eth_getBlockByNumber","params":["0x7",false]}),
    ))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc":"2.0","id":1,"result":{"number":"0x7","timestamp":format!("0x{:x}",1_764_072_000u64-1)}
    })))
    .mount(&server)
    .await;
    let receipt = service(&server)
        .await
        .evm_transaction_status(Chain::WorldChain, "0x11".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.cost.unwrap().network_fee, "0.0000000000000421");
}

#[tokio::test]
async fn wrong_block_cannot_establish_a_historical_zero_charge() {
    let server = MockServer::start().await;
    mount_receipt(&server, "0x1", json!("0x64")).await;
    Mock::given(body_partial_json(json!({"method":"eth_call"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc":"2.0","id":1,"result":"0x"
        })))
        .mount(&server)
        .await;
    Mock::given(body_partial_json(json!({"method":"eth_getBlockByNumber"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc":"2.0","id":1,"result":{"number":"0x6","timestamp":"0x1"}
        })))
        .mount(&server)
        .await;
    let receipt = service(&server)
        .await
        .evm_transaction_status(Chain::WorldChain, "0x11".into())
        .await
        .unwrap()
        .unwrap();
    assert!(receipt.is_confirmed);
    assert!(receipt.cost.is_none());
}

#[tokio::test]
async fn fee_model_without_operator_charge_needs_no_operator_oracle() {
    let server = MockServer::start().await;
    mount_receipt(&server, "0x1", json!("0x64")).await;
    let service = WalletService::new(vec![ChainEndpoints {
        chain_id: Chain::CeloSepolia,
        capabilities: vec![EndpointCapability::Verification],
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    let receipt = service
        .evm_transaction_status(Chain::CeloSepolia, "0x11".into())
        .await
        .unwrap()
        .unwrap();
    let cost = receipt.cost.unwrap();
    assert_eq!(cost.network_fee, "0.0000000000000421");
    assert_eq!(cost.operator_fee.as_deref(), Some("0"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn missing_or_malformed_rollup_components_remain_unknown() {
    for l1 in [serde_json::Value::Null, json!("not-hex"), json!("0x64")] {
        let server = MockServer::start().await;
        mount_receipt(&server, "0x0", l1).await;
        Mock::given(body_partial_json(json!({"method":"eth_call"})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":"0x"})),
            )
            .mount(&server)
            .await;
        Mock::given(body_partial_json(json!({"method":"eth_getBlockByNumber"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc":"2.0","id":1,"result":{"number":"0x7","timestamp":format!("0x{:x}",1_764_072_000u64)}
            })))
            .mount(&server)
            .await;
        let receipt = service(&server)
            .await
            .evm_transaction_status(Chain::WorldChain, "0x11".into())
            .await
            .unwrap()
            .unwrap();
        assert!(receipt.is_confirmed && receipt.is_failed);
        assert!(receipt.cost.is_none());
    }
}

#[test]
fn rollup_cost_rejects_subtotals_inconsistent_totals_and_overflow() {
    use crate::store::EvmReceiptCost;
    let subtotal = EvmReceiptCost::from_receipt(Some("21000"), Some("2"), 18).unwrap();
    assert!(subtotal.validated_for_chain(Chain::WorldChain).is_none());
    let mut cost =
        EvmReceiptCost::from_rollup_receipt(Some("21000"), Some("2"), Some("100"), Some("253"), 18)
            .unwrap();
    cost.network_fee = "0.1".into();
    assert!(cost.validated_for_chain(Chain::WorldChain).is_none());
    assert!(
        EvmReceiptCost::from_rollup_receipt(
            Some("1"),
            Some("1"),
            Some(&u128::MAX.to_string()),
            Some("1"),
            18
        )
        .is_none()
    );
}
