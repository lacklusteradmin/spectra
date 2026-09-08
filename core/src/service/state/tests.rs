use super::*;

fn database() -> String {
    std::env::temp_dir()
        .join(format!(
            "spectra-write-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned()
}

fn service() -> Arc<WalletService> {
    WalletService::new_typed(vec![]).unwrap()
}
fn currency(code: &str) -> StateCommand {
    StateCommand::SetFiatCurrency {
        fiat_currency_code: code.into(),
    }
}
fn sql(path: &str, statement: &str) {
    rusqlite::Connection::open(path)
        .unwrap()
        .execute_batch(statement)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_commands_and_events_match_reopened_database() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    let mut jobs = tokio::task::JoinSet::new();
    for i in 0..40 {
        let s = s.clone();
        jobs.spawn(async move {
            s.apply_state_command(currency(if i % 2 == 0 { "EUR" } else { "JPY" }))
                .await
                .unwrap();
            s.append_chain_operational_event(
                "Bitcoin".into(),
                crate::store::ChainOperationalEventLevel::Info,
                i.to_string(),
                None,
            )
            .await
            .unwrap();
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap();
    }
    let reopened = service();
    assert_eq!(reopened.open_state(db).await.unwrap(), s.app_state().await);
    let events = s.operational_events("Bitcoin".into()).await;
    assert_eq!(events.len(), 40);
    assert_eq!(
        serde_json::to_value(reopened.operational_events("Bitcoin".into()).await).unwrap(),
        serde_json::to_value(events).unwrap()
    );
}

#[tokio::test]
async fn failed_state_commit_does_not_publish_and_retry_persists() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    let before = s.app_state().await;
    sql(&db, "CREATE TRIGGER reject_meta BEFORE INSERT ON app_state_meta BEGIN SELECT RAISE(FAIL, 'injected'); END;");
    assert!(s.apply_state_command(currency("EUR")).await.is_err());
    assert_eq!(s.app_state().await, before);
    assert_eq!(crate::wallet_db::app_state_load(&db).unwrap(), before);
    sql(&db, "DROP TRIGGER reject_meta;");
    s.apply_state_command(currency("EUR")).await.unwrap();
    assert_eq!(service().open_state(db).await.unwrap(), s.app_state().await);
}

#[tokio::test]
async fn failed_keypool_and_address_writes_leave_memory_unchanged() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    sql(&db, "CREATE TRIGGER reject_pool BEFORE INSERT ON wallet_keypool BEGIN SELECT RAISE(FAIL, 'injected'); END;
        CREATE TRIGGER reject_address BEFORE INSERT ON wallet_owned_addresses BEGIN SELECT RAISE(FAIL, 'injected'); END;");
    assert!(s
        .reserve_receive_index("w".into(), "Bitcoin".into(), 1)
        .await
        .is_err());
    assert!(s.keypool.read().await.is_empty());
    assert!(s
        .register_owned_address(
            "w".into(),
            "Bitcoin".into(),
            "address".into(),
            None,
            None,
            None
        )
        .await
        .is_err());
    assert!(s.owned_addresses.read().await.is_empty());
    sql(
        &db,
        "DROP TRIGGER reject_pool; DROP TRIGGER reject_address;",
    );
    assert_eq!(
        s.reserve_receive_index("w".into(), "Bitcoin".into(), 1)
            .await
            .unwrap(),
        1
    );
    sql(&db, "CREATE TRIGGER reject_delete BEFORE DELETE ON wallet_keypool BEGIN SELECT RAISE(FAIL, 'injected'); END;");
    assert!(s.delete_keypool_for_wallet("w".into()).await.is_err());
    assert_eq!(
        s.keypool_state("w".into(), "Bitcoin".into())
            .await
            .reserved_receive_index,
        Some(1)
    );
}

#[tokio::test]
async fn failed_log_commit_does_not_publish() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    s.append_chain_operational_event(
        "Bitcoin".into(),
        crate::store::ChainOperationalEventLevel::Info,
        "original".into(),
        None,
    )
    .await
    .unwrap();
    sql(&db, "CREATE TRIGGER reject_log BEFORE INSERT ON state BEGIN SELECT RAISE(FAIL, 'injected'); END;");
    assert!(s.clear_operational_events(None).await.is_err());
    assert_eq!(s.operational_events("Bitcoin".into()).await.len(), 1);
    let reopened = service();
    reopened.open_state(db).await.unwrap();
    assert_eq!(reopened.operational_events("Bitcoin".into()).await.len(), 1);
}

#[tokio::test]
async fn cancelling_caller_does_not_interrupt_an_admitted_commit() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    // Block candidate creation after the worker has acquired the writer.
    let state_guard = s.wallet_state.write().await;
    let caller = {
        let s = s.clone();
        tokio::spawn(async move { s.apply_state_command(currency("EUR")).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if s.state_writer.try_lock().is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    let _ = caller.await;
    drop(state_guard);
    // Queue behind the admitted write, proving it completed despite cancellation.
    let _writer = s.state_writer.lock().await;
    assert_eq!(s.app_state().await.settings.fiat_currency_code, "EUR");
    assert_eq!(
        crate::wallet_db::app_state_load(&db).unwrap(),
        s.app_state().await
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_probes_advance_only_the_reservation_they_checked() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    let used = s
        .reserve_receive_index("w".into(), "Bitcoin".into(), 1)
        .await
        .unwrap();
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..20 {
        let s = s.clone();
        jobs.spawn(async move {
            s.advance_receive_index_if_current("w".into(), "Bitcoin".into(), used)
                .await
                .unwrap()
        });
    }
    let mut advanced = vec![];
    while let Some(result) = jobs.join_next().await {
        if let Some(index) = result.unwrap() {
            advanced.push(index);
        }
    }
    assert_eq!(advanced, vec![2]);
    let reopened = service();
    reopened.open_state(db).await.unwrap();
    assert_eq!(
        reopened
            .keypool_state("w".into(), "Bitcoin".into())
            .await
            .reserved_receive_index,
        Some(2)
    );
    // A probe returning after release/re-reserve must not touch the new index.
    s.clear_reserved_receive_index("w".into(), "Bitcoin".into())
        .await
        .unwrap();
    assert_eq!(
        s.reserve_receive_index("w".into(), "Bitcoin".into(), 1)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        s.advance_receive_index_if_current("w".into(), "Bitcoin".into(), 2)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        s.keypool_state("w".into(), "Bitcoin".into())
            .await
            .reserved_receive_index,
        Some(3)
    );
}

#[tokio::test]
async fn advancement_respects_addresses_discovered_while_probe_was_in_flight() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    let used = s
        .reserve_receive_index("w".into(), "Bitcoin".into(), 1)
        .await
        .unwrap();
    s.register_owned_address(
        "w".into(),
        "Bitcoin".into(),
        "bc1qknown".into(),
        None,
        Some("external".into()),
        Some(10),
    )
    .await
    .unwrap();
    assert_eq!(
        s.advance_receive_index_if_current("w".into(), "Bitcoin".into(), used)
            .await
            .unwrap(),
        Some(11)
    );
    let reopened = service();
    reopened.open_state(db).await.unwrap();
    assert_eq!(
        reopened
            .keypool_state("w".into(), "Bitcoin".into())
            .await
            .reserved_receive_index,
        Some(11)
    );
}
