use super::*;
use crate::api::endpoint::{EndpointApi, endpoint_capability_options};
use crate::secrets::backends::InMemorySecretStore;
use crate::secrets::store::{SecretClass, SecretStoreError};
use crate::store::state::WalletState;
use std::sync::atomic::{AtomicBool, Ordering};

fn database() -> String {
    std::env::temp_dir()
        .join(format!("spectra-write-{}.sqlite", crate::store::new_id()))
        .to_string_lossy()
        .into_owned()
}

fn service() -> Arc<WalletService> {
    WalletService::new(vec![]).unwrap()
}
fn currency(code: &str) -> StateCommand {
    StateCommand::SetAppSetting {
        update: crate::store::state::AppSettingUpdate::FiatCurrency {
            value: crate::store::state::FiatCurrency::from_code(code).unwrap(),
        },
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
    s.opened(db.clone()).await.unwrap();
    let mut jobs = tokio::task::JoinSet::new();
    for i in 0..40 {
        let s = s.clone();
        jobs.spawn(async move {
            s.apply_state_command(currency(if i % 2 == 0 { "EUR" } else { "JPY" }))
                .await
                .unwrap();
            s.append_chain_operational_event(
                crate::registry::Chain::Bitcoin,
                crate::service::DiagnosticLogLevel::Info,
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
    assert_eq!(
        serde_json::to_value(reopened.opened(db).await.unwrap()).unwrap(),
        serde_json::to_value(s.app_state().await).unwrap()
    );
    let events = s.operational_events(crate::registry::Chain::Bitcoin).await;
    assert_eq!(events.len(), 40);
    assert_eq!(
        serde_json::to_value(
            reopened
                .operational_events(crate::registry::Chain::Bitcoin)
                .await
        )
        .unwrap(),
        serde_json::to_value(events).unwrap()
    );
}

#[tokio::test]
async fn failed_state_commit_does_not_publish_and_retry_persists() {
    let s = service();
    let db = database();
    s.opened(db.clone()).await.unwrap();
    let before = s.app_state().await;
    sql(
        &db,
        "CREATE TRIGGER reject_meta BEFORE INSERT ON app_state_meta BEGIN SELECT RAISE(FAIL, 'injected'); END;",
    );
    assert!(s.apply_state_command(currency("EUR")).await.is_err());
    assert_eq!(s.app_state().await, before);
    assert_eq!(
        serde_json::to_value(
            crate::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db)).unwrap()
        )
        .unwrap(),
        serde_json::to_value(before).unwrap()
    );
    sql(&db, "DROP TRIGGER reject_meta;");
    s.apply_state_command(currency("EUR")).await.unwrap();
    assert_eq!(
        serde_json::to_value(service().opened(db).await.unwrap()).unwrap(),
        serde_json::to_value(s.app_state().await).unwrap()
    );
}

#[tokio::test]
async fn failed_keypool_and_address_writes_leave_memory_unchanged() {
    let s = service();
    let db = database();
    s.opened(db.clone()).await.unwrap();
    sql(&db, "CREATE TRIGGER reject_pool BEFORE INSERT ON wallet_keypool BEGIN SELECT RAISE(FAIL, 'injected'); END;
        CREATE TRIGGER reject_address BEFORE INSERT ON wallet_owned_addresses BEGIN SELECT RAISE(FAIL, 'injected'); END;");
    assert!(
        s.reserve_receive_index("w".into(), crate::registry::Chain::Bitcoin, 1)
            .await
            .is_err()
    );
    assert!(s.keypool.read().await.is_empty());
    assert!(
        s.register_owned_address(
            "w".into(),
            crate::registry::Chain::Bitcoin,
            "address".into(),
            None,
            None,
            None
        )
        .await
        .is_err()
    );
    assert!(s.keypool.read().await.owned_everywhere().next().is_none());
    sql(
        &db,
        "DROP TRIGGER reject_pool; DROP TRIGGER reject_address;",
    );
    assert_eq!(
        s.reserve_receive_index("w".into(), crate::registry::Chain::Bitcoin, 1)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn failed_log_commit_does_not_publish() {
    let s = service();
    let db = database();
    s.opened(db.clone()).await.unwrap();
    s.append_chain_operational_event(
        crate::registry::Chain::Bitcoin,
        crate::service::DiagnosticLogLevel::Info,
        "original".into(),
        None,
    )
    .await
    .unwrap();
    sql(
        &db,
        "CREATE TRIGGER reject_log BEFORE INSERT ON app_state_meta BEGIN SELECT RAISE(FAIL, 'injected'); END;",
    );
    assert!(s.clear_operational_events(None).await.is_err());
    assert_eq!(
        s.operational_events(crate::registry::Chain::Bitcoin)
            .await
            .len(),
        1
    );
    let reopened = service();
    reopened.opened(db).await.unwrap();
    assert_eq!(
        reopened
            .operational_events(crate::registry::Chain::Bitcoin)
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn cancelling_caller_does_not_interrupt_an_admitted_commit() {
    let s = service();
    let db = database();
    s.opened(db.clone()).await.unwrap();
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
    assert_eq!(
        s.app_state().await.settings.fiat_currency,
        crate::store::state::FiatCurrency::Eur
    );
    assert_eq!(
        serde_json::to_value(
            crate::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db)).unwrap()
        )
        .unwrap(),
        serde_json::to_value(s.app_state().await).unwrap()
    );
}

#[tokio::test]
async fn advancement_respects_addresses_discovered_while_probe_was_in_flight() {
    let s = service();
    let db = database();
    s.opened(db.clone()).await.unwrap();
    let used = s
        .reserve_receive_index("w".into(), crate::registry::Chain::Bitcoin, 1)
        .await
        .unwrap();
    s.register_owned_address(
        "w".into(),
        crate::registry::Chain::Bitcoin,
        "bc1qknown".into(),
        None,
        Some(crate::wallet_db::AddressBranch::Receive),
        Some(10),
    )
    .await
    .unwrap();
    assert_eq!(
        s.advance_receive_index_if_current("w".into(), crate::registry::Chain::Bitcoin, used)
            .await
            .unwrap(),
        Some(11)
    );
    let reopened = service();
    reopened.opened(db).await.unwrap();
    assert_eq!(
        reopened
            .keypool_state("w".into(), crate::registry::Chain::Bitcoin)
            .await
            .unwrap()
            .reserved_receive_index,
        Some(11)
    );
}

#[tokio::test]
async fn a_setting_update_only_writes_its_metadata_and_noop_writes_nothing() {
    use crate::store::state::WalletState;
    let s = service();
    let db = database();
    s.opened(db.clone()).await.unwrap();
    s.upsert_wallet(WalletState::single_address(
        "w",
        "Wallet",
        crate::registry::Chain::Bitcoin,
        "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
        None,
        true,
    ))
    .await
    .unwrap();
    sql(
        &db,
        "CREATE TABLE write_audit (name TEXT);\
        CREATE TRIGGER wallet_insert AFTER INSERT ON wallets BEGIN INSERT INTO write_audit VALUES ('wallet'); END;\
        CREATE TRIGGER wallet_update AFTER UPDATE ON wallets BEGIN INSERT INTO write_audit VALUES ('wallet'); END;\
        CREATE TRIGGER wallet_delete AFTER DELETE ON wallets BEGIN INSERT INTO write_audit VALUES ('wallet'); END;\
        CREATE TRIGGER book_insert AFTER INSERT ON address_book BEGIN INSERT INTO write_audit VALUES ('book'); END;\
        CREATE TRIGGER book_update AFTER UPDATE ON address_book BEGIN INSERT INTO write_audit VALUES ('book'); END;\
        CREATE TRIGGER book_delete AFTER DELETE ON address_book BEGIN INSERT INTO write_audit VALUES ('book'); END;\
        CREATE TRIGGER meta_insert AFTER INSERT ON app_state_meta BEGIN INSERT INTO write_audit VALUES (NEW.key); END;\
        CREATE TRIGGER meta_update AFTER UPDATE ON app_state_meta BEGIN INSERT INTO write_audit VALUES (NEW.key); END;\
        CREATE TRIGGER meta_delete AFTER DELETE ON app_state_meta BEGIN INSERT INTO write_audit VALUES (OLD.key); END;",
    );
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
    assert_eq!(
        serde_json::to_value(service().opened(db).await.unwrap()).unwrap(),
        serde_json::to_value(s.app_state().await).unwrap()
    );
}

#[tokio::test]
async fn unchanged_receive_reservation_skips_sql_but_merges_newly_owned_indices() {
    let s = service();
    let db = database();
    s.opened(db.clone()).await.unwrap();
    let reserved = s
        .reserve_receive_index("w".into(), crate::registry::Chain::Bitcoin, 1)
        .await
        .unwrap();
    sql(
        &db,
        "CREATE TABLE pool_writes (n INTEGER); CREATE TRIGGER audit_pool AFTER UPDATE ON wallet_keypool BEGIN INSERT INTO pool_writes VALUES (1); END;",
    );
    for _ in 0..3 {
        assert_eq!(
            s.reserve_receive_index("w".into(), crate::registry::Chain::Bitcoin, 1)
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
        crate::registry::Chain::Bitcoin,
        "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu".into(),
        Some("m/84'/0'/0'/0/10".into()),
        Some(crate::wallet_db::AddressBranch::Receive),
        Some(10),
    )
    .await
    .unwrap();
    assert_eq!(
        s.reserve_receive_index("w".into(), crate::registry::Chain::Bitcoin, 1)
            .await
            .unwrap(),
        reserved
    );
    assert_eq!(count(), 1);
    let state = service().opened(db.clone()).await.unwrap();
    assert_eq!(state.wallets.len(), 0);
    let pool = stored_keypool(&db);
    assert_eq!(pool.next_external_index, 11);
    assert_eq!(pool.reserved_receive_index, Some(reserved));
}

#[tokio::test]
async fn unreadable_history_refuses_keypool_reads_and_mutations() {
    let s = service();
    let db = database();
    s.opened(db.clone()).await.unwrap();
    let held = s
        .reserve_receive_index("w".into(), crate::registry::Chain::Bitcoin, 1)
        .await
        .unwrap();
    let before = s.keypool.read().await.indices().clone();
    sql(&db, "DROP TABLE history_records;");
    assert!(
        s.keypool_state("w".into(), crate::registry::Chain::Bitcoin)
            .await
            .is_err()
    );
    assert!(
        s.reserve_receive_index("w".into(), crate::registry::Chain::Bitcoin, 1)
            .await
            .is_err()
    );
    assert!(
        s.reserve_change_index("w".into(), crate::registry::Chain::Bitcoin)
            .await
            .is_err()
    );
    assert!(
        s.advance_receive_index_if_current("w".into(), crate::registry::Chain::Bitcoin, held)
            .await
            .is_err()
    );
    assert_eq!(*s.keypool.read().await.indices(), before);
    assert_eq!(
        stored_keypool(&db),
        before[&keypool_key("w", crate::registry::Chain::Bitcoin)]
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

    /// The Tor fields survive reopening the database, so a second front end
    /// and the CLI read what the app set.
    ///
    /// `tor_enabled` is never turned on here. The policy is process-wide — the
    /// HTTP layer reads it per request — and Tor on with no transport refuses
    /// every request, so a test that held it would refuse every other test's
    /// HTTP call. What the policy does is
    /// `the_kill_switch_engages_only_while_tor_is_wanted_and_not_ready`, over
    /// values rather than globals.
    #[tokio::test]
    async fn tor_settings_persist_and_refuse_an_address_that_is_not_socks5() {
        let s = service();
        let db = database();
        s.opened(db.clone()).await.unwrap();
        for update in [
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
                transition.events.iter().any(|event| matches!(
                    event,
                    crate::store::state::StateEvent::AppSettingRejected
                )),
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

        assert!(!crate::api::tor::kill_switch_engaged());

        let reopened = service();
        let state = reopened.opened(db).await.unwrap();
        assert!(!state.settings.tor_enabled);
        assert!(state.settings.tor_use_custom_proxy);
        assert_eq!(
            state.settings.tor_custom_proxy_address,
            "socks5://127.0.0.1:9150"
        );
        assert!(!crate::api::tor::kill_switch_engaged());
    }

    /// Requests are held back exactly when the user asked for Tor and Tor is
    /// not carrying traffic: asking for Tor is asking never to go out in the
    /// clear, with no setting to fall back to a direct connection.
    #[test]
    fn the_kill_switch_engages_only_while_tor_is_wanted_and_not_ready() {
        use crate::api::tor::TorStatus;
        use crate::api::tor::kill_switch_verdict;
        for (wanted, ready, expected) in [
            (true, false, true),
            (true, true, false),
            (false, false, false),
            (false, true, false),
        ] {
            let status = if ready {
                TorStatus::Ready
            } else {
                TorStatus::Stopped
            };
            assert_eq!(
                kill_switch_verdict(wanted, &status),
                expected,
                "wanted={wanted} ready={ready}"
            );
        }
        // Bootstrapping is not ready: the window a direct fallback would leak.
        assert!(kill_switch_verdict(
            true,
            &TorStatus::Bootstrapping { percent: 90 }
        ));
        assert!(kill_switch_verdict(
            true,
            &TorStatus::Error {
                message: "down".into()
            }
        ));
    }

    /// Rates are stored where the state is, so a reopened service quotes the
    /// same amounts without a network call.
    #[tokio::test]
    async fn stored_fiat_rates_survive_reopening() {
        let s = service();
        let db = database();
        s.opened(db.clone()).await.unwrap();
        assert!(s.app_state().await.fiat_rates_from_usd.is_empty());

        let rates =
            std::collections::HashMap::from([("USD".to_string(), 1.0), ("EUR".to_string(), 0.9)]);
        s.store_fiat_rates(rates.clone()).await.unwrap();
        // Storing what is already stored writes nothing.
        s.store_fiat_rates(rates.clone()).await.unwrap();

        let reopened = service();
        assert_eq!(
            reopened.opened(db).await.unwrap().fiat_rates_from_usd,
            rates
        );
    }
}

#[tokio::test]
async fn owned_alert_evaluation_uses_quotes_and_fires_once_across_reopen() {
    let service = service();
    let path = database();
    service.opened(path.clone()).await.unwrap();
    service
        .mutate_persisted_state(|state| {
            state.settings.use_price_alerts = true;
            state.price_alerts = vec![crate::store::PriceAlertRule {
                id: "a".into(),
                holding_key: "ethereum:native".into(),
                asset_display_name: "Ethereum".into(),
                symbol: "ETH".into(),
                chain_id: crate::registry::Chain::Ethereum,
                target_price: 2.0,
                condition: crate::store::wallet_domain::PriceAlertCondition::Above,
                is_enabled: true,
                has_triggered: false,
            }];
            state.quotes.prices.insert("ethereum:native".into(), 3.0);
            vec![crate::store::state::StateEvent::StateReplaced]
        })
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        service.evaluate_price_alerts(),
        service.evaluate_price_alerts()
    );
    assert_eq!(a.unwrap().len() + b.unwrap().len(), 1);
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.opened(path.clone()).await.unwrap();
    assert!(reopened.app_state().await.price_alerts[0].has_triggered);
    assert!(reopened.evaluate_price_alerts().await.unwrap().is_empty());
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn owned_receive_validates_scope_and_keeps_display_reads_read_only() {
    use crate::store::state::WalletState;
    let service = service();
    let path = database();
    service.opened(path.clone()).await.unwrap();
    service
        .upsert_wallet(WalletState::single_address(
            "watch",
            "Watch",
            crate::registry::Chain::Ethereum,
            "0x1111111111111111111111111111111111111111",
            None,
            true,
        ))
        .await
        .unwrap();
    assert!(
        service
            .receive_address("missing".into(), crate::registry::Chain::Ethereum, true)
            .await
            .is_err()
    );
    assert!(
        service
            .receive_address("watch".into(), crate::registry::Chain::Bitcoin, true)
            .await
            .unwrap()
            .is_none()
    );
    let read = service
        .receive_address("watch".into(), crate::registry::Chain::Ethereum, false)
        .await
        .unwrap()
        .unwrap();
    assert!(
        service
            .owned_addresses_for_wallet("watch".into(), None)
            .await
            .is_empty()
    );
    assert_eq!(
        service
            .receive_address("watch".into(), crate::registry::Chain::Ethereum, true)
            .await
            .unwrap(),
        Some(read.clone())
    );
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.opened(path.clone()).await.unwrap();
    assert!(
        reopened
            .owned_addresses_for_wallet("watch".into(), None)
            .await
            .contains(&read)
    );
    assert!(
        reopened
            .discover_chain_addresses(crate::registry::Chain::Bitcoin)
            .await
            .unwrap()
            .is_empty()
    );
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn owned_catalog_transport_reads_saved_settings_and_preserves_explicit_overrides() {
    let service = WalletService::new_catalog().unwrap();
    let path = database();
    service.opened(path.clone()).await.unwrap();
    let original = service
        .endpoints_for(crate::registry::Chain::Ethereum, &[])
        .await;
    assert!(!original.is_empty());
    assert!(
        !service
            .api_endpoints(crate::registry::Chain::Ton, EndpointApi::ToncenterV3, &[])
            .await
            .unwrap()
            .is_empty()
    );
    service
        .apply_state_command(StateCommand::SetAppSetting {
            update: crate::store::state::AppSettingUpdate::AddCustomEndpoint {
                capabilities: endpoint_capability_options(
                    crate::registry::Chain::Ethereum,
                    EndpointApi::EvmJsonRpc,
                ),
                chain_id: crate::registry::Chain::Ethereum,
                api: crate::api::endpoint::EndpointApi::EvmJsonRpc,
                endpoint: "http://127.0.0.1:8545".into(),
            },
        })
        .await
        .unwrap();
    assert_eq!(
        service
            .endpoints_for(crate::registry::Chain::Ethereum, &[])
            .await[0],
        "http://127.0.0.1:8545"
    );
    let reopened = WalletService::new_catalog().unwrap();
    reopened.opened(path.clone()).await.unwrap();
    assert_eq!(
        reopened
            .endpoints_for(crate::registry::Chain::Ethereum, &[])
            .await[0],
        "http://127.0.0.1:8545"
    );
    reopened
        .use_only_endpoints(
            crate::registry::Chain::Ethereum,
            vec!["http://127.0.0.1:9545".into()],
        )
        .await
        .unwrap();
    assert_eq!(
        &*reopened
            .endpoints_for(crate::registry::Chain::Ethereum, &[])
            .await,
        &["http://127.0.0.1:9545"]
    );
    service
        .reset_data(vec![crate::store::state::ResetScope::SettingsAndEndpoints])
        .await
        .unwrap();
    assert_eq!(
        service
            .endpoints_for(crate::registry::Chain::Ethereum, &[])
            .await,
        original
    );
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn failed_open_does_not_publish_and_can_retry_seeding() {
    for previously_bound in [false, true] {
        let s = service();
        if previously_bound {
            s.opened(database()).await.unwrap();
            s.apply_state_command(currency("EUR")).await.unwrap();
        }
        let before = s.app_state().await;
        let old = s.state_binding.connection().await;
        let path = database();
        let db = crate::wallet_db::WalletDatabase::new(&path);
        db.with_connection(|conn| {
            conn.execute_batch("CREATE TRIGGER reject_seed BEFORE INSERT ON app_state_meta WHEN NEW.key = 'token_preferences' BEGIN SELECT RAISE(FAIL, 'seed blocked'); END;").map_err(crate::wallet_db::error::DbError::from)
        }).unwrap();
        assert!(
            s.opened(path.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("seed blocked")
        );
        assert_eq!(s.app_state().await, before);
        assert!(!s.state_binding.is_bound_to(&path).await);
        assert_eq!(
            s.state_binding
                .connection()
                .await
                .as_ref()
                .map(|d| d.path().to_string()),
            old.as_ref().map(|d| d.path().to_string())
        );
        sql(&path, "DROP TRIGGER reject_seed");
        let opened = s.opened(path.clone()).await.unwrap();
        assert!(!opened.token_preferences.is_empty());
        assert_eq!(
            serde_json::to_value(&opened).unwrap(),
            serde_json::to_value(crate::wallet_db::app_state_load(&db).unwrap()).unwrap()
        );
        assert_eq!(opened, s.opened(path).await.unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn derived_wallet_maps_share_one_snapshot_during_mutation() {
    let s = service();
    let writer = s.clone();
    let mutations = tokio::spawn(async move {
        for i in 0..100 {
            writer
                .mutate_persisted_state(move |state| {
                    state.wallets = vec![crate::store::state::WalletState::single_address(
                        format!("w{i}"),
                        "watch",
                        crate::registry::Chain::Ethereum,
                        "0x1111111111111111111111111111111111111111",
                        None,
                        true,
                    )];
                    vec![crate::store::state::StateEvent::StateReplaced]
                })
                .await
                .unwrap();
        }
    });
    for _ in 0..100 {
        let derived = s.wallet_derived_state().await.unwrap();
        let ids = |map: &HashMap<String, Vec<AssetHolding>>| {
            map.keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(
            ids(&derived.send_coins_by_wallet_id),
            ids(&derived.receive_coins_by_wallet_id)
        );
    }
    mutations.await.unwrap();
}

#[tokio::test]
async fn committed_versions_order_reads_and_failed_writes_do_not_advance_them() {
    let s = service();
    let db = database();
    let initial = s.opened(db.clone()).await.unwrap();
    let first = s.apply_state_command(currency("EUR")).await.unwrap().state;
    assert!(first.revision > initial.revision);
    let noop = s.apply_state_command(currency("EUR")).await.unwrap();
    assert!(noop.events.is_empty());
    assert_eq!(noop.state.revision, first.revision);
    sql(
        &db,
        "CREATE TRIGGER reject_revision BEFORE INSERT ON app_state_meta BEGIN SELECT RAISE(FAIL, 'injected'); END;",
    );
    assert!(s.apply_state_command(currency("JPY")).await.is_err());
    assert_eq!(s.app_state().await.revision, first.revision);
    assert_eq!(
        s.portfolio_snapshot().await.unwrap().state.revision,
        first.revision
    );
    sql(&db, "DROP TRIGGER reject_revision");
    let next = s.apply_state_command(currency("JPY")).await.unwrap().state;
    assert!(next.revision > first.revision);
    assert_eq!(s.opened(db).await.unwrap().revision, next.revision);
}

/// The keypool row core persisted for wallet `w` on Bitcoin.
fn stored_keypool(db: &str) -> crate::wallet_db::KeypoolState {
    crate::wallet_db::keypool_load_all(&crate::wallet_db::WalletDatabase::new(db)).unwrap()
        [&crate::registry::Chain::Bitcoin]["w"]
        .clone()
}

/// A view keyed on a wallet's name and addresses re-reads when those change,
/// not when a balance, or a setting that is not a wallet's, does.
#[tokio::test]
async fn wallet_identity_revision_ignores_balances_and_other_state() {
    let s = service();
    s.opened(database()).await.unwrap();
    let identity = || async {
        s.portfolio_snapshot()
            .await
            .unwrap()
            .wallet_identity_revision
    };
    let wallet = |name: &str, amount: &str| {
        let mut wallet = crate::store::state::WalletState::single_address(
            "w",
            name,
            crate::registry::Chain::Ethereum,
            "0x1111111111111111111111111111111111111111",
            None,
            true,
        );
        wallet.holdings = vec![crate::store::wallet_domain::AssetHolding {
            id: String::new(),
            name: "Ether".into(),
            symbol: "ETH".into(),
            coingecko_id: "ethereum".into(),
            chain_id: crate::registry::Chain::Ethereum,
            token_standard: None,
            contract_address: None,
            amount: amount.into(),
        }];
        wallet
    };

    let empty = identity().await;
    s.upsert_wallet(wallet("W", "1")).await.unwrap();
    let added = identity().await;
    assert!(added > empty, "a new wallet");
    s.upsert_wallet(wallet("W", "2")).await.unwrap();
    assert_eq!(identity().await, added, "a balance");
    s.apply_state_command(currency("EUR")).await.unwrap();
    assert_eq!(identity().await, added, "a setting");
    s.upsert_wallet(wallet("Renamed", "2")).await.unwrap();
    assert!(identity().await > added, "a rename");
}

const MNEMONIC: &str = "test test test test test test test test test test test junk";

#[tokio::test]
async fn deleting_stored_secrets_requires_a_durable_state_binding() {
    let service = WalletService::new(vec![]).unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    crate::secrets::wallet::store_seed_phrase(&*secrets, "w", MNEMONIC, &Default::default(), None)
        .unwrap();
    service
        .upsert_wallet(crate::store::state::WalletState::single_address(
            "w",
            "W",
            crate::registry::Chain::Ethereum,
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            None,
            false,
        ))
        .await
        .unwrap();
    let before = service.app_state().await;
    let error = service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: "w".into(),
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("database must be opened"));
    assert_eq!(service.app_state().await, before);
    assert_eq!(
        &*crate::secrets::wallet::load_seed_phrase(&*secrets, "w", None).unwrap(),
        MNEMONIC
    );
}

fn path() -> String {
    std::env::temp_dir()
        .join(format!("delete-secrets-{}.sqlite", crate::store::new_id()))
        .to_string_lossy()
        .into_owned()
}

async fn imported_wallet(service: &WalletService, password: Option<&str>) -> String {
    let commit = crate::derivation::import::WalletImportCommit {
        password: password.map(str::to_owned),
        request: crate::derivation::import::WalletImportRequest {
            wallet_name: String::new(),
            chain: crate::registry::Chain::Ethereum,
            kind: crate::derivation::import::WalletImportKind::Phrase,
        },
        derivation_path: None,
        derivation_overrides: Default::default(),
        seed_phrase: Some(MNEMONIC.into()),
        private_key: None,
        restore_height: None,
        named_account: None,
        ton_wallet_version: None,
        upgrade_wallet_id: None,
        created: false,
    };
    service.import_wallets(commit).await.unwrap().wallets[0]
        .id
        .clone()
}

#[tokio::test]
async fn failed_database_deletion_keeps_the_wallet_and_its_signing_material() {
    let service = WalletService::new(vec![]).unwrap();
    let path = path();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    service.opened(path.clone()).await.unwrap();
    let id = imported_wallet(&service, None).await;
    let before = service.app_state().await;
    sql(
        &path,
        "CREATE TRIGGER reject_delete BEFORE DELETE ON wallets BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    assert!(
        service
            .apply_state_command(StateCommand::RemoveWallet {
                wallet_id: id.clone()
            })
            .await
            .is_err()
    );
    assert_eq!(service.app_state().await, before);
    assert_eq!(
        service.reveal_seed_phrase(id.clone(), None).unwrap(),
        SeedPhraseReveal::Phrase {
            phrase: MNEMONIC.into()
        }
    );
    let database = service.bound_database().await.unwrap();
    assert!(
        crate::wallet_db::pending_secret_deletions(&database)
            .unwrap()
            .is_empty()
    );
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.set_secret_store(secrets);
    assert_eq!(
        reopened.opened(path.clone()).await.unwrap().wallets.len(),
        1
    );
    assert_eq!(
        reopened.reveal_seed_phrase(id.clone(), None).unwrap(),
        SeedPhraseReveal::Phrase {
            phrase: MNEMONIC.into()
        }
    );
    sql(&path, "DROP TRIGGER reject_delete;");
    service
        .apply_state_command(StateCommand::RemoveWallet { wallet_id: id })
        .await
        .unwrap();
}

struct FailingCleanup {
    inner: InMemorySecretStore,
    fail: AtomicBool,
}

impl SecretStore for FailingCleanup {
    fn load_secret(&self, kind: SecretClass, key: String) -> Result<String, SecretStoreError> {
        self.inner.load_secret(kind, key)
    }
    fn save_secret(
        &self,
        kind: SecretClass,
        key: String,
        value: String,
    ) -> Result<(), SecretStoreError> {
        self.inner.save_secret(kind, key, value)
    }
    fn delete_secret(&self, kind: SecretClass, key: String) -> Result<(), SecretStoreError> {
        // Salt follows the signing blobs, so this failure proves partial cleanup.
        if key.ends_with(".salt") && self.fail.load(Ordering::SeqCst) {
            return Err(SecretStoreError::Backend {
                reason: "injected cleanup failure".into(),
            });
        }
        self.inner.delete_secret(kind, key)
    }
    fn wrap_device_key(&self, key: Vec<u8>) -> Result<Vec<u8>, SecretStoreError> {
        self.inner.wrap_device_key(key)
    }
    fn unwrap_device_key(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, SecretStoreError> {
        self.inner.unwrap_device_key(wrapped)
    }
}

#[tokio::test]
async fn partial_secret_cleanup_removes_the_wallet_and_retries_after_reopening() {
    let service = WalletService::new(vec![]).unwrap();
    let path = path();
    let secrets = Arc::new(FailingCleanup {
        inner: InMemorySecretStore::new(),
        fail: AtomicBool::new(false),
    });
    service.set_secret_store(secrets.clone());
    service.opened(path.clone()).await.unwrap();
    let id = imported_wallet(&service, Some("test password")).await;
    secrets.fail.store(true, Ordering::SeqCst);
    let wallet = service.app_state().await.wallets[0].clone();
    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: id.clone(),
        })
        .await
        .unwrap();
    let state = service.app_state().await;
    assert!(state.wallets.is_empty());
    assert!(state.diagnostics.logs.iter().any(|log| {
        log.input.category == "Secret Cleanup"
            && log.input.message.contains("secret cleanup is pending")
    }));
    assert!(service.app_state().await.wallets.is_empty());
    assert!(matches!(
        secrets.load_secret(SecretClass::Seed, format!("{id}.seed")),
        Err(SecretStoreError::NotFound)
    ));
    assert!(
        secrets
            .load_secret(SecretClass::Generic, format!("{id}.salt"))
            .is_ok()
    );
    let database = service.bound_database().await.unwrap();
    assert_eq!(
        crate::wallet_db::pending_secret_deletions(&database).unwrap(),
        vec![id.clone()]
    );
    // Unrelated commands remain usable; the uncleared ID alone is refused.
    service
        .apply_state_command(StateCommand::SetAppSetting {
            update: crate::store::state::AppSettingUpdate::FiatCurrency {
                value: crate::store::state::FiatCurrency::from_code("EUR").unwrap(),
            },
        })
        .await
        .unwrap();
    assert!(service.upsert_wallet(wallet).await.is_err());
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.set_secret_store(secrets.clone());
    assert!(
        reopened
            .opened(path.clone())
            .await
            .unwrap()
            .wallets
            .is_empty()
    );
    secrets.fail.store(false, Ordering::SeqCst);
    assert!(reopened.opened(path).await.unwrap().wallets.is_empty());
    assert!(
        crate::wallet_db::pending_secret_deletions(&database)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        secrets.load_secret(SecretClass::Generic, format!("{id}.password")),
        Err(SecretStoreError::NotFound)
    ));
    assert!(matches!(
        secrets.load_secret(SecretClass::Generic, format!("{id}.salt")),
        Err(SecretStoreError::NotFound)
    ));
}

#[tokio::test]
async fn failed_cleanup_acknowledgment_is_safe_to_retry() {
    let service = WalletService::new(vec![]).unwrap();
    let path = path();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    service.opened(path.clone()).await.unwrap();
    let id = imported_wallet(&service, None).await;
    sql(
        &path,
        "CREATE TRIGGER reject_ack BEFORE DELETE ON wallet_secret_deletions BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: id.clone(),
        })
        .await
        .unwrap();
    assert!(service.app_state().await.wallets.is_empty());
    let database = service.bound_database().await.unwrap();
    assert_eq!(
        crate::wallet_db::pending_secret_deletions(&database).unwrap(),
        vec![id.clone()]
    );
    assert_eq!(
        service.reveal_seed_phrase(id, None).unwrap(),
        SeedPhraseReveal::NotStored
    );
    sql(&path, "DROP TRIGGER reject_ack;");
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.set_secret_store(secrets);
    assert!(reopened.opened(path).await.unwrap().wallets.is_empty());
    assert!(
        crate::wallet_db::pending_secret_deletions(&database)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_store_with_an_undecodable_row_opens_as_unreadable_and_discards_with_its_secrets() {
    let path = path();
    let writer = WalletService::new(vec![]).unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    writer.set_secret_store(secrets.clone());
    writer.opened(path.clone()).await.unwrap();
    let id = imported_wallet(&writer, None).await;
    drop(writer);
    // A row written in a shape this build no longer reads.
    sql(
        &path,
        "UPDATE wallets SET payload = json_remove(payload, '$.hiddenHoldings')",
    );

    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(secrets.clone());
    let error = service.opened(path.clone()).await.unwrap_err();
    assert!(
        matches!(error, SpectraBridgeError::StoreUnreadable { .. }),
        "{error:?}"
    );
    // Nothing is bound, so the next call fails the same way rather than
    // running against a half-open store.
    assert!(matches!(
        service.opened(path.clone()).await,
        Err(SpectraBridgeError::StoreUnreadable { .. })
    ));

    service.discard_state(path.clone()).await.unwrap();
    assert!(service.app_state().await.wallets.is_empty());
    assert!(crate::secrets::wallet::load_seed_phrase(&*secrets, &id, None).is_err());
    // The new store is the open one, so discarding it again is refused.
    assert!(service.discard_state(path.clone()).await.is_err());
    assert!(service.app_state().await.wallets.is_empty());
}

#[tokio::test]
async fn discarding_a_file_that_is_no_database_names_no_wallets_and_replaces_it() {
    let path = path();
    std::fs::write(&path, b"not a database").unwrap();
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(Arc::new(InMemorySecretStore::new()));
    assert!(matches!(
        service.opened(path.clone()).await,
        Err(SpectraBridgeError::StoreUnreadable { .. })
    ));
    service.discard_state(path.clone()).await.unwrap();
    assert!(service.app_state().await.wallets.is_empty());
}

#[tokio::test]
async fn wallet_delete_removes_secrets_and_relations_and_can_retry() {
    let service = WalletService::new(vec![]).unwrap();
    let path = std::env::temp_dir().join(format!("delete-owned-{}.sqlite", crate::store::new_id()));
    service.opened(path.to_string_lossy().into()).await.unwrap();
    let secrets = Arc::new(crate::secrets::backends::InMemorySecretStore::new());
    crate::secrets::wallet::store_seed_phrase(
        &*secrets,
        "w",
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        &Default::default(),
        None,
    )
    .unwrap();
    service.set_secret_store(secrets);
    service
        .upsert_wallet(WalletState::single_address(
            "w",
            "W",
            crate::registry::Chain::Ethereum,
            "0x1111111111111111111111111111111111111111",
            None,
            false,
        ))
        .await
        .unwrap();
    service
        .reserve_receive_index("w".into(), crate::registry::Chain::Ethereum, 0)
        .await
        .unwrap();
    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: "w".into(),
        })
        .await
        .unwrap();
    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: "w".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        service.reveal_seed_phrase("w".into(), None).unwrap(),
        crate::service::SeedPhraseReveal::NotStored,
        "deleting the wallet deletes its secret"
    );
    assert!(service.keypool.read().await.is_empty());
    let reopened = WalletService::new(vec![]).unwrap();
    assert!(
        reopened
            .opened(path.to_string_lossy().into())
            .await
            .unwrap()
            .wallets
            .is_empty()
    );
    assert!(reopened.keypool.read().await.is_empty());
}
