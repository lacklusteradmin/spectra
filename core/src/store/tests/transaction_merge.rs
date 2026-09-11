use crate::fetch::transactions::CoreTransactionRecord;
use crate::service::{TransactionCommand, WalletService};

fn tmp_db(tag: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-tx-merge-{tag}-{}-{:?}.sqlite",
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

fn wire(id: &str, hash: &str, confirmations: Option<i64>) -> CoreTransactionRecord {
    CoreTransactionRecord {
        id: id.to_string(),
        wallet_id: Some("w1".to_string()),
        kind: "receive".to_string(),
        status: "confirmed".to_string(),
        wallet_name: "W".to_string(),
        asset_name: "Bitcoin".to_string(),
        symbol: "BTC".to_string(),
        chain_name: "Bitcoin".to_string(),
        amount: 1.0,
        address: "bc1qexample".to_string(),
        transaction_hash: Some(hash.to_string()),
        ethereum_nonce: None,
        receipt_block_number: None,
        receipt_gas_used: None,
        receipt_effective_gas_price_gwei: None,
        receipt_network_fee_eth: None,
        fee_priority_raw: None,
        fee_rate_description: None,
        confirmation_count: confirmations,
        dogecoin_confirmed_network_fee_doge: None,
        dogecoin_estimated_fee_rate_doge_per_kb: None,
        used_change_output: None,
        source_derivation_path: None,
        change_derivation_path: None,
        source_address: None,
        change_address: None,
        signed_transaction_payload: None,
        signed_transaction_payload_format: None,
        failure_reason: None,
        transaction_history_source: None,
        created_at_unix: 1_700_000_000.0,
    }
}

fn merge(incoming: Vec<CoreTransactionRecord>) -> TransactionCommand {
    // Strategy comes from the registry now — "Bitcoin" implies StandardUtxo.
    TransactionCommand::Merge {
        incoming,
        chain_name: "Bitcoin".to_string(),
        preserve_created_at_sentinel_unix: None,
    }
}

#[tokio::test]
async fn a_first_merge_adds_everything() {
    let (service, db) = opened("first").await;
    let change = service
        .apply_transaction_command(merge(vec![wire("tx1", "hash1", Some(1))]))
        .await
        .expect("merge");
    assert_eq!(change.added, vec!["tx1"]);
    assert_eq!(service.transactions().await.expect("read").len(), 1);
    let _ = std::fs::remove_file(&db);
}

/// The point of merging in core: a refresh that returns what is already
/// stored writes nothing at all.
#[tokio::test]
async fn re_merging_identical_records_is_a_no_op() {
    let (service, db) = opened("noop").await;
    service
        .apply_transaction_command(merge(vec![wire("tx1", "hash1", Some(1))]))
        .await
        .expect("merge");

    let again = service
        .apply_transaction_command(merge(vec![wire("tx1", "hash1", Some(1))]))
        .await
        .expect("merge");
    assert!(
        again.is_empty(),
        "unchanged records must not be rewritten: {again:?}"
    );
    let _ = std::fs::remove_file(&db);
}

/// A record whose confirmations moved is an update, and only it is written.
#[tokio::test]
async fn only_genuinely_changed_records_are_written() {
    let (service, db) = opened("changed").await;
    service
        .apply_transaction_command(merge(vec![
            wire("tx1", "hash1", Some(1)),
            wire("tx2", "hash2", Some(1)),
        ]))
        .await
        .expect("merge");

    let change = service
        .apply_transaction_command(merge(vec![
            wire("tx1", "hash1", Some(1)), // unchanged
            wire("tx2", "hash2", Some(6)), // confirmations advanced
            wire("tx3", "hash3", Some(1)), // new
        ]))
        .await
        .expect("merge");
    assert_eq!(change.updated, vec!["tx2"]);
    assert_eq!(change.added, vec!["tx3"]);
    assert_eq!(service.transactions().await.expect("read").len(), 3);
    let _ = std::fs::remove_file(&db);
}

/// Merging reads the store, so a record written by an unrelated command is
/// merged against rather than duplicated.
#[tokio::test]
async fn merge_sees_records_written_by_other_commands() {
    let (service, db) = opened("crosstalk").await;
    service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![wire("tx1", "hash1", Some(1)).into()],
        })
        .await
        .expect("upsert");

    let change = service
        .apply_transaction_command(merge(vec![wire("tx1", "hash1", Some(9))]))
        .await
        .expect("merge");
    assert_eq!(change.updated, vec!["tx1"], "must not be treated as new");
    assert_eq!(service.transactions().await.expect("read").len(), 1);
    let _ = std::fs::remove_file(&db);
}
