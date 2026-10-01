use super::*;
use crate::wallet_db::error::DbError;

mod connection;

/// A database no other test can be holding. Tests run in parallel, so the name
/// is keyed on process, thread and a counter, which cannot collide.
fn tmp_db() -> std::sync::Arc<WalletDatabase> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "wallet_db_test_{}_{:?}_{}.sqlite",
        std::process::id(),
        std::thread::current().id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    WalletDatabase::new(path.to_str().unwrap())
}

fn keypool_of(
    db: &WalletDatabase,
    wallet_id: &str,
    chain_id: crate::registry::Chain,
) -> Option<KeypoolState> {
    keypool_load_all(db)
        .unwrap()
        .get(&chain_id)
        .and_then(|wallets| wallets.get(wallet_id))
        .cloned()
}

fn addresses_of(
    db: &WalletDatabase,
    wallet_id: &str,
    chain_id: crate::registry::Chain,
) -> Vec<OwnedAddressRecord> {
    address_load_all_chains(db)
        .unwrap()
        .into_iter()
        .filter(|r| r.wallet_id == wallet_id && r.chain_id == chain_id)
        .collect()
}

#[test]
fn keypool_history_projection_is_scoped_indexed_and_tracks_edits() {
    let db = tmp_db();
    with_conn(&db, |conn| {
            // Only paths: deliberately not a decodable full transaction record.
            // The baseline query must not fetch/decode transaction bodies.
            for (id, wallet, chain, external, change) in [
                ("a", "w", "bitcoin", 4, 2), ("b", "w", "bitcoin", 4, 2),
                ("c", "w", "bitcoin", 8, 3), ("d", "other", "bitcoin", 90, 90),
                ("e", "w", "litecoin", 99, 99),
            ] {
                let payload = serde_json::json!({
                    "id": id, "kind": "receive", "status": "confirmed",
                    "sourceDerivationPath": format!("m/84'/0'/0'/0/{external}"),
                    "changeDerivationPath": format!("m/84'/0'/0'/1/{change}"),
                }).to_string();
                conn.execute("INSERT INTO history_records (id,wallet_id,chain_id,created_at,payload) VALUES (?1,?2,?3,0,?4)",
                    params![id,wallet,chain,payload]).unwrap();
            }
            for (field, index) in [("sourceDerivationPath", "idx_hr_source_path"), ("changeDerivationPath", "idx_hr_change_path")] {
                let plan: Vec<String> = conn.prepare(&format!("EXPLAIN QUERY PLAN SELECT DISTINCT json_extract(payload, '$.{field}') FROM history_records WHERE wallet_id = 'w' AND chain_id = 'Bitcoin'"))
                    .unwrap().query_map([], |r| r.get(3)).unwrap().map(Result::unwrap).collect();
                assert!(plan.iter().any(|p| p.contains(index)), "{plan:?}");
                assert!(!plan.iter().any(|p| p.contains("TEMP B-TREE")), "{plan:?}");
            }
            Ok::<_, DbError>(())
        }).unwrap();
    assert_eq!(
        history_keypool_indices(&db, "W", crate::registry::Chain::Bitcoin).unwrap(),
        (Some(8), Some(3))
    );
    history_delete(&db, &["c".into()]).unwrap();
    assert_eq!(
        history_keypool_indices(&db, "w", crate::registry::Chain::Bitcoin).unwrap(),
        (Some(4), Some(2))
    );
    with_conn(&db, |conn| {
        conn.execute(
            "UPDATE history_records SET payload = ?1 WHERE id = 'a'",
            params![serde_json::json!({"id":"a","kind":"receive","status":"confirmed","sourceDerivationPath":"m/84'/0'/0'/0/12"}).to_string()],
        )
        .unwrap();
        Ok::<_, DbError>(())
    })
    .unwrap();
    assert_eq!(
        history_keypool_indices(&db, "w", crate::registry::Chain::Bitcoin).unwrap(),
        (Some(12), Some(2))
    );
    history_clear(&db).unwrap();
    assert_eq!(
        history_keypool_indices(&db, "w", crate::registry::Chain::Bitcoin).unwrap(),
        (None, None)
    );
}

/// Malformed metadata refuses loading without rewriting stored bytes.
#[test]
fn unreadable_metadata_refuses_loading() {
    for key in [META_TOKEN_PREFERENCES, META_PRICE_ALERTS, META_FIAT_RATES] {
        let db = tmp_db();
        let saved = CoreAppState {
            wallets: vec![wallet("w1", crate::registry::Chain::Bitcoin)],
            selected_wallet_id: Some("w1".to_string()),
            ..CoreAppState::default()
        };
        app_state_save(&db, &saved).unwrap();
        for raw in ["{broken", "null"] {
            with_conn(&db, |conn| {
                conn.execute(
                    "INSERT INTO app_state_meta (key, value) VALUES (?2, ?1)
                         ON CONFLICT(key) DO UPDATE SET value = ?1",
                    params![raw, key],
                )
                .unwrap();
                Ok::<_, DbError>(())
            })
            .unwrap();

            assert!(app_state_load(&db).is_err(), "{key}: {raw}");
            assert_eq!(wallet_load_all(&db).unwrap(), saved.wallets);
            let stored: String = with_conn(&db, |conn| {
                conn.query_row(
                    "SELECT value FROM app_state_meta WHERE key = ?1",
                    params![key],
                    |row| row.get(0),
                )
                .map_err(DbError::from)
            })
            .unwrap();
            assert_eq!(stored, raw);
        }
    }
}

/// Settings and the wallet rows stay fatal: they are the state, not a
/// cache of it.
#[test]
fn unreadable_settings_still_fail_the_load() {
    let db = tmp_db();
    app_state_save(&db, &CoreAppState::default()).unwrap();
    with_conn(&db, |conn| {
        conn.execute(
            "UPDATE app_state_meta SET value = ?1 WHERE key = ?2",
            params!["{broken", META_SETTINGS],
        )
        .unwrap();
        Ok::<_, DbError>(())
    })
    .unwrap();
    assert!(
        app_state_load(&db)
            .unwrap_err()
            .to_string()
            .contains("settings")
    );
}

#[test]
fn keypool_round_trip() {
    let db = tmp_db();
    let state = KeypoolState {
        next_external_index: 5,
        next_change_index: 2,
        reserved_receive_index: Some(4),
    };
    keypool_save(&db, "wallet-1", crate::registry::Chain::Bitcoin, &state).unwrap();
    let loaded = keypool_of(&db, "wallet-1", crate::registry::Chain::Bitcoin).unwrap();
    assert_eq!(loaded.next_external_index, 5);
    assert_eq!(loaded.next_change_index, 2);
    assert_eq!(loaded.reserved_receive_index, Some(4));
}

#[test]
fn keypool_upsert_updates_existing() {
    let db = tmp_db();
    let first = KeypoolState {
        next_external_index: 0,
        next_change_index: 0,
        reserved_receive_index: None,
    };
    keypool_save(&db, "wallet-1", crate::registry::Chain::Dogecoin, &first).unwrap();
    let updated = KeypoolState {
        next_external_index: 10,
        next_change_index: 3,
        reserved_receive_index: Some(9),
    };
    keypool_save(&db, "wallet-1", crate::registry::Chain::Dogecoin, &updated).unwrap();
    let loaded = keypool_of(&db, "wallet-1", crate::registry::Chain::Dogecoin).unwrap();
    assert_eq!(loaded.next_external_index, 10);
    assert_eq!(loaded.reserved_receive_index, Some(9));
}

#[test]
fn keypool_load_all_groups_by_chain() {
    let db = tmp_db();
    keypool_save(
        &db,
        "w1",
        crate::registry::Chain::Bitcoin,
        &KeypoolState {
            next_external_index: 1,
            next_change_index: 0,
            reserved_receive_index: None,
        },
    )
    .unwrap();
    keypool_save(
        &db,
        "w2",
        crate::registry::Chain::Bitcoin,
        &KeypoolState {
            next_external_index: 2,
            next_change_index: 1,
            reserved_receive_index: None,
        },
    )
    .unwrap();
    keypool_save(
        &db,
        "w1",
        crate::registry::Chain::Dogecoin,
        &KeypoolState {
            next_external_index: 5,
            next_change_index: 2,
            reserved_receive_index: Some(4),
        },
    )
    .unwrap();
    let all = keypool_load_all(&db).unwrap();
    assert_eq!(
        all[&crate::registry::Chain::Bitcoin]["w1"].next_external_index,
        1
    );
    assert_eq!(
        all[&crate::registry::Chain::Bitcoin]["w2"].next_external_index,
        2
    );
    assert_eq!(
        all[&crate::registry::Chain::Dogecoin]["w1"].reserved_receive_index,
        Some(4)
    );
}

#[test]
fn address_round_trip() {
    let db = tmp_db();
    let rec = OwnedAddressRecord {
        wallet_id: "w1".to_string(),
        chain_id: crate::registry::Chain::Bitcoin,
        address: "bc1qtest".to_string(),
        derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
        branch: Some("external".to_string()),
        branch_index: Some(0),
    };
    address_save(&db, &rec).unwrap();
    let records = addresses_of(&db, "w1", crate::registry::Chain::Bitcoin);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].address, "bc1qtest");
    assert_eq!(records[0].branch.as_deref(), Some("external"));
}

use crate::store::state::{AppSettings, WalletAddress};

/// Minimal history record. The payload is decoded from JSON rather than
/// built field-by-field: `CorePersistedTransactionRecord` has ~30 fields of
/// which only these are required, and going through serde keeps the helper
/// honest about which ones those are.
fn history_record(id: &str, wallet_id: &str) -> HistoryRecord {
    history_record_on(id, wallet_id, crate::registry::Chain::Bitcoin)
}

fn history_record_on(id: &str, wallet_id: &str, chain_id: crate::registry::Chain) -> HistoryRecord {
    let payload = serde_json::from_value(serde_json::json!({
        "id": id,
        "walletId": wallet_id,
        "kind": "send",
        "status": "pending",
        "walletName": "Wallet",
        "assetDisplayName": chain_id,
        "symbol": "BTC",
        "chainId": chain_id,
        "amount": "1",
        "address": "bc1qexample",
        "createdAtUnix": 0.0,
    }))
    .expect("history payload fixture must match CorePersistedTransactionRecord");
    HistoryRecord {
        id: id.to_string(),
        wallet_id: Some(wallet_id.to_string()),
        chain_id,
        tx_hash: Some(format!("hash-{id}")),
        created_at: 0.0,
        payload,
    }
}

fn wallet(id: &str, chain: crate::registry::Chain) -> WalletState {
    WalletState {
        id: id.to_string(),
        name: format!("Wallet {id}"),
        signing: crate::store::state::WalletSigning::SeedPhrase {
            password_protected: false,
        },
        chain_id: chain,
        include_in_portfolio_total: true,
        xpub: None,
        derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
        derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
        derivation_overrides: Default::default(),
        holdings: Vec::new(),
        addresses: vec![WalletAddress {
            chain_id: chain,
            address: format!("addr-{id}"),
            kind: "receive".to_string(),
            derivation_path: None,
        }],
    }
}

#[test]
fn app_state_load_on_empty_db_is_default() {
    let db = tmp_db();
    assert_eq!(app_state_load(&db).unwrap(), CoreAppState::default());
}

#[test]
fn persisted_floats_round_trip_without_changing_bits() {
    let db = tmp_db();
    let mut state = CoreAppState::default();
    // Adjacent representable timestamps exercise decimal parsing without relying
    // on the wall clock to happen to produce a value that loses precision.
    for offset in 0..256 {
        let timestamp = f64::from_bits(1_789_862_724.180_276_6_f64.to_bits() + offset);
        state
            .diagnostics
            .last_good_unix
            .insert(crate::registry::Chain::Bitcoin, timestamp);

        app_state_save(&db, &state).unwrap();
        let loaded = app_state_load(&db).unwrap();
        assert_eq!(
            loaded.diagnostics.last_good_unix[&crate::registry::Chain::Bitcoin].to_bits(),
            timestamp.to_bits(),
            "timestamp {timestamp} changed after reopening"
        );
    }
}

#[test]
fn app_state_round_trips() {
    let db = tmp_db();
    let state = CoreAppState {
        revision: 0,
        movement_baseline: None,
        diagnostics: Default::default(),
        quotes: Default::default(),
        schema_version: 2,
        wallets: vec![
            wallet("w1", crate::registry::Chain::Bitcoin),
            wallet("w2", crate::registry::Chain::Ethereum),
        ],
        selected_wallet_id: Some("w2".to_string()),
        settings: AppSettings {
            fiat_currency: crate::store::state::FiatCurrency::Cny,
            pinned_dashboard_token_ids: vec!["bitcoin".to_string()],
            // Every other field is a settings field;
            // `every_settings_field_round_trips` covers them together.
            ..AppSettings::default()
        },
        token_preferences: Vec::new(),
        price_alerts: Vec::new(),
        fiat_rates_from_usd: std::collections::HashMap::new(),
        address_book: vec![AddressBookEntry {
            id: "ab1".to_string(),
            name: "Cold".to_string(),
            chain_id: crate::registry::Chain::Bitcoin,
            address: "bc1qexample".to_string(),
            note: "vault".to_string(),
        }],
    };
    app_state_save(&db, &state).unwrap();
    assert_eq!(app_state_load(&db).unwrap(), state);
}

#[test]
fn app_state_save_preserves_wallet_order() {
    let db = tmp_db();
    // Ids deliberately out of lexicographic order, so a load that sorted by
    // id instead of position would fail here.
    let ordered = vec![
        wallet("zz", crate::registry::Chain::Bitcoin),
        wallet("aa", crate::registry::Chain::Solana),
        wallet("mm", crate::registry::Chain::Sui),
    ];
    let state = CoreAppState {
        revision: 0,
        movement_baseline: None,
        diagnostics: Default::default(),
        quotes: Default::default(),
        wallets: ordered.clone(),
        ..CoreAppState::default()
    };
    app_state_save(&db, &state).unwrap();
    let ids: Vec<String> = app_state_load(&db)
        .unwrap()
        .wallets
        .iter()
        .map(|w| w.id.clone())
        .collect();
    assert_eq!(ids, vec!["zz", "aa", "mm"]);
}

#[test]
fn app_state_save_prunes_removed_wallets() {
    let db = tmp_db();
    app_state_save(
        &db,
        &CoreAppState {
            wallets: vec![
                wallet("w1", crate::registry::Chain::Bitcoin),
                wallet("w2", crate::registry::Chain::Ethereum),
            ],
            selected_wallet_id: Some("w1".to_string()),
            ..CoreAppState::default()
        },
    )
    .unwrap();
    app_state_save(
        &db,
        &CoreAppState {
            wallets: vec![wallet("w2", crate::registry::Chain::Ethereum)],
            ..CoreAppState::default()
        },
    )
    .unwrap();

    let loaded = app_state_load(&db).unwrap();
    assert_eq!(loaded.wallets.len(), 1);
    assert_eq!(loaded.wallets[0].id, "w2");
    assert!(!wallet_load_all(&db).unwrap().iter().any(|w| w.id == "w1"));
    // Clearing the selection must clear the stored row, not leave the stale
    // id behind.
    assert_eq!(loaded.selected_wallet_id, None);
}

#[test]
fn incremental_state_reorders_deletes_and_rolls_back_as_one_transaction() {
    let db = tmp_db();
    let before = CoreAppState {
        wallets: vec![
            wallet("a", crate::registry::Chain::Bitcoin),
            wallet("b", crate::registry::Chain::Solana),
            wallet("c", crate::registry::Chain::Sui),
        ],
        selected_wallet_id: Some("a".into()),
        address_book: vec![AddressBookEntry {
            id: "a".into(),
            name: "Alice".into(),
            chain_id: crate::registry::Chain::Bitcoin,
            address: "recipient".into(),
            note: "".into(),
        }],
        ..CoreAppState::default()
    };
    app_state_save(&db, &before).unwrap();
    let mut after = before.clone();
    after.wallets.remove(0);
    after.wallets.reverse();
    after.wallets[0].name = "Renamed".into();
    after.selected_wallet_id = None;
    after.address_book.clear();
    after.settings.fiat_currency = crate::store::state::FiatCurrency::Eur;
    with_conn(&db, |conn| conn.execute_batch("CREATE TRIGGER reject_meta BEFORE INSERT ON app_state_meta BEGIN SELECT RAISE(FAIL, 'injected'); END;").map_err(DbError::from)).unwrap();
    assert!(
        AppStateChanges::between(Some(&before), &after)
            .unwrap()
            .save(&db)
            .is_err()
    );
    assert_eq!(app_state_load(&db).unwrap(), before);
    with_conn(&db, |conn| {
        conn.execute_batch("DROP TRIGGER reject_meta;")
            .map_err(DbError::from)
    })
    .unwrap();
    AppStateChanges::between(Some(&before), &after)
        .unwrap()
        .save(&db)
        .unwrap();
    assert_eq!(app_state_load(&db).unwrap(), after);
}

#[test]
fn wallet_upsert_appends_then_updates_in_place() {
    let db = tmp_db();
    wallet_upsert(&db, &wallet("w1", crate::registry::Chain::Bitcoin)).unwrap();
    wallet_upsert(&db, &wallet("w2", crate::registry::Chain::Ethereum)).unwrap();

    let mut renamed = wallet("w1", crate::registry::Chain::Bitcoin);
    renamed.name = "Renamed".to_string();
    renamed.include_in_portfolio_total = false;
    wallet_upsert(&db, &renamed).unwrap();

    let all = wallet_load_all(&db).unwrap();
    assert_eq!(all.len(), 2, "upsert must not duplicate an existing wallet");
    // w1 keeps position 0 across the update.
    assert_eq!(all[0].id, "w1");
    assert_eq!(all[0].name, "Renamed");
    assert!(!all[0].include_in_portfolio_total);
    assert_eq!(all[1].id, "w2");
}

/// `history_fetch_for_wallet` filters in SQL, not by fetching every row and
/// discarding most of them in Rust: a wallet's worth of rows is what a caller
/// asking for one should pay for.
#[test]
fn scoped_history_fetches_return_only_their_own_rows() {
    let db = tmp_db();
    history_upsert_batch(
        &db,
        &[
            history_record_on("btc-w1", "w1", crate::registry::Chain::Bitcoin),
            history_record_on("btc-w2", "w2", crate::registry::Chain::Bitcoin),
            history_record_on("eth-w1", "w1", crate::registry::Chain::Ethereum),
        ],
    )
    .unwrap();

    let w1: Vec<String> = history_fetch_for_wallet(&db, "w1")
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(
        std::collections::BTreeSet::from_iter(w1),
        std::collections::BTreeSet::from(["btc-w1".to_string(), "eth-w1".to_string()])
    );
}

/// Removing a wallet takes its row, derivation state and history, and leaves
/// every other wallet's alone.
#[test]
fn removing_a_wallet_takes_its_rows_and_leaves_the_others() {
    let db = tmp_db();
    let before = CoreAppState {
        wallets: vec![
            wallet("w1", crate::registry::Chain::Bitcoin),
            wallet("w2", crate::registry::Chain::Bitcoin),
        ],
        ..CoreAppState::default()
    };
    app_state_save(&db, &before).unwrap();
    for id in ["w1", "w2"] {
        keypool_save(
            &db,
            id,
            crate::registry::Chain::Bitcoin,
            &KeypoolState {
                next_external_index: 1,
                next_change_index: 0,
                reserved_receive_index: None,
            },
        )
        .unwrap();
        address_save(
            &db,
            &OwnedAddressRecord {
                wallet_id: id.to_string(),
                chain_id: crate::registry::Chain::Bitcoin,
                address: format!("{id}-addr"),
                derivation_path: None,
                branch: None,
                branch_index: None,
            },
        )
        .unwrap();
    }
    history_upsert_batch(
        &db,
        &[history_record("tx1", "w1"), history_record("tx2", "w2")],
    )
    .unwrap();

    let mut after = before.clone();
    after.wallets.remove(0);
    AppStateChanges::between(Some(&before), &after)
        .unwrap()
        .save(&db)
        .unwrap();

    let wallets: Vec<String> = wallet_load_all(&db)
        .unwrap()
        .into_iter()
        .map(|w| w.id)
        .collect();
    assert_eq!(wallets, vec!["w2"]);
    assert!(keypool_of(&db, "w1", crate::registry::Chain::Bitcoin).is_none());
    assert!(keypool_of(&db, "w2", crate::registry::Chain::Bitcoin).is_some());
    assert!(addresses_of(&db, "w1", crate::registry::Chain::Bitcoin).is_empty());
    assert_eq!(
        addresses_of(&db, "w2", crate::registry::Chain::Bitcoin).len(),
        1
    );
    let remaining: Vec<String> = history_fetch_all(&db)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(remaining, vec!["tx2"], "only w1's history should be gone");
}

#[test]
fn history_id_lookup_uses_the_primary_key_and_normalizes_duplicates() {
    let db = std::env::temp_dir().join(format!(
        "history-ids-{}.sqlite",
        crate::store::new_event_id()
    ));
    let db = WalletDatabase::new(db.to_str().unwrap());
    let rows: Vec<_> = (0..1100)
        .map(|i| history_record(&format!("tx{i}"), "w"))
        .collect();
    history_upsert_batch(&db, &rows).unwrap();
    let mut ids: Vec<_> = (0..1100).map(|i| format!("TX{i}")).collect();
    ids.extend(["tx0".into(), "absent".into()]);
    assert_eq!(history_existing_ids(&db, &ids).unwrap().len(), 1100);
    with_conn(&db, |conn| {
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT id FROM history_records WHERE id = ?1",
                params!["tx0"],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("SEARCH") && !plan.contains("SCAN"), "{plan}");
        Ok::<_, DbError>(())
    })
    .unwrap();
    history_delete_for_wallet(&db, "W").unwrap();
    assert!(history_fetch_all(&db).unwrap().is_empty());
}

#[test]
fn unknown_metadata_and_schema_versions_are_refused() {
    for (key, value) in [
        ("unknown_key", "null"),
        (META_SCHEMA_VERSION, "1"),
        (META_SCHEMA_VERSION, "3"),
    ] {
        let db = tmp_db();
        app_state_save(&db, &CoreAppState::default()).unwrap();
        with_conn(&db, |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO app_state_meta (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .unwrap();
            Ok::<_, DbError>(())
        })
        .unwrap();
        assert!(app_state_load(&db).is_err());
    }
}

#[test]
fn monero_scan_cache_is_network_scoped_and_rejects_stale_writers() {
    let db = tmp_db();
    monero_save(
        &db,
        "wallet",
        crate::registry::Chain::Monero,
        None,
        "encrypted-mainnet",
    )
    .unwrap();
    monero_save(
        &db,
        "wallet",
        crate::registry::Chain::MoneroStagenet,
        None,
        "encrypted-stagenet",
    )
    .unwrap();
    assert_eq!(
        monero_load(&db, "wallet", crate::registry::Chain::Monero)
            .unwrap()
            .unwrap()
            .1,
        "encrypted-mainnet"
    );
    monero_save(
        &db,
        "wallet",
        crate::registry::Chain::Monero,
        Some(0),
        "next-batch",
    )
    .unwrap();
    assert!(
        monero_save(
            &db,
            "wallet",
            crate::registry::Chain::Monero,
            Some(0),
            "stale"
        )
        .is_err()
    );
    assert_eq!(
        monero_load(&db, "wallet", crate::registry::Chain::Monero).unwrap(),
        Some((1, "next-batch".into()))
    );
    assert_eq!(
        monero_load(&db, "wallet", crate::registry::Chain::MoneroStagenet).unwrap(),
        Some((0, "encrypted-stagenet".into()))
    );
}

#[test]
fn history_batches_roll_back_partial_writes_and_leave_connection_usable() {
    let db = tmp_db();
    let original = history_record_on("original", "w1", crate::registry::Chain::Bitcoin);
    history_upsert_batch(&db, std::slice::from_ref(&original)).unwrap();
    with_conn(&db, |conn| {
        conn.execute_batch("CREATE TRIGGER reject_history BEFORE INSERT ON history_records WHEN NEW.id = 'reject' BEGIN SELECT RAISE(FAIL, 'injected'); END;").map_err(DbError::from)
    }).unwrap();
    let batch = [
        history_record_on("new", "w1", crate::registry::Chain::Bitcoin),
        history_record_on("reject", "w1", crate::registry::Chain::Bitcoin),
    ];
    assert!(history_upsert_batch(&db, &batch).is_err());
    let rows = history_fetch_all(&db).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "original");
    with_conn(&db, |conn| {
        conn.execute_batch("DROP TRIGGER reject_history")
            .map_err(DbError::from)
    })
    .unwrap();
    history_upsert_batch(&db, &batch).unwrap();
    assert_eq!(history_fetch_all(&db).unwrap().len(), 3);
    with_conn(&db, |conn| conn.execute_batch("CREATE TRIGGER reject_delete BEFORE DELETE ON history_records WHEN OLD.id = 'reject' BEGIN SELECT RAISE(FAIL, 'injected'); END;").map_err(DbError::from)).unwrap();
    assert!(history_delete(&db, &["new".into(), "reject".into()]).is_err());
    assert_eq!(history_fetch_all(&db).unwrap().len(), 3);
    with_conn(&db, |conn| {
        conn.execute_batch("DROP TRIGGER reject_delete")
            .map_err(DbError::from)
    })
    .unwrap();
    history_delete(&db, &["new".into(), "reject".into()]).unwrap();
    assert_eq!(history_fetch_all(&db).unwrap().len(), 1);
}

#[test]
fn pending_sender_query_uses_index_and_excludes_unrelated_history() {
    let db = tmp_db();
    let mut pending = history_record_on("pending", "w1", crate::registry::Chain::Ethereum);
    pending.payload.kind = crate::store::wallet_domain::CoreTransactionKind::Send;
    pending.payload.status = crate::store::wallet_domain::CoreTransactionStatus::Pending;
    pending.payload.source_address = Some("0xAbC".into());
    pending.payload.nonce = Some(7);
    let mut confirmed = pending.clone();
    confirmed.id = "confirmed".into();
    confirmed.payload.id = confirmed.id.clone();
    confirmed.payload.status = crate::store::wallet_domain::CoreTransactionStatus::Confirmed;
    let mut other = pending.clone();
    other.id = "other".into();
    other.payload.id = other.id.clone();
    other.payload.source_address = Some("0xdef".into());
    history_upsert_batch(&db, &[pending, confirmed, other]).unwrap();
    let rows = history_pending_for_sender(&db, crate::registry::Chain::Ethereum, "0xabc").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].payload.nonce, Some(7));
    assert!(
        history_pending_for_sender(&db, crate::registry::Chain::Base, "0xabc")
            .unwrap()
            .is_empty()
    );
    with_conn(&db, |conn| {
        let plan: Vec<String> = conn.prepare("EXPLAIN QUERY PLAN SELECT payload FROM history_records WHERE chain_id = 'Ethereum' AND lower(json_extract(payload, '$.sourceAddress')) = '0xabc' AND json_extract(payload, '$.kind') = 'send' AND json_extract(payload, '$.status') = 'pending'")
            .unwrap().query_map([], |r| r.get(3)).unwrap().map(Result::unwrap).collect();
        assert!(plan.iter().any(|line| line.contains("idx_hr_pending_sender")), "{plan:?}");
        Ok::<_, DbError>(())
    }).unwrap();
}

#[test]
fn failed_history_commit_rolls_back_and_allows_retry() {
    let db = tmp_db();
    let row = history_record_on("commit", "w1", crate::registry::Chain::Bitcoin);
    with_conn(&db, |conn| {
        conn.execute_batch("PRAGMA foreign_keys = ON;
            CREATE TABLE commit_parent (id INTEGER PRIMARY KEY);
            CREATE TABLE commit_child (parent INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_history_commit AFTER INSERT ON history_records BEGIN INSERT INTO commit_child VALUES (1); END;")
            .map_err(DbError::from)
    }).unwrap();
    assert!(history_upsert_batch(&db, std::slice::from_ref(&row)).is_err());
    assert!(history_fetch_all(&db).unwrap().is_empty());
    with_conn(&db, |conn| {
        assert!(
            conn.is_autocommit(),
            "failed commit must not strand an open transaction"
        );
        let count: i64 = conn
            .query_row("SELECT count(*) FROM commit_child", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        Ok::<_, DbError>(())
    })
    .unwrap();
    with_conn(&db, |conn| {
        conn.execute_batch("INSERT INTO commit_parent VALUES (1)")
            .map_err(DbError::from)
    })
    .unwrap();
    history_upsert_batch(&db, &[row]).unwrap();
    assert_eq!(history_fetch_all(&db).unwrap().len(), 1);
}

/// `hide_small_amounts` leaves out transfers below the threshold, zero-value
/// ones included, and keeps the threshold itself; a cursor from one setting
/// does not continue the other.
#[test]
fn history_pages_can_hide_small_amounts() {
    let db = tmp_db();
    app_state_save(
        &db,
        &CoreAppState {
            wallets: vec![wallet("w1", crate::registry::Chain::Bitcoin)],
            ..CoreAppState::default()
        },
    )
    .unwrap();
    let rows: Vec<HistoryRecord> = [
        ("zero", "0"),
        ("dust", "0.000009"),
        ("edge", "0.00001"),
        ("one", "1"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (id, amount))| {
        let mut row = history_record(id, "w1");
        row.created_at = i as f64;
        row.payload.amount = amount.to_string();
        row.payload.created_at_unix = i as f64;
        row
    })
    .collect();
    history_upsert_batch(&db, &rows).unwrap();
    let page = |hide_small_amounts: bool, cursor: Option<String>| {
        history_page(
            &db,
            &crate::service::HistoryQuery {
                hide_small_amounts,
                cursor,
                limit: 1,
                ..Default::default()
            },
        )
    };
    let ids = |hide: bool| {
        let mut ids = Vec::new();
        let mut cursor = None;
        loop {
            let p = page(hide, cursor).unwrap();
            ids.extend(p.records.into_iter().map(|r| r.id));
            match p.next_cursor {
                Some(next) => cursor = Some(next),
                None => break ids,
            }
        }
    };
    assert_eq!(ids(false), ["one", "edge", "dust", "zero"]);
    assert_eq!(ids(true), ["one", "edge"]);
    let cursor = page(false, None).unwrap().next_cursor;
    assert!(page(true, cursor).is_err());
}

/// An undated pending transaction is the newest row, in both directions and
/// across pages, and an undated confirmed one the oldest; neither counts as
/// the wallet's earliest dated transaction.
#[test]
fn undated_pending_transactions_sort_as_the_newest() {
    let db = tmp_db();
    app_state_save(
        &db,
        &CoreAppState {
            wallets: vec![wallet("w1", crate::registry::Chain::Bitcoin)],
            ..CoreAppState::default()
        },
    )
    .unwrap();
    let unknown = -62_135_596_800.0;
    let rows: Vec<HistoryRecord> = [
        ("dated", "confirmed", 1_700_000_000.0),
        ("undated-pending", "pending", unknown),
        ("undated-confirmed", "confirmed", unknown),
        ("recent", "pending", 1_800_000_000.0),
    ]
    .iter()
    .map(|(id, status, time)| {
        let mut payload = history_record(id, "w1").payload;
        payload.status = serde_json::from_value(serde_json::json!(status)).unwrap();
        payload.created_at_unix = *time;
        history_record_from_payload(payload)
    })
    .collect();
    history_upsert_batch(&db, &rows).unwrap();
    let ids = |oldest_first: bool| {
        let mut ids = Vec::new();
        let mut cursor = None;
        loop {
            let page = history_page(
                &db,
                &crate::service::HistoryQuery {
                    oldest_first,
                    cursor,
                    limit: 1,
                    ..Default::default()
                },
            )
            .unwrap();
            ids.extend(page.records.into_iter().map(|r| r.id));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break ids,
            }
        }
    };
    assert_eq!(
        ids(false),
        ["undated-pending", "recent", "dated", "undated-confirmed"]
    );
    assert_eq!(
        ids(true),
        ["undated-confirmed", "dated", "recent", "undated-pending"]
    );
    let undated = history_find(&db, "undated-pending").unwrap().unwrap();
    assert_eq!(undated.created_at_unix, unknown, "still reads as undated");
    let sequence = std::sync::atomic::AtomicU64::new(0);
    let snapshot = history_snapshot(&db, &sequence).unwrap();
    assert_eq!(
        snapshot.earliest[0].earliest_created_at_unix,
        1_700_000_000.0
    );
}
