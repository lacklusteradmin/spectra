use crate::service::WalletService;
use crate::store::persistence_models::TransactionRecord;
use crate::store::wallet_domain::{TransactionKind, TransactionStatus};

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
    let service = WalletService::new(Vec::new()).expect("service");
    let db = tmp_db(tag);
    service.open_state(db.clone()).await.expect("open");
    // A fetched page is merged only for a wallet stored on its network.
    service
        .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
            wallet: crate::store::state::WalletState::single_address(
                "w1",
                "W",
                crate::registry::Chain::Bitcoin,
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
                None,
                true,
            ),
        })
        .await
        .expect("wallet");
    (service, db)
}

fn record(id: &str, hash: &str, confirmations: Option<i64>) -> TransactionRecord {
    TransactionRecord {
        actions: Default::default(),
        deployment_id: None,
        id: id.to_string(),
        wallet_id: Some("w1".to_string()),
        kind: TransactionKind::Receive,
        status: TransactionStatus::Confirmed,
        wallet_name: "W".to_string(),
        asset_display_name: "Bitcoin".to_string(),
        symbol: "BTC".to_string(),
        chain_id: crate::registry::Chain::Bitcoin,
        amount: "1".into(),
        address: "bc1qexample".to_string(),
        transaction_hash: Some(hash.to_string()),
        nonce: None,
        receipt_block_number: None,
        receipt_gas_used: None,
        receipt_effective_gas_price_gwei: None,
        receipt_network_fee: None,
        fee_rate_description: None,
        confirmation_count: confirmations,
        confirmed_network_fee: None,
        used_change_output: None,
        source_derivation_path: None,
        change_derivation_path: None,
        source_address: None,
        change_address: None,
        failure_reason: None,
        transaction_history_source: None,
        created_at_unix: 1_700_000_000.0,
    }
}

#[tokio::test]
async fn a_first_merge_adds_everything() {
    let (service, db) = opened("first").await;
    let change = service
        .merge_fetched_history(vec![record("tx1", "hash1", Some(1))])
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
        .merge_fetched_history(vec![record("tx1", "hash1", Some(1))])
        .await
        .expect("merge");

    let again = service
        .merge_fetched_history(vec![record("tx1", "hash1", Some(1))])
        .await
        .expect("merge");
    assert!(
        again.added.is_empty() && again.updated.is_empty(),
        "unchanged records must not be rewritten: {again:?}"
    );
    let _ = std::fs::remove_file(&db);
}

/// A record whose confirmations moved is an update, and only it is written.
#[tokio::test]
async fn only_genuinely_changed_records_are_written() {
    let (service, db) = opened("changed").await;
    service
        .merge_fetched_history(vec![
            record("tx1", "hash1", Some(1)),
            record("tx2", "hash2", Some(1)),
        ])
        .await
        .expect("merge");

    let change = service
        .merge_fetched_history(vec![
            record("tx1", "hash1", Some(1)), // unchanged
            record("tx2", "hash2", Some(6)), // confirmations advanced
            record("tx3", "hash3", Some(1)), // new
        ])
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
        .upsert_history_records(vec![record("tx1", "hash1", Some(1))])
        .await
        .expect("upsert");

    let change = service
        .merge_fetched_history(vec![record("tx1", "hash1", Some(9))])
        .await
        .expect("merge");
    assert_eq!(change.updated, vec!["tx1"], "must not be treated as new");
    assert_eq!(service.transactions().await.expect("read").len(), 1);
    let _ = std::fs::remove_file(&db);
}
