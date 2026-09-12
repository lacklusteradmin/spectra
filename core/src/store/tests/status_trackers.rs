use crate::service::WalletService;
use crate::store::persistence_models::CorePersistedTransactionRecord;

fn tmp_db(tag: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-status-{tag}-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    path.to_string_lossy().into_owned()
}

fn pending_send(id: &str, chain: &str) -> CorePersistedTransactionRecord {
    serde_json::from_value(serde_json::json!({
        "id": id, "walletId": "w1", "kind": "send", "status": "pending",
        "walletName": "W", "assetName": chain, "symbol": "BTC",
        "chainName": chain, "amount": 1.0, "address": "bc1qexample",
        "transactionHash": format!("hash-{id}"), "createdAt": 0.0,
    }))
    .expect("fixture must match CorePersistedTransactionRecord")
}

// ── Confirmation-poll trackers (core-owned) ───────────────────────────

#[tokio::test]
async fn untracked_transaction_is_always_due_for_poll() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let due = service
        .transactions_due_for_status_poll(vec!["tx1".into(), "tx2".into()])
        .await;
    assert_eq!(due, vec!["tx1".to_string(), "tx2".to_string()]);
}

#[tokio::test]
async fn a_polled_transaction_waits_out_its_interval() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
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

/// Applying a resolution writes the record and reports the change.
///
/// Core used to hand back a decision and the caller built the new record
/// and stored it, so nothing on either side asserted that what came out of
/// the planner reached the database. It does now, and this reads it back.
#[tokio::test]
async fn applying_a_resolution_stores_it_and_reports_the_change() {
    use crate::store::ResolvedPendingStatus;
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service
        .open_state(tmp_db("apply-resolved"))
        .await
        .expect("open");
    service
        .upsert_history_records(vec![crate::wallet_db::HistoryRecord {
            id: "tx1".into(),
            wallet_id: Some("w1".into()),
            chain_name: "Bitcoin".into(),
            tx_hash: Some("hash-tx1".into()),
            created_at: 0.0,
            payload: pending_send("tx1", "Bitcoin"),
        }])
        .await
        .expect("store");

    let changes = service
        .apply_resolved_pending_statuses(
            "Bitcoin".into(),
            vec![ResolvedPendingStatus {
                id: "tx1".into(),
                status: "confirmed".into(),
                confirmations: Some(6),
                receipt_block_number: Some(900_000),
                dogecoin_network_fee_doge: None,
            }],
        )
        .await
        .expect("apply");

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].old_status, "pending");
    assert_eq!(changes[0].new_status, "confirmed");
    assert!(changes[0].status_changed);
    assert_eq!(changes[0].emit_event_code.as_deref(), Some("confirmed"));
    assert_eq!(changes[0].transaction_hash.as_deref(), Some("hash-tx1"));

    let stored = service.transactions().await.expect("read");
    let tx = stored.iter().find(|t| t.id == "tx1").expect("still there");
    assert_eq!(
        tx.status,
        Some(crate::store::wallet_domain::CoreTransactionStatus::Confirmed)
    );
    assert_eq!(tx.confirmation_count, Some(6));
    assert_eq!(tx.receipt_block_number, Some(900_000));

    // A transaction given up on stores a code, not a sentence: the text a
    // user reads is localized at render, so changing language does not
    // leave old records in the old one.
    assert_eq!(crate::store::FAILURE_REASON_STUCK, "stuckAfterRetries");

    // Applying the same resolution again is not a change.
    let again = service
        .apply_resolved_pending_statuses(
            "Bitcoin".into(),
            vec![ResolvedPendingStatus {
                id: "tx1".into(),
                status: "confirmed".into(),
                confirmations: Some(6),
                receipt_block_number: None,
                dogecoin_network_fee_doge: None,
            }],
        )
        .await
        .expect("apply");
    assert!(!again[0].status_changed);
}

/// Age alone is not failure. A transaction is given up on only after it is
/// both old and has failed to resolve repeatedly.
///
/// Goes through the store: the service reads its own transactions to find
/// the candidates, so a test that handed it a synthetic input list would
/// no longer exercise the path the app takes.
#[tokio::test]
async fn stale_pending_needs_both_age_and_repeated_failures() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service
        .open_state(tmp_db("stale-pending"))
        .await
        .expect("open");
    service
        .upsert_history_records(vec![crate::wallet_db::HistoryRecord {
            id: "tx1".into(),
            wallet_id: Some("w1".into()),
            chain_name: "Bitcoin".into(),
            tx_hash: Some("hash-tx1".into()),
            created_at: 0.0,
            payload: pending_send("tx1", "Bitcoin"),
        }])
        .await
        .expect("store");

    assert!(
        service
            .stale_pending_failure_ids("Bitcoin".into())
            .await
            .expect("read")
            .is_empty(),
        "old enough, but it has never failed a poll"
    );

    for _ in 0..6 {
        service
            .record_status_poll("tx1".into(), crate::service::StatusPollOutcome::Failed)
            .await;
    }
    assert_eq!(
        service
            .stale_pending_failure_ids("Bitcoin".into())
            .await
            .expect("read"),
        vec!["tx1".to_string()]
    );

    // Another chain's sweep must not pick it up.
    assert!(service
        .stale_pending_failure_ids("Litecoin".into())
        .await
        .expect("read")
        .is_empty());
}

/// Pruning drops trackers for transactions core does not hold.
///
/// It took the ids to keep, which meant the front end filtered core's own
/// transaction table and told core the answer.
#[tokio::test]
async fn pruning_drops_trackers_for_transactions_that_no_longer_exist() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
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
