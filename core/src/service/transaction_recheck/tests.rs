use super::*;
use crate::service::{ChainEndpoints, StatusPollOutcome};
use crate::store::wallet_domain::CoreTransactionStatus;
use serde_json::json;
use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

fn record(id: &str, chain: Chain, status: &str) -> CorePersistedTransactionRecord {
    serde_json::from_value(json!({
        "id":id, "walletId":"wallet", "walletName":"Original", "kind":"send",
        "chainName":chain.chain_display_name(), "symbol":chain.coin_symbol(), "assetName":"Coin",
        "status":status, "amount":1, "address":"recipient", "createdAt":1234.0,
        "transactionHash":"ab".repeat(32), "failureReason":"old failure",
        "receiptBlockNumber":90, "confirmationCount":99
    }))
    .unwrap()
}
async fn service(chain: Chain, server: &MockServer) -> (std::sync::Arc<WalletService>, String) {
    let service = WalletService::new_typed(vec![ChainEndpoints {
        chain_id: chain.str_id().into(),
        endpoints: vec![server.uri()],
        api_key: None,
    }])
    .unwrap();
    let path = std::env::temp_dir()
        .join(format!(
            "spectra-recheck-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(path.clone()).await.unwrap();
    (service, path)
}
async fn save(service: &WalletService, record: CorePersistedTransactionRecord) {
    service
        .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(record)])
        .await
        .unwrap();
}

#[tokio::test]
async fn explicit_recheck_targets_failed_and_confirmed_records_on_the_stored_network() {
    let server = MockServer::start().await;
    let (service, path) = service(Chain::BitcoinTestnet4, &server).await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"confirmed":true,"block_height":123})),
        )
        .expect(2)
        .mount(&server)
        .await;
    for previous in ["failed", "confirmed"] {
        save(&service, record("TARGET", Chain::BitcoinTestnet4, previous)).await;
        save(
            &service,
            record("unrelated", Chain::BitcoinTestnet4, "pending"),
        )
        .await;
        service
            .record_status_poll(
                "TARGET".into(),
                StatusPollOutcome::Confirmed {
                    confirmations: Some(99),
                },
            )
            .await;
        let change = service
            .recheck_transaction_status("target".into())
            .await
            .unwrap();
        assert_eq!(change.old_status, previous);
        assert_eq!(change.new_status, "confirmed");
        assert_eq!(change.status_changed, previous != "confirmed");
        let reopened = WalletService::new_typed(vec![]).unwrap();
        reopened.open_state(path.clone()).await.unwrap();
        let rows = reopened.fetch_all_history_records_typed().await.unwrap();
        let target = rows.iter().find(|r| r.id == "target").unwrap();
        assert_eq!(target.payload.receipt_block_number, Some(123));
        assert_eq!(target.payload.failure_reason, None);
        assert_eq!(target.created_at, 1234.0 + 978307200.0);
        assert_eq!(
            rows.iter()
                .find(|r| r.id == "unrelated")
                .unwrap()
                .payload
                .status,
            Some(CoreTransactionStatus::Pending)
        );
    }
    server.verify().await;
}

#[tokio::test]
async fn explicit_recheck_reopens_finality_and_clears_reorg_metadata() {
    let server = MockServer::start().await;
    let (service, _) = service(Chain::Dogecoin, &server).await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"hash":"ab".repeat(32),"block_height":-1,"confirmations":0})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut row = record("target", Chain::Dogecoin, "confirmed");
    row.dogecoin_confirmed_network_fee_doge = Some(1.0);
    save(&service, row).await;
    service
        .record_status_poll(
            "target".into(),
            StatusPollOutcome::Confirmed {
                confirmations: Some(99),
            },
        )
        .await;
    let change = service
        .recheck_transaction_status("target".into())
        .await
        .unwrap();
    assert_eq!(change.new_status, "pending");
    let row = service.transactions().await.unwrap().remove(0);
    assert_eq!(row.receipt_block_number, None);
    assert_eq!(row.confirmation_count, Some(0));
    assert_eq!(row.dogecoin_confirmed_network_fee_doge, None);
    assert!(!service.status_trackers.read().await["target"].reached_finality);
    server.verify().await;
}

#[tokio::test]
async fn explicit_recheck_refuses_invalid_scope_before_network_or_tracker_mutation() {
    let server = MockServer::start().await;
    let (service, _) = service(Chain::Bitcoin, &server).await;
    assert!(WalletService::new_typed(vec![])
        .unwrap()
        .recheck_transaction_status("missing".into())
        .await
        .is_err());
    assert!(service
        .recheck_transaction_status("missing".into())
        .await
        .is_err());
    for chain in [Chain::Ethereum, Chain::Bitcoin] {
        let mut row = record("target", chain, "pending");
        if chain == Chain::Bitcoin {
            row.transaction_hash = Some("  ".into());
        }
        save(&service, row).await;
        assert!(service
            .recheck_transaction_status("target".into())
            .await
            .is_err());
    }
    let mut row = record("target", Chain::Bitcoin, "pending");
    row.kind = crate::store::wallet_domain::CoreTransactionKind::Receive;
    save(&service, row).await;
    assert!(service
        .recheck_transaction_status("target".into())
        .await
        .is_err());
    row = record("receive", Chain::Litecoin, "failed");
    row.kind = crate::store::wallet_domain::CoreTransactionKind::Receive;
    assert!(recheck_chain(&row).is_ok());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(service.status_trackers.read().await.is_empty());
}

#[tokio::test]
async fn explicit_recheck_failed_or_mismatched_provider_preserves_saved_state() {
    for response in [
        json!({"garbage":true}),
        json!({"hash":"cd".repeat(32),"block_height":5,"confirmations":99}),
    ] {
        let server = MockServer::start().await;
        let (service, _) = service(Chain::Dogecoin, &server).await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        save(&service, record("target", Chain::Dogecoin, "failed")).await;
        service
            .record_status_poll("target".into(), StatusPollOutcome::Failed)
            .await;
        let before = serde_json::to_value(service.transactions().await.unwrap()).unwrap();
        let tracker =
            serde_json::to_value(&service.status_trackers.read().await["target"]).unwrap();
        assert!(service
            .recheck_transaction_status("target".into())
            .await
            .is_err());
        assert_eq!(
            serde_json::to_value(service.transactions().await.unwrap()).unwrap(),
            before
        );
        assert_eq!(
            serde_json::to_value(&service.status_trackers.read().await["target"]).unwrap(),
            tracker
        );
    }
}

#[tokio::test]
async fn explicit_recheck_does_not_resurrect_deleted_or_overwrite_changed_transactions() {
    for action in ["delete", "hash", "metadata"] {
        let server = MockServer::start().await;
        let (service, path) = service(Chain::Bitcoin, &server).await;
        save(&service, record("target", Chain::Bitcoin, "failed")).await;
        Mock::given(any())
            .respond_with(move |_: &Request| {
                if action == "delete" {
                    crate::wallet_db::history_delete(&path, &["target".into()]).unwrap();
                } else {
                    let mut row = crate::wallet_db::history_fetch_all(&path)
                        .unwrap()
                        .remove(0);
                    if action == "hash" {
                        row.payload.transaction_hash = Some("cd".repeat(32));
                    } else {
                        row.payload.wallet_name = "Edited during read".into();
                    }
                    crate::wallet_db::history_upsert_batch(&path, &[row]).unwrap();
                }
                ResponseTemplate::new(200)
                    .set_body_json(json!({"confirmed":true,"block_height":123}))
            })
            .expect(1)
            .mount(&server)
            .await;
        let result = service.recheck_transaction_status("target".into()).await;
        let rows = service.transactions().await.unwrap();
        match action {
            "delete" => {
                assert!(result.is_err());
                assert!(rows.is_empty());
            }
            "hash" => {
                assert!(result.is_err());
                assert_eq!(rows[0].status, Some(CoreTransactionStatus::Failed));
            }
            _ => {
                assert!(result.is_ok());
                assert_eq!(rows[0].wallet_name, "Edited during read");
            }
        }
        if action != "metadata" {
            assert!(service.status_trackers.read().await.is_empty());
        }
        server.verify().await;
    }
}
