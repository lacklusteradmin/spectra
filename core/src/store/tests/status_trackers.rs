use crate::service::WalletService;
use crate::store::persistence_models::TransactionRecord;

fn tmp_db(tag: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-status-{tag}-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    path.to_string_lossy().into_owned()
}

fn pending_send(id: &str, chain: crate::registry::Chain) -> TransactionRecord {
    serde_json::from_value(serde_json::json!({
        "id": id, "walletId": "w1", "kind": "send", "status": "pending",
        "walletName": "W", "assetDisplayName": chain, "symbol": "BTC",
        "chainId": chain, "amount": "1", "address": "bc1qexample",
        "transactionHash": format!("hash-{id}"), "createdAtUnix": 0.0,
    }))
    .expect("fixture must match TransactionRecord")
}

// ── Confirmation-poll trackers (core-owned) ───────────────────────────

#[tokio::test]
async fn untracked_transaction_is_always_due_for_poll() {
    let service = WalletService::new(Vec::new()).expect("service");
    let due = service
        .transactions_due_for_status_poll(vec!["tx1".into(), "tx2".into()])
        .await;
    assert_eq!(due, vec!["tx1".to_string(), "tx2".to_string()]);
}

#[tokio::test]
async fn a_polled_transaction_waits_out_its_interval() {
    let service = WalletService::new(Vec::new()).expect("service");
    service
        .record_status_poll("tx1".into(), crate::service::StatusPollOutcome::Pending)
        .await;
    assert!(
        service
            .transactions_due_for_status_poll(vec!["tx1".into()])
            .await
            .is_empty(),
        "polled just now, and the pending interval is twenty seconds"
    );
}

/// Applying a resolution writes the record and reports the change; this reads
/// it back from the database.
#[tokio::test]
async fn applying_a_resolution_stores_it_and_reports_the_change() {
    use crate::store::ResolvedPendingStatus;
    let service = WalletService::new(Vec::new()).expect("service");
    service
        .open_state(tmp_db("apply-resolved"))
        .await
        .expect("open");
    service
        .upsert_history_records(vec![crate::wallet_db::HistoryRecord {
            id: "tx1".into(),
            wallet_id: Some("w1".into()),
            chain_id: crate::registry::Chain::Bitcoin,
            tx_hash: Some("hash-tx1".into()),
            created_at: 0.0,
            payload: pending_send("tx1", crate::registry::Chain::Bitcoin),
        }])
        .await
        .expect("store");

    let changes = service
        .apply_resolved_pending_statuses(
            crate::registry::Chain::Bitcoin,
            vec![ResolvedPendingStatus {
                id: "tx1".into(),
                status: "confirmed".into(),
                confirmations: Some(6),
                receipt_block_number: Some(900_000),
                evm_receipt_cost: None,
            }],
        )
        .await
        .expect("apply");

    assert_eq!(changes.len(), 1);
    use crate::store::wallet_domain::TransactionStatus;
    assert_eq!(changes[0].old_status, TransactionStatus::Pending);
    assert_eq!(changes[0].new_status, TransactionStatus::Confirmed);
    assert!(changes[0].status_changed);
    assert_eq!(changes[0].transaction_hash.as_deref(), Some("hash-tx1"));

    let stored = service.transactions().await.expect("read");
    let tx = stored.iter().find(|t| t.id == "tx1").expect("still there");
    assert_eq!(
        tx.status,
        crate::store::wallet_domain::TransactionStatus::Confirmed
    );
    assert_eq!(tx.confirmation_count, Some(6));
    assert_eq!(tx.receipt_block_number, Some(900_000));

    // Applying the same resolution again is not a change.
    let again = service
        .apply_resolved_pending_statuses(
            crate::registry::Chain::Bitcoin,
            vec![ResolvedPendingStatus {
                id: "tx1".into(),
                status: "confirmed".into(),
                confirmations: Some(6),
                receipt_block_number: None,
                evm_receipt_cost: None,
            }],
        )
        .await
        .expect("apply");
    assert!(!again[0].status_changed);
}

/// Pruning drops trackers for transactions core does not hold, judged from
/// core's own table.
#[tokio::test]
async fn pruning_drops_trackers_for_transactions_that_no_longer_exist() {
    let service = WalletService::new(Vec::new()).expect("service");
    for id in ["tx1", "tx2"] {
        service
            .record_status_poll(id.into(), crate::service::StatusPollOutcome::Pending)
            .await;
    }
    // No database is bound, so core cannot say which transactions exist.
    // It refuses rather than reading that as "none exist" — dropping a live
    // tracker stops a pending send from ever being polled again, and a
    // stale one only costs a poll.
    assert!(service.prune_status_trackers().await.is_err());
    assert!(
        service
            .transactions_due_for_status_poll(vec!["tx1".into(), "tx2".into()])
            .await
            .is_empty(),
        "a failed prune dropped trackers it could not verify"
    );
}
