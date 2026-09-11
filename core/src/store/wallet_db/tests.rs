use super::*;

/// A database no other test can be holding.
///
/// This used to key on `subsec_nanos()` alone. Thirteen tests share the
/// helper and the runner runs them in parallel, so two could land on the
/// same nanosecond and read each other's rows — which is exactly how
/// `app_state_round_trips` failed in a full run and passed on its own.
/// Process, thread and a counter cannot collide.
fn tmp_db() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "wallet_db_test_{}_{:?}_{}.sqlite",
        std::process::id(),
        std::thread::current().id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

#[test]
fn keypool_history_projection_is_scoped_indexed_and_tracks_edits() {
    let db = tmp_db();
    with_conn(&db, |conn| {
            // Only paths: deliberately not a decodable full transaction record.
            // The baseline query must not fetch/decode transaction bodies.
            for (id, wallet, chain, external, change) in [
                ("a", "w", "Bitcoin", 4, 2), ("b", "w", "Bitcoin", 4, 2),
                ("c", "w", "Bitcoin", 8, 3), ("d", "other", "Bitcoin", 90, 90),
                ("e", "w", "Litecoin", 99, 99),
            ] {
                let payload = serde_json::json!({
                    "sourceDerivationPath": format!("m/84'/0'/0'/0/{external}"),
                    "changeDerivationPath": format!("m/84'/0'/0'/1/{change}"),
                }).to_string();
                conn.execute("INSERT INTO history_records (id,wallet_id,chain_name,created_at,payload) VALUES (?1,?2,?3,0,?4)",
                    params![id,wallet,chain,payload]).unwrap();
            }
            for (field, index) in [("sourceDerivationPath", "idx_hr_source_path"), ("changeDerivationPath", "idx_hr_change_path")] {
                let plan: Vec<String> = conn.prepare(&format!("EXPLAIN QUERY PLAN SELECT DISTINCT json_extract(payload, '$.{field}') FROM history_records WHERE wallet_id = 'w' AND chain_name = 'Bitcoin'"))
                    .unwrap().query_map([], |r| r.get(3)).unwrap().map(Result::unwrap).collect();
                assert!(plan.iter().any(|p| p.contains(index)), "{plan:?}");
                assert!(!plan.iter().any(|p| p.contains("TEMP B-TREE")), "{plan:?}");
            }
            Ok(())
        }).unwrap();
    assert_eq!(
        history_keypool_indices(&db, "W", "Bitcoin").unwrap(),
        (Some(8), Some(3))
    );
    history_delete(&db, &["c".into()]).unwrap();
    assert_eq!(
        history_keypool_indices(&db, "w", "Bitcoin").unwrap(),
        (Some(4), Some(2))
    );
    with_conn(&db, |conn| {
        conn.execute(
            "UPDATE history_records SET payload = ?1 WHERE id = 'a'",
            params![serde_json::json!({"sourceDerivationPath":"m/84'/0'/0'/0/12"}).to_string()],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(
        history_keypool_indices(&db, "w", "Bitcoin").unwrap(),
        (Some(12), Some(2))
    );
    history_clear(&db).unwrap();
    assert_eq!(
        history_keypool_indices(&db, "w", "Bitcoin").unwrap(),
        (None, None)
    );
}

/// A rebuildable row this build cannot read costs that row, not the
/// wallet list. Failing the load over a price alert would lose the one
/// thing in the file that cannot be rebuilt from anything.
#[test]
fn unreadable_rebuildable_metadata_costs_only_that_row() {
    for key in [META_TOKEN_PREFERENCES, META_PRICE_ALERTS, META_FIAT_RATES] {
        let db = tmp_db();
        let saved = CoreAppState {
            wallets: vec![wallet("w1", "Bitcoin")],
            selected_wallet_id: Some("w1".to_string()),
            ..CoreAppState::default()
        };
        app_state_save(&db, &saved).unwrap();
        for raw in ["{broken", "{}", "null"] {
            with_conn(&db, |conn| {
                conn.execute(
                    "INSERT INTO app_state_meta (key, value) VALUES (?2, ?1)
                         ON CONFLICT(key) DO UPDATE SET value = ?1",
                    params![raw, key],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();

            let loaded = app_state_load(&db).expect("a cache row cannot fail the load");
            assert_eq!(
                loaded.wallets, saved.wallets,
                "{key} took the wallets with it"
            );
            assert_eq!(loaded.selected_wallet_id, saved.selected_wallet_id);
            match key {
                META_TOKEN_PREFERENCES => assert!(loaded.token_preferences.is_empty()),
                META_PRICE_ALERTS => assert!(loaded.price_alerts.is_empty()),
                _ => assert!(loaded.fiat_rates_from_usd.is_empty()),
            }

            // Left on disk, so a build that can read it still will.
            let stored: String = with_conn(&db, |conn| {
                conn.query_row(
                    "SELECT value FROM app_state_meta WHERE key = ?1",
                    params![key],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())
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
        Ok(())
    })
    .unwrap();
    assert!(app_state_load(&db).unwrap_err().contains("settings"));
}

#[test]
fn keypool_round_trip() {
    let db = tmp_db();
    let state = KeypoolState {
        next_external_index: 5,
        next_change_index: 2,
        reserved_receive_index: Some(4),
    };
    keypool_save(&db, "wallet-1", "Bitcoin", &state).unwrap();
    let loaded = keypool_load(&db, "wallet-1", "Bitcoin").unwrap().unwrap();
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
    keypool_save(&db, "wallet-1", "Dogecoin", &first).unwrap();
    let updated = KeypoolState {
        next_external_index: 10,
        next_change_index: 3,
        reserved_receive_index: Some(9),
    };
    keypool_save(&db, "wallet-1", "Dogecoin", &updated).unwrap();
    let loaded = keypool_load(&db, "wallet-1", "Dogecoin").unwrap().unwrap();
    assert_eq!(loaded.next_external_index, 10);
    assert_eq!(loaded.reserved_receive_index, Some(9));
}

#[test]
fn keypool_load_all_groups_by_chain() {
    let db = tmp_db();
    keypool_save(
        &db,
        "w1",
        "Bitcoin",
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
        "Bitcoin",
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
        "Dogecoin",
        &KeypoolState {
            next_external_index: 5,
            next_change_index: 2,
            reserved_receive_index: Some(4),
        },
    )
    .unwrap();
    let all = keypool_load_all(&db).unwrap();
    assert_eq!(all["Bitcoin"]["w1"].next_external_index, 1);
    assert_eq!(all["Bitcoin"]["w2"].next_external_index, 2);
    assert_eq!(all["Dogecoin"]["w1"].reserved_receive_index, Some(4));
}

#[test]
fn keypool_delete_for_wallet() {
    let db = tmp_db();
    keypool_save(
        &db,
        "w1",
        "Bitcoin",
        &KeypoolState {
            next_external_index: 5,
            next_change_index: 1,
            reserved_receive_index: None,
        },
    )
    .unwrap();
    keypool_save(
        &db,
        "w2",
        "Bitcoin",
        &KeypoolState {
            next_external_index: 3,
            next_change_index: 0,
            reserved_receive_index: None,
        },
    )
    .unwrap();
    super::keypool_delete_for_wallet(&db, "w1").unwrap();
    assert!(keypool_load(&db, "w1", "Bitcoin").unwrap().is_none());
    assert!(keypool_load(&db, "w2", "Bitcoin").unwrap().is_some());
}

#[test]
fn address_round_trip() {
    let db = tmp_db();
    let rec = OwnedAddressRecord {
        wallet_id: "w1".to_string(),
        chain_name: "Bitcoin".to_string(),
        address: "bc1qtest".to_string(),
        derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
        branch: Some("external".to_string()),
        branch_index: Some(0),
    };
    address_save(&db, &rec).unwrap();
    let records = address_load_all(&db, "w1", "Bitcoin").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].address, "bc1qtest");
    assert_eq!(records[0].branch.as_deref(), Some("external"));
}

/// A network switch has to take both of a chain's derivation tables. The
/// caller used to issue the two deletes separately and could leave one
/// behind; the point of the combined call is that it cannot.
#[test]
fn chain_derivation_delete_takes_keypool_and_addresses_together() {
    let db = tmp_db();
    for chain in ["Bitcoin", "Litecoin"] {
        keypool_save(
            &db,
            "w1",
            chain,
            &KeypoolState {
                next_external_index: 4,
                next_change_index: 2,
                reserved_receive_index: Some(4),
            },
        )
        .unwrap();
        address_save(
            &db,
            &OwnedAddressRecord {
                wallet_id: "w1".to_string(),
                chain_name: chain.to_string(),
                address: format!("{chain}-addr"),
                derivation_path: None,
                branch: None,
                branch_index: None,
            },
        )
        .unwrap();
    }

    super::chain_derivation_delete_for_chain(&db, "Bitcoin").unwrap();

    assert!(keypool_load(&db, "w1", "Bitcoin").unwrap().is_none());
    assert!(address_load_all(&db, "w1", "Bitcoin").unwrap().is_empty());
    // The chain that did not switch keeps both halves.
    assert!(keypool_load(&db, "w1", "Litecoin").unwrap().is_some());
    assert_eq!(address_load_all(&db, "w1", "Litecoin").unwrap().len(), 1);
}

/// Deleting a chain nothing was derived on is a no-op, not an error: the
/// switch still has to go through.
#[test]
fn chain_derivation_delete_is_a_no_op_for_an_unused_chain() {
    let db = tmp_db();
    super::chain_derivation_delete_for_chain(&db, "Dogecoin").unwrap();
}

#[test]
fn delete_wallet_data_removes_both_tables() {
    let db = tmp_db();
    keypool_save(
        &db,
        "w1",
        "Dogecoin",
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
            wallet_id: "w1".to_string(),
            chain_name: "Dogecoin".to_string(),
            address: "D1test".to_string(),
            derivation_path: None,
            branch: None,
            branch_index: None,
        },
    )
    .unwrap();
    delete_wallet_data(&db, "w1").unwrap();
    assert!(keypool_load(&db, "w1", "Dogecoin").unwrap().is_none());
    assert!(address_load_all(&db, "w1", "Dogecoin").unwrap().is_empty());
}

use crate::store::state::{AppSettings, WalletAddress};

/// Minimal history record. The payload is decoded from JSON rather than
/// built field-by-field: `CorePersistedTransactionRecord` has ~30 fields of
/// which only these are required, and going through serde keeps the helper
/// honest about which ones those are.
fn history_record(id: &str, wallet_id: &str) -> HistoryRecord {
    history_record_on(id, wallet_id, "Bitcoin")
}

fn history_record_on(id: &str, wallet_id: &str, chain_name: &str) -> HistoryRecord {
    let payload = serde_json::from_value(serde_json::json!({
        "id": id,
        "walletId": wallet_id,
        "kind": "send",
        "walletName": "Wallet",
        "assetName": chain_name,
        "symbol": "BTC",
        "chainName": chain_name,
        "amount": 1.0,
        "address": "bc1qexample",
        "createdAt": 0.0,
    }))
    .expect("history payload fixture must match CorePersistedTransactionRecord");
    HistoryRecord {
        id: id.to_string(),
        wallet_id: Some(wallet_id.to_string()),
        chain_name: chain_name.to_string(),
        tx_hash: Some(format!("hash-{id}")),
        created_at: 0.0,
        payload,
    }
}

fn wallet(id: &str, chain: &str) -> WalletSummary {
    WalletSummary {
        id: id.to_string(),
        name: format!("Wallet {id}"),
        is_watch_only: false,
        chain_name: chain.to_string(),
        include_in_portfolio_total: true,
        network_mode: None,
        xpub: None,
        derivation_preset: "default".to_string(),
        derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
        derivation_overrides: Default::default(),
        holdings: Vec::new(),
        addresses: vec![WalletAddress {
            chain_name: chain.to_string(),
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
fn app_state_round_trips() {
    let db = tmp_db();
    let state = CoreAppState {
        schema_version: 2,
        wallets: vec![wallet("w1", "Bitcoin"), wallet("w2", "Ethereum")],
        selected_wallet_id: Some("w2".to_string()),
        settings: AppSettings {
            fiat_currency_code: "CNY".to_string(),
            pinned_dashboard_asset_symbols: vec!["BTC".to_string()],
            // Every other field is a settings field the blob used to hold;
            // `every_settings_field_round_trips` covers them together.
            ..AppSettings::default()
        },
        token_preferences: Vec::new(),
        price_alerts: Vec::new(),
        fiat_rates_from_usd: std::collections::HashMap::new(),
        address_book: vec![AddressBookEntry {
            id: "ab1".to_string(),
            name: "Cold".to_string(),
            chain_name: "Bitcoin".to_string(),
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
        wallet("zz", "Bitcoin"),
        wallet("aa", "Solana"),
        wallet("mm", "Sui"),
    ];
    let state = CoreAppState {
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
            wallets: vec![wallet("w1", "Bitcoin"), wallet("w2", "Ethereum")],
            selected_wallet_id: Some("w1".to_string()),
            ..CoreAppState::default()
        },
    )
    .unwrap();
    app_state_save(
        &db,
        &CoreAppState {
            wallets: vec![wallet("w2", "Ethereum")],
            ..CoreAppState::default()
        },
    )
    .unwrap();

    let loaded = app_state_load(&db).unwrap();
    assert_eq!(loaded.wallets.len(), 1);
    assert_eq!(loaded.wallets[0].id, "w2");
    assert!(wallet_load(&db, "w1").unwrap().is_none());
    // Clearing the selection must clear the stored row, not leave the stale
    // id behind.
    assert_eq!(loaded.selected_wallet_id, None);
}

#[test]
fn incremental_state_reorders_deletes_and_rolls_back_as_one_transaction() {
    let db = tmp_db();
    let before = CoreAppState {
        wallets: vec![
            wallet("a", "Bitcoin"),
            wallet("b", "Solana"),
            wallet("c", "Sui"),
        ],
        selected_wallet_id: Some("a".into()),
        address_book: vec![AddressBookEntry {
            id: "a".into(),
            name: "Alice".into(),
            chain_name: "Bitcoin".into(),
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
    after.settings.fiat_currency_code = "EUR".into();
    with_conn(&db, |conn| conn.execute_batch("CREATE TRIGGER reject_meta BEFORE INSERT ON app_state_meta BEGIN SELECT RAISE(FAIL, 'injected'); END;").map_err(|e| e.to_string())).unwrap();
    assert!(AppStateChanges::between(Some(&before), &after)
        .unwrap()
        .save(&db)
        .is_err());
    assert_eq!(app_state_load(&db).unwrap(), before);
    with_conn(&db, |conn| {
        conn.execute_batch("DROP TRIGGER reject_meta;")
            .map_err(|e| e.to_string())
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
    wallet_upsert(&db, &wallet("w1", "Bitcoin")).unwrap();
    wallet_upsert(&db, &wallet("w2", "Ethereum")).unwrap();

    let mut renamed = wallet("w1", "Bitcoin");
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

/// `history_fetch_for_wallet` and `history_fetch_for_chain` filter in
/// SQL, not by fetching every row and discarding most of them in Rust.
///
/// Both used to be implemented as `history_fetch_all(..).filter(..)` (the
/// chain-scoped one didn't exist at all — every caller fetched
/// everything). A wallet's or a chain's worth of rows is what a caller
/// asking for one should pay for.
#[test]
fn scoped_history_fetches_return_only_their_own_rows() {
    let db = tmp_db();
    history_upsert_batch(
        &db,
        &[
            history_record_on("btc-w1", "w1", "Bitcoin"),
            history_record_on("btc-w2", "w2", "Bitcoin"),
            history_record_on("eth-w1", "w1", "Ethereum"),
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

    let btc: Vec<String> = history_fetch_for_chain(&db, "Bitcoin")
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(
        std::collections::BTreeSet::from_iter(btc),
        std::collections::BTreeSet::from(["btc-w1".to_string(), "btc-w2".to_string()])
    );
}

#[test]
fn delete_wallet_data_removes_the_wallet_row_and_its_history() {
    let db = tmp_db();
    app_state_save(
        &db,
        &CoreAppState {
            wallets: vec![wallet("w1", "Bitcoin"), wallet("w2", "Bitcoin")],
            ..CoreAppState::default()
        },
    )
    .unwrap();
    history_upsert_batch(
        &db,
        &[history_record("tx1", "w1"), history_record("tx2", "w2")],
    )
    .unwrap();

    delete_wallet_data(&db, "w1").unwrap();

    assert!(wallet_load(&db, "w1").unwrap().is_none());
    assert!(wallet_load(&db, "w2").unwrap().is_some());
    let remaining: Vec<String> = history_fetch_all(&db)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(remaining, vec!["tx2"], "only w1's history should be gone");
}
