//! Exercise app-facing adapters through protocol fixtures, not their decoders.
use super::*;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn service(chain: &str, server: &MockServer) -> Arc<WalletService> {
    WalletService::new_typed(vec![ChainEndpoints {
        chain_id: chain.into(),
        endpoints: vec![server.uri()],
        api_key: None,
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
        .fetch_utxo_fee_preview_typed(
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
        .fetch_utxo_tx_status_typed(Chain::BitcoinTestnet4.str_id().into(), "hash".into())
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
            .fetch_evm_tx_nonce_typed("ethereum".into(), "hash".into())
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
        .fetch_simple_chain_send_preview_typed("solana".into(), "sender".into())
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
        .fetch_simple_chain_send_preview_typed("solana".into(), "sender".into())
        .await
        .is_err());
    assert!(svc
        .fetch_simple_chain_send_preview_typed("ethereum".into(), "sender".into())
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
        .fetch_dogecoin_send_preview_typed("sender".into(), 1.0, "standard".into())
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
    let evaluate = core_evaluate_large_movement;
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
        core_private_key_hex(format!("  0X{}  ", "AB".repeat(32))),
        Some("ab".repeat(32))
    );
    for invalid in ["ab".repeat(31), "gg".repeat(32), String::new()] {
        assert!(core_private_key_hex(invalid).is_none());
    }
}

#[tokio::test]
async fn hd_receive_skips_spent_addresses_and_preview_accounts_for_network_fee() {
    let server = MockServer::start().await;
    let svc = service("bitcoin", &server);
    let xpub=svc.derive_bitcoin_account_xpub_typed(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(),
        String::new(),"m/44'/0'/0'".into()).unwrap();
    let children = crate::derivation::xpub_walker::derive_children(&xpub, 0, 0, 2).unwrap();
    for (index, child) in children.iter().enumerate() {
        Mock::given(method("GET")).and(path(format!("/address/{}",child.address)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "address":child.address,
                "chain_stats":{"funded_txo_sum":0,"spent_txo_sum":0,"tx_count":if index==0 {2}else{0}},
                "mempool_stats":{"funded_txo_sum":0,"spent_txo_sum":0,"tx_count":0}
            }))).mount(&server).await;
    }
    assert_eq!(
        svc.fetch_bitcoin_next_unused_address_typed(xpub.clone(), 0, 2)
            .await
            .unwrap(),
        Some(children[1].address.clone())
    );
    assert_eq!(
        svc.fetch_bitcoin_next_unused_address_typed(xpub.clone(), 0, 1)
            .await
            .unwrap(),
        None
    );
    Mock::given(method("GET"))
        .and(path("/fee-estimates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"3":2.0,"6":2.0})))
        .mount(&server)
        .await;
    let preview = svc
        .fetch_bitcoin_hd_send_preview_typed("bitcoin".into(), xpub.clone(), 2, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preview.spendableBalance, Some(0.0));
    assert_eq!(preview.estimatedFeeRateSatVb, 2);
    assert!(svc
        .fetch_bitcoin_hd_send_preview_typed("ethereum".into(), xpub, 2, 0)
        .await
        .is_err());
}
