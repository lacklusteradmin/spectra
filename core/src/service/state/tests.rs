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
    assert!(s.keypool.read().await.owned_everywhere().next().is_none());
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
    assert!(s
        .apply_state_command(StateCommand::SelectNetworkChain {
            chain_id: "bitcoin-testnet-4".into()
        })
        .await
        .is_err());
    assert_eq!(
        s.keypool_state("w".into(), "Bitcoin".into())
            .await
            .unwrap()
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
    sql(&db, "CREATE TRIGGER reject_log BEFORE INSERT ON app_state_meta BEGIN SELECT RAISE(FAIL, 'injected'); END;");
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
            .unwrap()
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
            .unwrap()
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
            .unwrap()
            .reserved_receive_index,
        Some(11)
    );
}

#[tokio::test]
async fn a_setting_update_only_writes_its_metadata_and_noop_writes_nothing() {
    use crate::store::state::WalletSummary;
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    s.apply_state_command(StateCommand::UpsertWallet {
        wallet: WalletSummary::single_address(
            "w",
            "Wallet",
            "Bitcoin",
            "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
            None,
            true,
        ),
    })
    .await
    .unwrap();
    sql(&db, "CREATE TABLE write_audit (name TEXT);\
        CREATE TRIGGER wallet_insert AFTER INSERT ON wallets BEGIN INSERT INTO write_audit VALUES ('wallet'); END;\
        CREATE TRIGGER wallet_update AFTER UPDATE ON wallets BEGIN INSERT INTO write_audit VALUES ('wallet'); END;\
        CREATE TRIGGER wallet_delete AFTER DELETE ON wallets BEGIN INSERT INTO write_audit VALUES ('wallet'); END;\
        CREATE TRIGGER book_insert AFTER INSERT ON address_book BEGIN INSERT INTO write_audit VALUES ('book'); END;\
        CREATE TRIGGER book_update AFTER UPDATE ON address_book BEGIN INSERT INTO write_audit VALUES ('book'); END;\
        CREATE TRIGGER book_delete AFTER DELETE ON address_book BEGIN INSERT INTO write_audit VALUES ('book'); END;\
        CREATE TRIGGER meta_insert AFTER INSERT ON app_state_meta BEGIN INSERT INTO write_audit VALUES (NEW.key); END;\
        CREATE TRIGGER meta_update AFTER UPDATE ON app_state_meta BEGIN INSERT INTO write_audit VALUES (NEW.key); END;\
        CREATE TRIGGER meta_delete AFTER DELETE ON app_state_meta BEGIN INSERT INTO write_audit VALUES (OLD.key); END;");
    s.apply_state_command(currency("EUR")).await.unwrap();
    s.apply_state_command(currency("EUR")).await.unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM write_audit")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert_eq!(service().open_state(db).await.unwrap(), s.app_state().await);
}

#[tokio::test]
async fn unchanged_receive_reservation_skips_sql_but_merges_newly_owned_indices() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    let reserved = s
        .reserve_receive_index("w".into(), "Bitcoin".into(), 1)
        .await
        .unwrap();
    sql(&db, "CREATE TABLE pool_writes (n INTEGER); CREATE TRIGGER audit_pool AFTER UPDATE ON wallet_keypool BEGIN INSERT INTO pool_writes VALUES (1); END;");
    for _ in 0..3 {
        assert_eq!(
            s.reserve_receive_index("w".into(), "Bitcoin".into(), 1)
                .await
                .unwrap(),
            reserved
        );
    }
    let count = || {
        rusqlite::Connection::open(&db)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM pool_writes", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
    };
    assert_eq!(count(), 0);
    s.register_owned_address(
        "w".into(),
        "Bitcoin".into(),
        "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu".into(),
        Some("m/84'/0'/0'/0/10".into()),
        Some("external".into()),
        Some(10),
    )
    .await
    .unwrap();
    assert_eq!(
        s.reserve_receive_index("w".into(), "Bitcoin".into(), 1)
            .await
            .unwrap(),
        reserved
    );
    assert_eq!(count(), 1);
    let state = service().open_state(db.clone()).await.unwrap();
    assert_eq!(state.wallets.len(), 0);
    let pool = crate::wallet_db::keypool_load(&db, "w", "Bitcoin")
        .unwrap()
        .unwrap();
    assert_eq!(pool.next_external_index, 11);
    assert_eq!(pool.reserved_receive_index, Some(reserved));
}

#[tokio::test]
async fn unreadable_history_refuses_keypool_reads_and_mutations() {
    let s = service();
    let db = database();
    s.open_state(db.clone()).await.unwrap();
    let held = s
        .reserve_receive_index("w".into(), "Bitcoin".into(), 1)
        .await
        .unwrap();
    let before = s.keypool.read().await.indices().clone();
    sql(&db, "DROP TABLE history_records;");
    assert!(s.keypool_state("w".into(), "Bitcoin".into()).await.is_err());
    assert!(s
        .reserve_receive_index("w".into(), "Bitcoin".into(), 1)
        .await
        .is_err());
    assert!(s
        .reserve_change_index("w".into(), "Bitcoin".into())
        .await
        .is_err());
    assert!(s
        .advance_receive_index_if_current("w".into(), "Bitcoin".into(), held)
        .await
        .is_err());
    assert_eq!(*s.keypool.read().await.indices(), before);
    assert_eq!(
        crate::wallet_db::keypool_load(&db, "w", "Bitcoin")
            .unwrap()
            .unwrap(),
        before[&keypool_key("w", "Bitcoin")]
    );
}

/// Tor policy, the display-currency catalog and the fiat rates are core state
/// now: a front end kept all three, two of them in `UserDefaults`.
mod tor_and_rates {
    use super::*;
    use crate::store::state::AppSettingUpdate;

    fn setting(update: AppSettingUpdate) -> StateCommand {
        StateCommand::SetAppSetting { update }
    }

    /// The four Tor fields survive reopening the database, which is the whole
    /// point of moving them: a second front end and the CLI read what the app
    /// set.
    ///
    /// `tor_enabled` and `tor_kill_switch` are never true together here. The
    /// policy is process-wide — the HTTP layer reads it per request — so a test
    /// that engaged the switch would refuse every other test's HTTP call for as
    /// long as it held it. What the switch itself does is
    /// `the_kill_switch_engages_only_while_tor_is_wanted_and_not_ready`, over
    /// values rather than globals.
    #[tokio::test]
    async fn tor_settings_persist_and_refuse_an_address_that_is_not_socks5() {
        let s = service();
        let db = database();
        s.open_state(db.clone()).await.unwrap();
        for update in [
            AppSettingUpdate::TorEnabled { value: true },
            AppSettingUpdate::TorUseCustomProxy { value: true },
            AppSettingUpdate::TorCustomProxyAddress {
                value: "socks5h://10.0.0.2:9050".into(),
            },
        ] {
            s.apply_state_command(setting(update)).await.unwrap();
        }

        for bad in [
            "127.0.0.1:9150",
            "http://127.0.0.1:9150",
            "socks5://127.0.0.1",
            "socks5://:9150",
            "socks5://127.0.0.1:0",
            "socks5://127.0.0.1:notaport",
        ] {
            let transition = s
                .apply_state_command(setting(AppSettingUpdate::TorCustomProxyAddress {
                    value: bad.into(),
                }))
                .await
                .unwrap();
            assert!(
                transition
                    .events
                    .iter()
                    .any(|event| event.kind == "appSettingRejected"),
                "{bad} was not refused"
            );
            assert_eq!(
                transition.state.settings.tor_custom_proxy_address,
                "socks5h://10.0.0.2:9050"
            );
        }

        // Empty restores the default rather than storing nothing.
        let transition = s
            .apply_state_command(setting(AppSettingUpdate::TorCustomProxyAddress {
                value: "   ".into(),
            }))
            .await
            .unwrap();
        assert_eq!(
            transition.state.settings.tor_custom_proxy_address,
            "socks5://127.0.0.1:9150"
        );

        // Turn Tor off before turning the switch on, so the two are never both
        // set while this test holds the process-wide policy.
        s.apply_state_command(setting(AppSettingUpdate::TorEnabled { value: false }))
            .await
            .unwrap();
        s.apply_state_command(setting(AppSettingUpdate::TorKillSwitch { value: true }))
            .await
            .unwrap();
        assert!(!crate::tor::kill_switch_engaged());

        let reopened = service();
        let state = reopened.open_state(db).await.unwrap();
        assert!(!state.settings.tor_enabled);
        assert!(state.settings.tor_use_custom_proxy);
        assert!(state.settings.tor_kill_switch);
        assert_eq!(
            state.settings.tor_custom_proxy_address,
            "socks5://127.0.0.1:9150"
        );
        assert!(!crate::tor::kill_switch_engaged());
    }

    /// The switch blocks exactly when the user asked for Tor, asked for the
    /// switch, and Tor is not carrying traffic.
    #[test]
    fn the_kill_switch_engages_only_while_tor_is_wanted_and_not_ready() {
        use crate::tor::kill_switch_verdict;
        use crate::tor::TorStatus;
        for (wanted, switch, ready, expected) in [
            (true, true, false, true),
            (true, true, true, false),
            (true, false, false, false),
            (false, true, false, false),
            (false, false, false, false),
        ] {
            let status = if ready {
                TorStatus::Ready
            } else {
                TorStatus::Stopped
            };
            assert_eq!(
                kill_switch_verdict(switch, wanted, &status),
                expected,
                "wanted={wanted} switch={switch} ready={ready}"
            );
        }
        // Bootstrapping is not ready: the window this switch exists for.
        assert!(kill_switch_verdict(
            true,
            true,
            &TorStatus::Bootstrapping { percent: 90 }
        ));
    }

    /// A display currency the app cannot quote in is refused. It used to be
    /// stored, and every amount then rendered unconverted beside that code.
    #[tokio::test]
    async fn only_a_currency_the_app_quotes_in_can_be_selected() {
        let s = service();
        s.open_state(database()).await.unwrap();
        for code in ["EUR", "jpy", " aed "] {
            let transition = s
                .apply_state_command(StateCommand::SetFiatCurrency {
                    fiat_currency_code: code.into(),
                })
                .await
                .unwrap();
            assert_eq!(
                transition.state.settings.fiat_currency_code,
                code.trim().to_uppercase()
            );
        }
        for code in ["ZZZ", "", "US", "BITCOIN"] {
            let transition = s
                .apply_state_command(StateCommand::SetFiatCurrency {
                    fiat_currency_code: code.into(),
                })
                .await
                .unwrap();
            assert!(
                transition
                    .events
                    .iter()
                    .any(|event| event.kind == "fiatCurrencyRejected"),
                "{code:?} was not refused"
            );
            assert_eq!(transition.state.settings.fiat_currency_code, "AED");
        }
    }

    /// Rates are stored where the state is, so a reopened service quotes the
    /// same amounts without a network call.
    #[tokio::test]
    async fn stored_fiat_rates_survive_reopening() {
        let s = service();
        let db = database();
        s.open_state(db.clone()).await.unwrap();
        assert!(s.app_state().await.fiat_rates_from_usd.is_empty());

        let rates =
            std::collections::HashMap::from([("USD".to_string(), 1.0), ("EUR".to_string(), 0.9)]);
        s.store_fiat_rates(rates.clone()).await.unwrap();
        // Storing what is already stored writes nothing.
        s.store_fiat_rates(rates.clone()).await.unwrap();

        let reopened = service();
        assert_eq!(
            reopened.open_state(db).await.unwrap().fiat_rates_from_usd,
            rates
        );
    }
}
