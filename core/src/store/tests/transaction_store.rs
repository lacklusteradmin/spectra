use crate::service::{TransactionCommand, WalletService};
use crate::store::persistence_models::CorePersistedTransactionRecord;

fn tmp_db(tag: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-tx-store-{tag}-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

async fn opened(tag: &str) -> (std::sync::Arc<WalletService>, String) {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let db = tmp_db(tag);
    service.open_state(db.clone()).await.expect("open");
    (service, db)
}

fn record(id: &str, wallet: &str, status: &str) -> CorePersistedTransactionRecord {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "walletId": wallet,
        "kind": "send",
        "status": status,
        "walletName": "W",
        "assetName": "Bitcoin",
        "symbol": "BTC",
        "chainName": "Bitcoin",
        "amount": 1.0,
        "address": "bc1qexample",
        "transactionHash": format!("hash-{id}"),
        "createdAt": 0.0,
    }))
    .expect("fixture must match CorePersistedTransactionRecord")
}

/// The whole point: core decides what is new and what is an update.
#[tokio::test]
async fn upsert_reports_added_then_updated_for_the_same_id() {
    let (service, db) = opened("delta").await;

    let first = service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![record("tx1", "w1", "pending")],
        })
        .await
        .expect("upsert");
    assert_eq!(first.added, vec!["tx1"]);
    assert!(first.updated.is_empty());

    let second = service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![record("tx1", "w1", "confirmed")],
        })
        .await
        .expect("upsert");
    assert!(second.added.is_empty(), "same id is not a new record");
    assert_eq!(second.updated, vec!["tx1"]);

    // And the update actually landed — the failure mode a caller-computed
    // delta produces is a silently dropped status change.
    let stored = service.transactions().await.expect("read");
    assert_eq!(stored.len(), 1);
    assert_eq!(
        serde_json::to_value(stored[0].status).unwrap().as_str(),
        Some("confirmed")
    );

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn a_mixed_batch_is_split_into_added_and_updated() {
    let (service, db) = opened("mixed").await;
    service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![record("tx1", "w1", "pending")],
        })
        .await
        .expect("upsert");

    let change = service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![
                record("tx1", "w1", "confirmed"),
                record("tx2", "w1", "pending"),
            ],
        })
        .await
        .expect("upsert");
    assert_eq!(change.updated, vec!["tx1"]);
    assert_eq!(change.added, vec!["tx2"]);

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn ids_are_matched_case_insensitively() {
    let (service, db) = opened("case").await;
    service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![record("ABC-123", "w1", "pending")],
        })
        .await
        .expect("upsert");
    let change = service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![record("abc-123", "w1", "confirmed")],
        })
        .await
        .expect("upsert");
    assert_eq!(change.updated, vec!["abc-123"], "not a second record");
    assert_eq!(service.transactions().await.expect("read").len(), 1);

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn removes_by_id_by_wallet_and_wholesale() {
    let (service, db) = opened("remove").await;
    service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![
                record("tx1", "w1", "confirmed"),
                record("tx2", "w1", "confirmed"),
                record("tx3", "w2", "confirmed"),
            ],
        })
        .await
        .expect("upsert");

    let removed = service
        .apply_transaction_command(TransactionCommand::Remove {
            ids: vec!["tx1".to_string()],
        })
        .await
        .expect("remove");
    assert_eq!(removed.removed, vec!["tx1"]);

    assert_eq!(
        service
            .transactions_for_wallet("w1".to_string())
            .await
            .expect("read")
            .len(),
        1
    );

    let by_wallet = service
        .apply_transaction_command(TransactionCommand::RemoveForWallet {
            wallet_id: "w1".to_string(),
        })
        .await
        .expect("remove");
    assert_eq!(by_wallet.removed, vec!["tx2"]);
    assert_eq!(service.transactions().await.expect("read").len(), 1);

    let cleared = service
        .apply_transaction_command(TransactionCommand::Clear)
        .await
        .expect("clear");
    assert_eq!(cleared.removed, vec!["tx3"]);
    assert!(service.transactions().await.expect("read").is_empty());

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn removing_what_is_absent_is_not_a_change() {
    let (service, db) = opened("absent").await;
    let change = service
        .apply_transaction_command(TransactionCommand::Remove {
            ids: vec!["nope".to_string()],
        })
        .await
        .expect("remove");
    assert!(change.is_empty());
    let _ = std::fs::remove_file(&db);
}

/// Without a bound store the error says what to do, rather than writing
/// somewhere nobody will look.
#[tokio::test]
async fn commands_require_an_opened_store() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let error = service
        .apply_transaction_command(TransactionCommand::Clear)
        .await
        .expect_err("must refuse");
    assert!(
        error.to_string().contains("open_state"),
        "unhelpful error: {error}"
    );
}

/// `created_at` is Swift reference time in the record and Unix time in the
/// indexed column. Getting that wrong misorders history by 31 years.
#[tokio::test]
async fn created_at_is_converted_to_unix_time_for_the_index() {
    let (service, db) = opened("epoch").await;
    service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![record("tx1", "w1", "confirmed")],
        })
        .await
        .expect("upsert");

    let rows = crate::wallet_db::history_fetch_all(&db).expect("rows");
    // createdAt 0.0 in Swift reference time is 2001-01-01 UTC.
    assert_eq!(rows[0].created_at, 978_307_200.0);
    // The record itself keeps the reference-time value it was given.
    assert_eq!(rows[0].payload.created_at, 0.0);

    let _ = std::fs::remove_file(&db);
}
