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

mod history_page_failures {
    use super::*;
    #[tokio::test]
    async fn history_page_without_a_configured_source_is_an_error() {
        let service = WalletService::new(vec![]).unwrap();
        // BSC has no configured keyless history source. This fails offline,
        // before HTTP, and must not masquerade as an empty successful page.
        let result = service
            .fetch_evm_history_page(Chain::BnbChain, "from".into(), vec![], 2, 7)
            .await;
        assert!(result.unwrap_err().to_string().contains("no explorer"));
    }
}

mod receipt_cost_completeness {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};

    #[tokio::test]
    async fn world_chain_receipt_omits_partial_cost_without_losing_outcome() {
        for chain in [Chain::WorldChain, Chain::Ethereum] {
            for (status, is_failed) in [("0x1", false), ("0x0", true)] {
                let server = MockServer::start().await;
                Mock::given(body_partial_json(
                    json!({"method": "eth_getTransactionReceipt"}),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0", "id": 1,
                    "result": {
                        "status": status, "blockNumber": "0x7",
                        "gasUsed": "0x5208", "effectiveGasPrice": "0x1"
                    }
                })))
                .expect(if chain == Chain::WorldChain { 2 } else { 1 })
                .mount(&server)
                .await;
                let service = WalletService::new(vec![ChainEndpoints {
                    chain_id: chain,
                    capabilities: vec![EndpointCapability::Verification],
                    endpoints: vec![server.uri()],
                }])
                .unwrap();
                let receipt = service
                    .evm_transaction_status(chain, format!("0x{}", "11".repeat(32)))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(receipt.is_confirmed);
                assert_eq!(receipt.is_failed, is_failed);
                assert_eq!(receipt.block_number, Some(7));
                if chain == Chain::WorldChain {
                    assert!(receipt.cost.is_none());
                    let database = std::env::temp_dir()
                        .join(format!("op-receipt-{}.sqlite", crate::store::new_id()))
                        .to_string_lossy()
                        .into_owned();
                    service.opened(database).await.unwrap();
                    let old: crate::store::persistence_models::TransactionRecord =
                        serde_json::from_value(json!({
                            "id": "pending", "walletId": "w", "walletName": "Wallet",
                            "kind": "send", "status": "pending", "chainId": chain,
                            "symbol": "ETH", "assetDisplayName": "Ethereum", "amount": "1",
                            "address": "recipient", "createdAtUnix": 0.0,
                            "transactionHash": format!("0x{}", "11".repeat(32)),
                            "receiptGasUsed": "21000", "receiptEffectiveGasPriceGwei": "0.000000001",
                            "receiptNetworkFee": "0.000000000000021",
                            "confirmedNetworkFee": "0.000000000000021"
                        }))
                        .unwrap();
                    service.upsert_history_records(vec![old]).await.unwrap();
                    let changes = service.poll_pending_transactions(chain).await.unwrap();
                    assert_eq!(changes.len(), 1);
                    let polled = service.transactions().await.unwrap().remove(0);
                    service
                        .apply_resolved_pending_statuses(
                            chain,
                            vec![crate::store::ResolvedPendingStatus {
                                id: "pending".into(),
                                status: crate::store::wallet_domain::TransactionStatus::executed(
                                    !is_failed,
                                ),
                                confirmations: None,
                                receipt_block_number: Some(7),
                                evm_receipt_cost: crate::store::EvmReceiptCost::from_receipt(
                                    Some("21000"),
                                    Some("1"),
                                    18,
                                ),
                            }],
                        )
                        .await
                        .unwrap();
                    let supplied = service.transactions().await.unwrap().remove(0);
                    use crate::store::wallet_domain::TransactionStatus;
                    for stored in [polled, supplied] {
                        assert_eq!(
                            stored.status,
                            if is_failed {
                                TransactionStatus::Failed
                            } else {
                                TransactionStatus::Confirmed
                            }
                        );
                        assert_eq!(stored.receipt_block_number, Some(7));
                        assert!(stored.receipt_gas_used.is_none());
                        assert!(stored.receipt_effective_gas_price_gwei.is_none());
                        assert!(stored.receipt_network_fee.is_none());
                        assert!(stored.confirmed_network_fee.is_none());
                    }
                } else {
                    let cost = receipt.cost.unwrap();
                    assert_eq!(cost.gas_used, "21000");
                    assert_eq!(cost.effective_gas_price_gwei, "0.000000001");
                    assert_eq!(cost.network_fee, "0.000000000000021");
                }
                server.verify().await;
            }
        }
    }
}

mod transaction_reads {
    //! Exercise app-facing adapters through protocol fixtures, not their decoders.
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn service(chain: crate::registry::Chain, server: &MockServer) -> Arc<WalletService> {
        WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap()
    }

    #[tokio::test]
    async fn bitcoin_testnet_status_uses_the_selected_network() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tx/hash/status"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"confirmed":true,"block_height":100})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let svc = service(Chain::BitcoinTestnet4, &server);
        let status = svc
            .fetch_utxo_tx_status(Chain::BitcoinTestnet4, "hash".into())
            .await
            .unwrap();
        assert!(status.confirmed);
        assert_eq!(status.block_height, Some(100));
    }

    #[tokio::test]
    async fn replacement_nonce_is_read_from_the_transaction_and_missing_is_an_error() {
        let server = MockServer::start().await;
        let svc = service(crate::registry::Chain::Ethereum, &server);
        for (result, expected) in [
            (json!({"nonce":"0x2a"}), Some(42)),
            (json!(null), None),
            (json!({"nonce":"invalid"}), None),
        ] {
            server.reset().await;
            Mock::given(method("POST"))
                .and(body_partial_json(
                    json!({"method":"eth_getTransactionByHash","params":["hash"]}),
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":result})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let result = svc
                .fetch_evm_tx_nonce(crate::registry::Chain::Ethereum, "hash".into())
                .await;
            match expected {
                Some(n) => assert_eq!(result.unwrap(), n),
                None => assert!(result.is_err()),
            }
        }
    }
}
