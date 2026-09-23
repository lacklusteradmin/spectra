//! Exercise app-facing adapters through protocol fixtures, not their decoders.
use super::*;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn service(chain: &str, server: &MockServer) -> Arc<WalletService> {
    WalletService::new(vec![ChainEndpoints {
        capabilities: crate::app_core::ENDPOINT_CAPABILITIES
            .map(String::from)
            .to_vec(),
        chain_id: chain.into(),
        endpoints: vec![server.uri()],
    }])
    .unwrap()
}

#[tokio::test]
async fn bitcoin_testnet_preview_and_status_use_the_selected_network() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/address/sender/utxo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"txid":"ab","vout":0,"value":100000,"status":{"confirmed":true}},
            {"txid":"cd","vout":0,"value":1,"status":{"confirmed":false}}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tx/hash/status"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"confirmed":true,"block_height":100})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let svc = service(Chain::BitcoinTestnet4.str_id(), &server);
    let preview = svc
        .fetch_utxo_fee_preview(
            Chain::BitcoinTestnet4.str_id().into(),
            "sender".into(),
            2,
            "destination".into(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preview.selectedInputCount, Some(1), "dust is not spendable");
    assert_eq!(preview.estimatedNetworkFee, 384.0 / 100_000_000.0);
    assert_eq!(preview.maxSendable, Some(99616.0 / 100_000_000.0));
    let status = svc
        .fetch_utxo_tx_status(Chain::BitcoinTestnet4.str_id().into(), "hash".into())
        .await
        .unwrap();
    assert!(status.confirmed);
    assert_eq!(status.block_height, Some(100));
}

#[tokio::test]
async fn replacement_nonce_is_read_from_the_transaction_and_missing_is_an_error() {
    let server = MockServer::start().await;
    let svc = service("ethereum", &server);
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
            .fetch_evm_tx_nonce("ethereum".into(), "hash".into())
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
    let svc = service("solana", &server);
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getBalance"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":{"value":2_000_000_000u64}})),
        )
        .mount(&server)
        .await;
    let result = svc
        .fetch_simple_chain_send_preview("solana".into(), "sender".into())
        .await
        .unwrap();
    let crate::send::preview_decode::SimpleChainPreview::Solana { preview } = result else {
        panic!("wrong chain")
    };
    assert_eq!(preview.estimatedNetworkFee, 0.000005);
    assert_eq!(preview.maxSendable, 1.999995);
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"unavailable"}}),
        ))
        .mount(&server)
        .await;
    assert!(svc
        .fetch_simple_chain_send_preview("solana".into(), "sender".into())
        .await
        .is_err());
    assert!(svc
        .fetch_simple_chain_send_preview("ethereum".into(), "sender".into())
        .await
        .is_err());
}

#[tokio::test]
async fn dogecoin_preview_excludes_spent_outputs_and_preserves_requested_amount() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/addrs/sender"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"txrefs":[
            {"tx_hash":"a","tx_output_n":0,"value":200_000_000,"spent":false},
            {"tx_hash":"b","tx_output_n":0,"value":900_000_000,"spent":true}
        ]})))
        .mount(&server)
        .await;
    let preview = service("dogecoin", &server)
        .fetch_dogecoin_send_preview("sender".into(), 1.0, "standard".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preview.requestedAmountDoge, 1.0);
    assert!(preview.maxSendableDoge <= 2.0);
    assert!(preview.maxSendableDoge > 1.0);
    assert_eq!(preview.selectedInputCount, 1);
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

#[test]
fn private_key_editor_normalizes_only_a_complete_hex_key() {
    assert_eq!(
        private_key_hex(format!("  0X{}  ", "AB".repeat(32))),
        Some("ab".repeat(32))
    );
    for invalid in ["ab".repeat(31), "gg".repeat(32), String::new()] {
        assert!(!is_private_key_hex(invalid.clone()));
        assert!(private_key_hex(invalid).is_none());
    }
    assert!(is_private_key_hex(format!("0x{}", "ab".repeat(32))));
}

#[tokio::test]
async fn owned_non_evm_preview_needs_only_stored_watch_address_and_valid_input() {
    let server = MockServer::start().await;
    let svc = service("solana", &server);
    let address = "11111111111111111111111111111111";
    let mut wallet = crate::store::state::WalletState::single_address(
        "watch", "Watch", "Solana", address, None, true,
    );
    wallet
        .holdings
        .push(crate::store::wallet_domain::AssetHolding {
            name: "Solana".into(),
            symbol: "SOL".into(),
            chain_name: "Solana".into(),
            token_standard: "Native".into(),
            amount: 2.0,
            ..Default::default()
        });
    svc.wallet_state.write().await.wallets.push(wallet);
    for (amount, nonce) in [("0.0000000001", None), ("NaN", None), ("1", Some(1))] {
        assert!(svc
            .preview_owned_send(
                "watch".into(),
                "solana:native".into(),
                amount.into(),
                "".into(),
                nonce,
                None
            )
            .await
            .is_err());
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
        assert_eq!(quote.chain_id, "solana");
        quote.preview
    }) else {
        panic!("wrong preview")
    };
    assert_eq!(preview.maxSendable, 1.999995);
    svc.wallet_state.write().await.wallets[0].addresses.clear();
    let count = server.received_requests().await.unwrap().len();
    assert!(svc
        .preview_owned_send(
            "watch".into(),
            "solana:native".into(),
            "1".into(),
            "".into(),
            None,
            None
        )
        .await
        .is_err());
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
        let balance = service(chain.str_id(), &server)
            .fetch_native_balance_summary(chain.str_id().into(), address.into())
            .await
            .unwrap();
        assert_eq!(balance.smallest_unit, "123456789");
        assert_eq!(balance.amount_display, "1.23456789");
    }
    let client = BlockbookClient::new(Arc::new(vec![server.uri()]), Chain::Dash);
    assert!(client
        .sign_litecoin_and_broadcast("from", "to", 1, 1, &[], None)
        .await
        .unwrap_err()
        .contains("network"));
}

#[tokio::test]
async fn balance_summary_keeps_sub_micro_native_amounts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"eth_getBalance"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":"0x1"})))
        .mount(&server)
        .await;
    let balance = service("ethereum", &server)
        .fetch_native_balance_summary("ethereum".into(), "holder".into())
        .await
        .unwrap();
    assert_eq!(balance.amount_display, "0.000000000000000001");
}
