use crate::service::WalletService;
use crate::store::persistence_models::TransactionRecord;

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
    let service = WalletService::new(Vec::new()).expect("service");
    let db = tmp_db(tag);
    service.open_state(db.clone()).await.expect("open");
    (service, db)
}

fn record(id: &str, wallet: &str, status: &str) -> TransactionRecord {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "walletId": wallet,
        "kind": "send",
        "status": status,
        "walletName": "W",
        "assetDisplayName": "Bitcoin",
        "symbol": "BTC",
        "chainId": "bitcoin",
        "amount": "1",
        "address": "bc1qexample",
        "transactionHash": format!("hash-{id}"),
        "createdAtUnix": 0.0,
    }))
    .expect("fixture must match TransactionRecord")
}

#[tokio::test]
async fn ids_are_matched_case_insensitively() {
    let (service, db) = opened("case").await;
    service
        .upsert_history_records(vec![record("ABC-123", "w1", "pending")])
        .await
        .expect("upsert");
    service
        .upsert_history_records(vec![record("abc-123", "w1", "confirmed")])
        .await
        .expect("upsert");
    let stored = service.transactions().await.expect("read");
    assert_eq!(stored.len(), 1, "not a second record");
    assert_eq!(
        serde_json::to_value(stored[0].status).unwrap().as_str(),
        Some("confirmed")
    );

    let _ = std::fs::remove_file(&db);
}

/// Clearing deletes what it cannot decode: a record written by another build
/// would otherwise fail a reset after its wallets were already removed.
#[tokio::test]
async fn clearing_removes_records_this_build_cannot_read() {
    let (service, db) = opened("clear-unreadable").await;
    rusqlite::Connection::open(&db)
        .expect("open")
        .execute(
            "INSERT INTO history_records (id, chain_id, created_at, payload)
             VALUES ('old', 'bitcoin', 0, '{\"id\":\"old\",\"kind\":\"send\",\"status\":\"pending\"}')",
            [],
        )
        .expect("insert");
    assert!(
        service.transactions().await.is_err(),
        "fixture must not decode"
    );

    crate::wallet_db::history_clear(&crate::wallet_db::WalletDatabase::new(&db)).expect("clear");
    assert!(service.transactions().await.expect("read").is_empty());
    let _ = std::fs::remove_file(&db);
}

/// Without a bound store the error says what to do, rather than reading
/// somewhere nobody will look.
#[tokio::test]
async fn reads_require_an_opened_store() {
    let service = WalletService::new(Vec::new()).expect("service");
    let error = service.transactions().await.expect_err("must refuse");
    assert!(
        error.to_string().contains("open_state"),
        "unhelpful error: {error}"
    );
}

/// A record is indexed under its own Unix time, which it keeps.
#[tokio::test]
async fn created_at_uses_unix_time_in_payload_and_index() {
    let (service, db) = opened("epoch").await;
    let mut dated = record("tx1", "w1", "confirmed");
    dated.created_at_unix = 1_700_000_000.0;
    service
        .upsert_history_records(vec![dated])
        .await
        .expect("upsert");

    let index: f64 = rusqlite::Connection::open(&db)
        .expect("open")
        .query_row("SELECT created_at FROM history_records", [], |row| {
            row.get(0)
        })
        .expect("index");
    assert_eq!(index, 1_700_000_000.0);
    assert_eq!(
        service.transactions().await.expect("read")[0].created_at_unix,
        1_700_000_000.0
    );

    let _ = std::fs::remove_file(&db);
}
