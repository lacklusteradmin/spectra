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
            ResponseTemplate::new(200).set_body_json(json!({"confirmed":true,"block_height":100})),
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

#[tokio::test]
async fn simple_preview_subtracts_native_fee_and_propagates_unread_balance() {
    let server = MockServer::start().await;
    let svc = service(crate::registry::Chain::Solana, &server);
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getBalance"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":{"value":2_000_000_000u64}})),
        )
        .mount(&server)
        .await;
    let result = svc
        .fetch_simple_chain_send_preview(crate::registry::Chain::Solana, "sender".into())
        .await
        .unwrap()
        .unwrap();
    let crate::send::preview_decode::SimpleChainPreview::Solana { preview } = result else {
        panic!("wrong chain")
    };
    assert_eq!(preview.estimatedNetworkFee, "0.000005");
    assert_eq!(preview.maxSendable, "1.999995");
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"unavailable"}}),
        ))
        .mount(&server)
        .await;
    assert!(
        svc.fetch_simple_chain_send_preview(crate::registry::Chain::Solana, "sender".into())
            .await
            .is_err()
    );
    assert!(
        svc.fetch_simple_chain_send_preview(crate::registry::Chain::Ethereum, "sender".into())
            .await
            .is_err()
    );
}

#[test]
fn movement_alert_requires_both_thresholds_and_valid_observations() {
    let evaluate = evaluate_large_movement;
    assert!(!evaluate(100.0, 109.0, 10.0, 5.0).should_alert);
    assert!(!evaluate(1000.0, 1020.0, 10.0, 5.0).should_alert);
    let down = evaluate(100.0, 80.0, 10.0, 5.0);
    assert!(down.should_alert && !down.direction_up);
    assert_eq!(down.absolute_delta, 20.0);
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        assert!(!evaluate(100.0, invalid, 10.0, 5.0).should_alert);
        assert!(!evaluate(100.0, 200.0, invalid, 5.0).should_alert);
    }
}

#[tokio::test]
async fn owned_non_evm_preview_needs_only_stored_watch_address_and_valid_input() {
    let server = MockServer::start().await;
    let svc = service(crate::registry::Chain::Solana, &server);
    let address = "11111111111111111111111111111111";
    let mut wallet = crate::store::state::WalletState::single_address(
        "watch",
        "Watch",
        crate::registry::Chain::Solana,
        address,
        None,
        true,
    );
    wallet
        .holdings
        .push(crate::store::wallet_domain::AssetHolding {
            name: "Solana".into(),
            symbol: "SOL".into(),
            chain_id: crate::registry::Chain::Solana,
            token_standard: "Native".into(),
            amount: "2".into(),
            id: String::new(),
            coingecko_id: String::new(),
            contract_address: None,
        });
    svc.wallet_state.write().await.wallets.push(wallet);
    for (amount, nonce) in [("0.0000000001", None), ("NaN", None), ("1", Some(1))] {
        assert!(
            svc.preview_owned_send(
                "watch".into(),
                "solana:native".into(),
                amount.into(),
                "".into(),
                nonce,
                None
            )
            .await
            .is_err()
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({"method":"getBalance","params":[address]}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":{"value":2_000_000_000u64}})),
        )
        .mount(&server)
        .await;
    let result = svc
        .preview_owned_send(
            "watch".into(),
            "solana:native".into(),
            "1".into(),
            "".into(),
            None,
            None,
        )
        .await
        .unwrap();
    let Some(crate::send::flow::SendPreview::Solana { preview }) = result.map(|quote| {
        assert!(quote.shortcuts.contains_key(&100));
        assert_eq!(quote.chain_id, crate::registry::Chain::Solana);
        quote.preview
    }) else {
        panic!("wrong preview")
    };
    assert_eq!(preview.maxSendable, "1.999995");
    svc.wallet_state.write().await.wallets[0].addresses.clear();
    let count = server.received_requests().await.unwrap().len();
    assert!(
        svc.preview_owned_send(
            "watch".into(),
            "solana:native".into(),
            "1".into(),
            "".into(),
            None,
            None
        )
        .await
        .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), count);
}

#[tokio::test]
async fn one_blockbook_adapter_reads_each_network_and_keeps_bch_address_rules() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/address/holder"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"balance":"123456789"})))
        .expect(5)
        .mount(&server)
        .await;
    for chain in [
        Chain::Litecoin,
        Chain::BitcoinCashTestnet,
        Chain::BitcoinGold,
        Chain::Dash,
        Chain::Zcash,
    ] {
        let address = if chain.mainnet_counterpart() == Chain::BitcoinCash {
            "bitcoincash:holder"
        } else {
            "holder"
        };
        let balance = service(chain, &server)
            .fetch_native_balance_summary(chain, address.into())
            .await
            .unwrap();
        assert_eq!(balance.smallest_unit, "123456789");
        assert_eq!(balance.amount_display, "1.23456789");
    }
}

#[tokio::test]
async fn balance_summary_keeps_sub_micro_native_amounts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"eth_getBalance"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":"0x1"})))
        .mount(&server)
        .await;
    let balance = service(crate::registry::Chain::Ethereum, &server)
        .fetch_native_balance_summary(crate::registry::Chain::Ethereum, "holder".into())
        .await
        .unwrap();
    assert_eq!(balance.amount_display, "0.000000000000000001");
}

/// No keyless source answers Substrate history: asking is an error, never an
/// empty list.
#[tokio::test]
async fn substrate_history_is_an_error_not_an_empty_list() {
    let service = WalletService::new(vec![]).unwrap();
    for chain in [
        crate::registry::Chain::Polkadot,
        crate::registry::Chain::Bittensor,
    ] {
        let history = service
            .fetch_history(chain, "address".into())
            .await
            .unwrap_err()
            .to_string();
        assert!(history.contains("no keyless history source"), "{history}");
    }
}

/// Storage is decoded only after Asset Hub identity and its layout are verified.
#[tokio::test]
async fn a_polkadot_balance_is_read_from_system_account_storage() {
    let server = MockServer::start().await;
    for (request, result) in [
        (
            json!({"method":"chain_getBlockHash","params":[0]}),
            json!(
                crate::registry::Chain::Polkadot
                    .substrate_genesis_hash()
                    .unwrap()
            ),
        ),
        (
            json!({"method":"chain_getBlockHash","params":[]}),
            json!(format!("0x{}", "11".repeat(32))),
        ),
        (
            json!({"method":"chain_getFinalizedHead"}),
            json!(format!("0x{}", "11".repeat(32))),
        ),
        (
            json!({"method":"chain_getHeader"}),
            json!({"number":"0x64"}),
        ),
        (
            json!({"method":"state_getRuntimeVersion"}),
            json!({"specVersion":2_005_000,"transactionVersion":15}),
        ),
        (
            json!({"method":"state_getMetadata"}),
            json!(format!(
                "0x{}",
                hex::encode(crate::api::substrate_json_rpc::tests::fixture(
                    crate::registry::Chain::Polkadot
                ))
            )),
        ),
    ] {
        Mock::given(method("POST"))
            .and(body_partial_json(request))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":result})),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method": "state_getStorage"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0", "id": 1,
            "result": "0x000000000000000001000000000000001a8ea401a31900000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000080"
        })))
        .mount(&server)
        .await;
    let summary = service(crate::registry::Chain::Polkadot, &server)
        .fetch_native_balance_summary(
            crate::registry::Chain::Polkadot,
            "13UVJyLnbVp9RBZYFwFGyDvVd1y27Tt8tkntv6Q7JVPhFsTB".into(),
        )
        .await
        .unwrap();
    assert_eq!(summary.smallest_unit, "28187897925146");
    assert_eq!(summary.amount_display, "2818.7897925146");
}
