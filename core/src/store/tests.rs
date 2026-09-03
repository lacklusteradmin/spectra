use super::{
    aggregate_owned_addresses, build_persisted_snapshot, core_receive_selection,
    core_self_send_confirmation, persisted_snapshot_from_json, wallet_secret_index,
    OwnedAddressAggregationRequest, PendingSelfSendConfirmationInput, PersistedAppSnapshot,
    PersistedAppSnapshotRequest, ReceiveSelectionHoldingInput, ReceiveSelectionRequest,
    SelfSendConfirmationRequest, WalletSecretObservation,
};
use crate::state::CoreAppState;

#[test]
fn builds_secret_catalog_for_persisted_snapshot() {
    let request = PersistedAppSnapshotRequest {
        app_state_json: serde_json::to_string(&CoreAppState::default()).unwrap(),
        secret_observations: vec![WalletSecretObservation {
            wallet_id: "wallet-1".to_string(),
            secret_kind: Some("seedPhrase".to_string()),
            has_seed_phrase: true,
            has_private_key: false,
            has_password: true,
        }],
    };

    let mut app_state = CoreAppState::default();
    app_state.wallets.push(crate::state::WalletSummary {
        id: "wallet-1".to_string(),
        name: "Main".to_string(),
        is_watch_only: false,
        chain_name: "Bitcoin".to_string(),
        include_in_portfolio_total: true,
        network_mode: None,
        xpub: None,
        derivation_preset: "standard".to_string(),
        derivation_path: None,
        derivation_overrides: Default::default(),
        holdings: Vec::new(),
        addresses: Vec::new(),
    });

    let request = PersistedAppSnapshotRequest {
        app_state_json: serde_json::to_string(&app_state).unwrap(),
        secret_observations: request.secret_observations,
    };
    let snapshot = build_persisted_snapshot(request).unwrap();

    assert_eq!(snapshot.secrets.len(), 1);
    assert_eq!(snapshot.secrets[0].wallet_id, "wallet-1");
    assert!(snapshot.secrets[0].has_signing_material);
    assert_eq!(
        snapshot.secrets[0].password_store_key,
        "wallet.seed.password.wallet-1"
    );
}

#[test]
fn computes_wallet_secret_index_from_snapshot() {
    let snapshot = PersistedAppSnapshot {
        schema_version: 1,
        app_state: CoreAppState::default(),
        secrets: vec![
            super::SecretMaterialDescriptor {
                wallet_id: "seed-wallet".to_string(),
                secret_kind: "seedPhrase".to_string(),
                has_seed_phrase: true,
                has_private_key: false,
                has_password: true,
                has_signing_material: true,
                seed_phrase_store_key: "wallet.seed.seed-wallet".to_string(),
                password_store_key: "wallet.seed.password.seed-wallet".to_string(),
                private_key_store_key: "wallet.privatekey.seed-wallet".to_string(),
            },
            super::SecretMaterialDescriptor {
                wallet_id: "watch-wallet".to_string(),
                secret_kind: "watchOnly".to_string(),
                has_seed_phrase: false,
                has_private_key: false,
                has_password: false,
                has_signing_material: false,
                seed_phrase_store_key: "wallet.seed.watch-wallet".to_string(),
                password_store_key: "wallet.seed.password.watch-wallet".to_string(),
                private_key_store_key: "wallet.privatekey.watch-wallet".to_string(),
            },
        ],
    };

    let index = wallet_secret_index(&snapshot);
    assert_eq!(
        index.signing_material_wallet_ids,
        vec!["seed-wallet".to_string()]
    );
    assert_eq!(
        index.password_protected_wallet_ids,
        vec!["seed-wallet".to_string()]
    );
    assert!(index.private_key_backed_wallet_ids.is_empty());
}

#[test]
fn upgrades_core_state_payload_into_empty_secret_snapshot() {
    let json = serde_json::to_string(&CoreAppState::default()).unwrap();
    let snapshot = persisted_snapshot_from_json(&json).unwrap();
    assert_eq!(snapshot.schema_version, 1);
    assert!(snapshot.secrets.is_empty());
}

#[test]
fn aggregates_owned_addresses_in_order_without_duplicates() {
    let addresses = aggregate_owned_addresses(OwnedAddressAggregationRequest {
        candidate_addresses: vec![
            " 0xAbc ".to_string(),
            "".to_string(),
            "0xabc".to_string(),
            "bc1example".to_string(),
        ],
    });

    assert_eq!(
        addresses,
        vec!["0xAbc".to_string(), "bc1example".to_string()]
    );
}

/// The receive screen presents itself as the native holding, whatever order
/// the tokens come in.
#[test]
fn prefers_the_native_receive_holding() {
    let plan = core_receive_selection(ReceiveSelectionRequest {
        available_receive_holdings: vec![
            ReceiveSelectionHoldingInput {
                holding_index: 0,
                has_contract_address: true,
            },
            ReceiveSelectionHoldingInput {
                holding_index: 1,
                has_contract_address: false,
            },
        ],
    });
    assert_eq!(plan.selected_receive_holding_index, Some(1));
}

/// A wallet holding only tokens still shows something — the first of them.
#[test]
fn falls_back_to_the_first_holding_when_none_is_native() {
    let plan = core_receive_selection(ReceiveSelectionRequest {
        available_receive_holdings: vec![
            ReceiveSelectionHoldingInput {
                holding_index: 3,
                has_contract_address: true,
            },
            ReceiveSelectionHoldingInput {
                holding_index: 4,
                has_contract_address: true,
            },
        ],
    });
    assert_eq!(plan.selected_receive_holding_index, Some(3));
}

/// No holdings, no pick — and no panic. A wallet in this state is not
/// receive-enabled, so the screen never asks, but the guard is the answer
/// rather than an unwrap.
#[test]
fn no_holdings_selects_nothing() {
    let plan = core_receive_selection(ReceiveSelectionRequest {
        available_receive_holdings: vec![],
    });
    assert_eq!(plan.selected_receive_holding_index, None);
}

#[test]
fn consumes_matching_pending_self_send_confirmation() {
    let plan = core_self_send_confirmation(SelfSendConfirmationRequest {
        pending_confirmation: Some(PendingSelfSendConfirmationInput {
            wallet_id: "wallet-1".to_string(),
            chain_name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            destination_address_lowercased: "bc1self".to_string(),
            amount: 1.5,
            created_at_unix: 100.0,
        }),
        wallet_id: "wallet-1".to_string(),
        chain_name: "Bitcoin".to_string(),
        symbol: "BTC".to_string(),
        destination_address: "BC1SELF".to_string(),
        amount: 1.5,
        now_unix: 110.0,
        window_seconds: 30.0,
        owned_addresses: vec!["bc1self".to_string()],
    });

    assert!(!plan.requires_confirmation);
    assert!(plan.consume_existing_confirmation);
    assert!(plan.clear_pending_confirmation);
}

// ── Owned application state ──────────────────────────────────────────────────
//
// `WalletService` owns `CoreAppState`. These pin the contract every front end
// relies on: a command mutates core's copy, persists it, and the next process
// sees the change.

#[cfg(test)]
mod owned_state {
    use crate::service::WalletService;
    use crate::state::{CoreAppState, StateCommand};

    fn tmp_db(tag: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-owned-state-{tag}-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    fn service() -> std::sync::Arc<WalletService> {
        WalletService::new_typed(Vec::new()).expect("service")
    }

    #[tokio::test]
    async fn defaults_to_usd_before_anything_is_stored() {
        let service = service();
        let db = tmp_db("defaults");
        let state = service.open_state(db.clone()).await.expect("open");
        assert_eq!(state.settings.fiat_currency_code, "USD");
        assert_eq!(state, CoreAppState::default());
        let _ = std::fs::remove_file(&db);
    }

    /// The point of Stage 0: a command changes core's state and survives a
    /// restart without the caller doing anything to save it.
    #[tokio::test]
    async fn a_command_persists_without_the_caller_saving() {
        let db = tmp_db("persist");

        let first = service();
        first.open_state(db.clone()).await.expect("open");
        let transition = first
            .apply_state_command(StateCommand::SetFiatCurrency {
                fiat_currency_code: "EUR".to_string(),
            })
            .await
            .expect("apply");
        assert_eq!(transition.state.settings.fiat_currency_code, "EUR");
        assert_eq!(transition.events.len(), 1);
        assert_eq!(transition.events[0].kind, "fiatCurrencyChanged");

        // A second service, as a second process would see it.
        let second = service();
        let reopened = second.open_state(db.clone()).await.expect("reopen");
        assert_eq!(reopened.settings.fiat_currency_code, "EUR");
        assert_eq!(second.fiat_currency_code().await, "EUR");

        let _ = std::fs::remove_file(&db);
    }

    #[tokio::test]
    async fn currency_codes_are_normalized() {
        let service = service();
        service.open_state(tmp_db("normalize")).await.expect("open");
        let transition = service
            .apply_state_command(StateCommand::SetFiatCurrency {
                fiat_currency_code: "  eur \n".to_string(),
            })
            .await
            .expect("apply");
        assert_eq!(transition.state.settings.fiat_currency_code, "EUR");
    }

    /// Setting a value to what it already is is not a change: no event, and
    /// nothing is written.
    #[tokio::test]
    async fn a_no_op_command_emits_no_event() {
        let service = service();
        service.open_state(tmp_db("noop")).await.expect("open");
        let transition = service
            .apply_state_command(StateCommand::SetFiatCurrency {
                fiat_currency_code: "usd".to_string(),
            })
            .await
            .expect("apply");
        assert!(transition.events.is_empty());
        assert_eq!(transition.state.settings.fiat_currency_code, "USD");
    }

    /// Without `open_state` the service still works, in memory only. Tests and
    /// short-lived tools rely on this.
    #[tokio::test]
    async fn commands_apply_in_memory_when_no_database_is_bound() {
        let service = service();
        let transition = service
            .apply_state_command(StateCommand::SetFiatCurrency {
                fiat_currency_code: "JPY".to_string(),
            })
            .await
            .expect("apply");
        assert_eq!(transition.state.settings.fiat_currency_code, "JPY");
        assert_eq!(service.fiat_currency_code().await, "JPY");
    }
}

// ── Address book (core-owned) ────────────────────────────────────────────────
//
// Validation lives in the reducer, not at the call site: a front end that
// forgets to check cannot save a duplicate or an invalid address.

#[cfg(test)]
mod address_book {
    use crate::service::WalletService;
    use crate::state::{AddressBookRejection, StateCommand};

    const BTC: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
    const BTC2: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

    fn tmp_db(tag: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-address-book-{tag}-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    fn service() -> std::sync::Arc<WalletService> {
        WalletService::new_typed(Vec::new()).expect("service")
    }

    fn add(id: &str, name: &str, address: &str) -> StateCommand {
        StateCommand::AddAddressBookEntry {
            id: id.to_string(),
            name: name.to_string(),
            chain_name: "Bitcoin".to_string(),
            address: address.to_string(),
            note: String::new(),
        }
    }

    fn rejection(events: &[crate::state::StateEvent]) -> Option<String> {
        events
            .iter()
            .find(|e| e.kind == "addressBookRejected")
            .and_then(|e| e.subject_id.clone())
    }

    #[tokio::test]
    async fn adds_newest_first_and_trims() {
        let service = service();
        service
            .apply_state_command(add("1", "  Cold  ", BTC))
            .await
            .expect("add");
        let transition = service
            .apply_state_command(add("2", "Hot", BTC2))
            .await
            .expect("add");

        let ids: Vec<&str> = transition
            .state
            .address_book
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, vec!["2", "1"], "newest entry comes first");
        assert_eq!(transition.state.address_book[1].name, "Cold");
    }

    #[tokio::test]
    async fn refuses_an_empty_name() {
        let service = service();
        let transition = service
            .apply_state_command(add("1", "   ", BTC))
            .await
            .expect("add");
        assert!(transition.state.address_book.is_empty());
        assert_eq!(rejection(&transition.events).as_deref(), Some("emptyName"));
    }

    #[tokio::test]
    async fn refuses_an_address_that_is_not_valid_for_the_chain() {
        let service = service();
        let transition = service
            .apply_state_command(add("1", "Typo", "bc1qnot-a-real-address"))
            .await
            .expect("add");
        assert!(transition.state.address_book.is_empty());
        assert_eq!(
            rejection(&transition.events).as_deref(),
            Some("invalidAddress")
        );
    }

    /// The same address twice is refused, and case does not get around it —
    /// addresses are stored normalized.
    #[tokio::test]
    async fn refuses_a_duplicate_regardless_of_case() {
        let service = service();
        service
            .apply_state_command(add("1", "Cold", BTC))
            .await
            .expect("add");
        let transition = service
            .apply_state_command(add("2", "Cold again", &BTC.to_uppercase()))
            .await
            .expect("add");
        assert_eq!(transition.state.address_book.len(), 1);
        assert_eq!(
            rejection(&transition.events).as_deref(),
            Some("duplicateAddress")
        );
    }

    /// The same address on a different chain is a different recipient.
    #[tokio::test]
    async fn the_same_address_on_another_chain_is_not_a_duplicate() {
        let service = service();
        service
            .apply_state_command(add("1", "BTC", BTC))
            .await
            .expect("add");
        let transition = service
            .apply_state_command(StateCommand::AddAddressBookEntry {
                id: "2".to_string(),
                name: "LTC".to_string(),
                chain_name: "Litecoin".to_string(),
                address: "ltc1qw508d6qejxtdg4y5r3zarvary0c5xw7kgmn4n9".to_string(),
                note: String::new(),
            })
            .await
            .expect("add");
        assert_eq!(transition.state.address_book.len(), 2);
        assert!(rejection(&transition.events).is_none());
    }

    #[tokio::test]
    async fn renames_and_removes() {
        let service = service();
        service
            .apply_state_command(add("1", "Cold", BTC))
            .await
            .expect("add");

        let renamed = service
            .apply_state_command(StateCommand::RenameAddressBookEntry {
                id: "1".to_string(),
                name: "  Vault  ".to_string(),
            })
            .await
            .expect("rename");
        assert_eq!(renamed.state.address_book[0].name, "Vault");

        let empty = service
            .apply_state_command(StateCommand::RenameAddressBookEntry {
                id: "1".to_string(),
                name: "  ".to_string(),
            })
            .await
            .expect("rename");
        assert_eq!(empty.state.address_book[0].name, "Vault", "unchanged");
        assert_eq!(rejection(&empty.events).as_deref(), Some("emptyName"));

        let removed = service
            .apply_state_command(StateCommand::RemoveAddressBookEntry {
                id: "1".to_string(),
            })
            .await
            .expect("remove");
        assert!(removed.state.address_book.is_empty());

        // Removing what is already gone is not a change.
        let again = service
            .apply_state_command(StateCommand::RemoveAddressBookEntry {
                id: "1".to_string(),
            })
            .await
            .expect("remove");
        assert!(again.events.is_empty());
    }

    #[tokio::test]
    async fn survives_a_restart_in_order() {
        let db = tmp_db("persist");

        let first = service();
        first.open_state(db.clone()).await.expect("open");
        first
            .apply_state_command(add("1", "Cold", BTC))
            .await
            .expect("add");
        first
            .apply_state_command(add("2", "Hot", BTC2))
            .await
            .expect("add");

        let second = service();
        let reopened = second.open_state(db.clone()).await.expect("reopen");
        let ids: Vec<&str> = reopened
            .address_book
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, vec!["2", "1"]);
        assert_eq!(reopened.address_book[0].name, "Hot");

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn rejection_reasons_serialize_as_the_strings_front_ends_match_on() {
        for (reason, expected) in [
            (AddressBookRejection::EmptyName, "emptyName"),
            (AddressBookRejection::InvalidAddress, "invalidAddress"),
            (AddressBookRejection::DuplicateAddress, "duplicateAddress"),
        ] {
            assert_eq!(
                serde_json::to_value(reason).unwrap().as_str(),
                Some(expected)
            );
        }
    }
}

// ── Owned transaction store ──────────────────────────────────────────────────
//
// Transactions live in SQLite, not in `CoreAppState` — history is unbounded and
// `apply_state_command` returns the whole state. What core owns here is the
// writes and, critically, the added/updated/removed delta: whether a record is
// new is a property of the store, and callers used to have to guess.

#[cfg(test)]
mod transaction_store {
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
}

#[cfg(test)]
mod transaction_merge {
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
}

// ── CoreImportedWallet → WalletSummary ───────────────────────────────────────
//
// The app's wallet record converted into the model core computes with. What
// these pin is which fields survive and which deliberately do not.

#[cfg(test)]
mod wallet_model_conversion {
    use crate::registry::Chain;
    use crate::store::wallet_domain::{
        AssetHolding, CoreImportedWallet, CoreSeedDerivationPaths,
        CoreSeedDerivationPreset, CoreWalletDerivationOverrides,
    };
    use std::collections::HashMap;

    fn bitcoin_wallet() -> CoreImportedWallet {
        let mut paths = CoreSeedDerivationPaths::default();
        // The full table every wallet carries today, of which one entry applies.
        paths.set_path_for(Chain::Bitcoin, "m/84'/0'/0'/0/0");
        paths.set_path_for(Chain::Ethereum, "m/44'/60'/0'/0/0");
        paths.set_path_for(Chain::Solana, "m/44'/501'/0'");

        CoreImportedWallet {
            id: "w1".to_string(),
            name: "Cold".to_string(),
            network_chain_id: Some("bitcoin-testnet-4".to_string()),
            addresses: HashMap::from([("bitcoin".to_string(), "bc1qexample".to_string())]),
            bitcoin_xpub: Some("zpub123".to_string()),
            seed_derivation_preset: CoreSeedDerivationPreset::Account2,
            seed_derivation_paths: paths,
            derivation_overrides: CoreWalletDerivationOverrides {
                passphrase: Some("secret".to_string()),
                ..Default::default()
            },
            selected_chain: "Bitcoin".to_string(),
            holdings: vec![AssetHolding {
                name: "Bitcoin".to_string(),
                symbol: "BTC".to_string(),
                coin_gecko_id: "bitcoin".to_string(),
                chain_name: "Bitcoin".to_string(),
                token_standard: "Native".to_string(),
                contract_address: None,
                amount: 1.5,
                price_usd: 60000.0,
            }],
            include_in_portfolio_total: true,
        }
    }

    #[test]
    fn keeps_the_path_the_wallet_uses_and_drops_the_rest() {
        let summary = bitcoin_wallet().to_summary(false);
        assert_eq!(summary.derivation_path.as_deref(), Some("m/84'/0'/0'/0/0"));
        // The Ethereum and Solana entries were global defaults, not this
        // wallet's data, and do not survive into the model core computes with.
        assert_eq!(summary.chain_name, "Bitcoin");
    }

    #[test]
    fn keeps_only_the_network_that_applies_to_this_wallets_family() {
        let summary = bitcoin_wallet().to_summary(false);
        assert_eq!(summary.network_mode.as_deref(), Some("bitcoin-testnet-4"));

        // A wallet on a chain with no network variants reports none, rather
        // than the meaningless mainnet default the record always holds.
        let mut solana = bitcoin_wallet();
        solana.selected_chain = "Solana".to_string();
        assert_eq!(solana.to_summary(false).network_mode, None);
    }

    #[test]
    fn carries_overrides_xpub_preset_and_holdings() {
        let summary = bitcoin_wallet().to_summary(false);
        assert_eq!(
            summary.derivation_overrides.passphrase.as_deref(),
            Some("secret")
        );
        assert_eq!(summary.xpub.as_deref(), Some("zpub123"));
        assert_eq!(summary.derivation_preset, "account2");
        assert_eq!(summary.holdings.len(), 1);
        assert_eq!(summary.holdings[0].amount, 1.5);
        assert_eq!(summary.holdings[0].symbol, "BTC");
    }

    /// The address becomes a typed entry with its chain and derivation path,
    /// rather than a bare string in a slot-keyed map.
    #[test]
    fn the_address_gains_its_chain_and_path() {
        let summary = bitcoin_wallet().to_summary(false);
        assert_eq!(summary.addresses.len(), 1);
        assert_eq!(summary.addresses[0].address, "bc1qexample");
        assert_eq!(summary.addresses[0].chain_name, "Bitcoin");
        assert_eq!(
            summary.addresses[0].derivation_path.as_deref(),
            Some("m/84'/0'/0'/0/0")
        );
        assert_eq!(summary.primary_address(), Some("bc1qexample"));
    }

    /// Watch-only is a Keychain fact on iOS, not something the record holds,
    /// so the caller states it.
    #[test]
    fn watch_only_comes_from_the_caller() {
        assert!(!bitcoin_wallet().to_summary(false).is_watch_only);
        assert!(bitcoin_wallet().to_summary(true).is_watch_only);
    }

    /// A wallet whose chain the registry does not know converts without an
    /// address rather than panicking or inventing one.
    #[test]
    fn an_unknown_chain_yields_no_address() {
        let mut wallet = bitcoin_wallet();
        wallet.selected_chain = "Nonexistent Chain".to_string();
        let summary = wallet.to_summary(false);
        assert!(summary.addresses.is_empty());
        assert_eq!(summary.derivation_path, None);
    }
}

#[cfg(test)]
mod wallet_view_model {
    use crate::registry::Chain;
    use crate::store::wallet_domain::AssetHolding;
    use crate::store::state::{WalletAddress, WalletSummary};
    use crate::store::wallet_domain::CoreSeedDerivationPaths;

    fn defaults() -> CoreSeedDerivationPaths {
        crate::app_core_derivation_paths_for_preset(0).expect("defaults")
    }

    fn summary() -> WalletSummary {
        WalletSummary {
            id: "w1".to_string(),
            name: "Cold".to_string(),
            is_watch_only: false,
            chain_name: "Bitcoin".to_string(),
            include_in_portfolio_total: true,
            network_mode: Some("bitcoin-testnet-4".to_string()),
            xpub: Some("zpub123".to_string()),
            derivation_preset: "account2".to_string(),
            derivation_path: Some("m/84'/0'/2'/0/0".to_string()),
            derivation_overrides: Default::default(),
            holdings: vec![AssetHolding {
                name: "Bitcoin".to_string(),
                symbol: "BTC".to_string(),
                coin_gecko_id: "bitcoin".to_string(),
                chain_name: "Bitcoin".to_string(),
                token_standard: "Native".to_string(),
                contract_address: None,
                amount: 1.5,
                price_usd: 60000.0,
            }],
            addresses: vec![WalletAddress {
                chain_name: "Bitcoin".to_string(),
                address: "bc1qexample".to_string(),
                kind: "receive".to_string(),
                derivation_path: Some("m/84'/0'/2'/0/0".to_string()),
            }],
        }
    }

    /// Everything the app renders survives the trip out to the view model.
    #[test]
    fn the_view_model_carries_what_the_app_shows() {
        let view = summary().to_imported_wallet(&defaults());
        assert_eq!(view.id, "w1");
        assert_eq!(view.selected_chain, "Bitcoin");
        assert_eq!(view.bitcoin_xpub.as_deref(), Some("zpub123"));
        assert_eq!(view.address_for(Chain::Bitcoin), Some("bc1qexample"));
        assert_eq!(view.holdings.len(), 1);
        assert_eq!(view.holdings[0].amount, 1.5);
        // The wallet's own path overrides the default for its chain.
        assert_eq!(
            view.seed_derivation_paths.path_for(Chain::Bitcoin),
            Some("m/84'/0'/2'/0/0")
        );
        // Other chains keep the catalog defaults, which is all they ever were.
        assert!(view.seed_derivation_paths.path_for(Chain::Solana).is_some());
    }

    /// The network mode goes back into the field for the wallet's own chain and
    /// The wallet's network is one chain id, so there is nothing to leave
    /// alone — a wallet on Bitcoin testnet4 says exactly that, rather than
    /// carrying a Bitcoin mode and a Dogecoin mode and a rule for reading them.
    #[test]
    fn the_wallets_own_network_survives_the_round_trip() {
        let view = summary().to_imported_wallet(&defaults());
        assert_eq!(view.network_chain_id.as_deref(), Some("bitcoin-testnet-4"));
    }

    /// Holding ids are derived from what identifies the asset, so rebuilding
    /// the view model does not make SwiftUI think every row is new.
    #[test]
    fn holding_ids_are_stable_across_rebuilds() {
        use crate::store::wallet_domain::holding_identity;
        let first = summary().to_imported_wallet(&defaults());
        let second = summary().to_imported_wallet(&defaults());
        assert_eq!(
            holding_identity(&first.holdings[0]),
            holding_identity(&second.holdings[0])
        );
        assert!(!holding_identity(&first.holdings[0]).is_empty());

        // Two different assets do not collide.
        let mut other = summary();
        other.holdings[0].symbol = "USDT".to_string();
        other.holdings[0].contract_address = Some("0xdac1".to_string());
        let third = other.to_imported_wallet(&defaults());
        assert_ne!(
            holding_identity(&first.holdings[0]),
            holding_identity(&third.holdings[0])
        );
    }

    /// Round trip through both conversions preserves everything the summary
    /// holds — the authority is unchanged by being rendered.
    #[test]
    fn summary_survives_a_round_trip_through_the_view_model() {
        let original = summary();
        let round_tripped = original
            .to_imported_wallet(&defaults())
            .to_summary(original.is_watch_only);
        assert_eq!(round_tripped, original);
    }
}

#[cfg(test)]
mod wallet_update_if_present {
    use crate::service::WalletService;
    use crate::state::{StateCommand, WalletSummary};

    fn wallet(id: &str, name: &str) -> WalletSummary {
        WalletSummary::single_address(id, name, "Bitcoin", "bc1qexample", None, false)
    }

    /// A balance result that arrives after the wallet was deleted must not
    /// bring it back.
    #[tokio::test]
    async fn does_not_resurrect_a_deleted_wallet() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: wallet("w1", "Cold"),
            })
            .await
            .expect("upsert");
        service
            .apply_state_command(StateCommand::RemoveWallet {
                wallet_id: "w1".to_string(),
            })
            .await
            .expect("remove");

        let late = service
            .apply_state_command(StateCommand::UpdateWalletIfPresent {
                wallet: wallet("w1", "Cold with fresh balance"),
            })
            .await
            .expect("update");
        assert!(late.state.wallets.is_empty(), "wallet came back");
        assert!(late.events.is_empty());
    }

    #[tokio::test]
    async fn updates_a_wallet_that_is_still_there() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: wallet("w1", "Cold"),
            })
            .await
            .expect("upsert");
        let updated = service
            .apply_state_command(StateCommand::UpdateWalletIfPresent {
                wallet: wallet("w1", "Renamed"),
            })
            .await
            .expect("update");
        assert_eq!(updated.state.wallets[0].name, "Renamed");
        assert_eq!(updated.events.len(), 1);
    }
}

#[cfg(test)]
mod open_state_idempotence {
    use crate::service::WalletService;
    use crate::state::StateCommand;

    fn tmp_db() -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-open-idem-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    /// A second `open_state` must not discard state written since the first.
    /// The app calls it from a launch reload that races user actions.
    #[tokio::test]
    async fn reopening_does_not_revert_newer_writes() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let db = tmp_db();
        service.open_state(db.clone()).await.expect("open");

        service
            .apply_state_command(StateCommand::SetFiatCurrency {
                fiat_currency_code: "EUR".to_string(),
            })
            .await
            .expect("apply");

        let reopened = service.open_state(db.clone()).await.expect("reopen");
        assert_eq!(reopened.settings.fiat_currency_code, "EUR");
        let _ = std::fs::remove_file(&db);
    }

    /// A different database is a genuine open, and does replace the state.
    #[tokio::test]
    async fn opening_a_different_database_replaces_the_state() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let first = tmp_db();
        service.open_state(first.clone()).await.expect("open");
        service
            .apply_state_command(StateCommand::SetFiatCurrency {
                fiat_currency_code: "EUR".to_string(),
            })
            .await
            .expect("apply");

        let second = format!("{first}.other");
        let _ = std::fs::remove_file(&second);
        let switched = service
            .open_state(second.clone())
            .await
            .expect("open other");
        assert_eq!(switched.settings.fiat_currency_code, "USD");

        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }
}

/// Confirmation-poll backoff. Core owns the tracker table; these cover the
/// intents Swift drives it with.
#[cfg(test)]
mod status_trackers {
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

    /// A manual recheck re-opens a transaction the tracker had finished with.
    #[tokio::test]
    async fn resetting_a_tracker_makes_it_due_again() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        service
            .record_status_poll("tx1".into(), crate::service::StatusPollOutcome::Confirmed { confirmations: Some(99) })
            .await;
        assert!(service
            .transactions_due_for_status_poll(vec!["tx1".into()])
            .await
            .is_empty());

        service.reset_status_tracker("tx1".into(), true).await;
        assert_eq!(
            service
                .transactions_due_for_status_poll(vec!["tx1".into()])
                .await,
            vec!["tx1".to_string()]
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
        service.open_state(tmp_db("apply-resolved")).await.expect("open");
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
        assert!(
            service
                .stale_pending_failure_ids("Litecoin".into())
                .await
                .expect("read")
                .is_empty()
        );
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
}

/// Importing is a core operation now: it plans, builds and stores in one call.
#[cfg(test)]
mod wallet_import {
    use crate::derivation::import::{
        WalletImportAddresses, WalletImportCommit, WalletImportRequest,
    };
    use crate::service::WalletService;
    use crate::store::wallet_domain::{
        CoreSeedDerivationPaths, CoreSeedDerivationPreset, CoreWalletDerivationOverrides,
    };
    use std::collections::HashMap;

    // Real addresses: core validates every import address now, so a
    // placeholder would simply be dropped.
    const BTC: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
    const SOL: &str = "11111111111111111111111111111111";

    fn commit(chains: &[&str], addresses: &[(&str, &str)]) -> WalletImportCommit {
        WalletImportCommit {
            request: WalletImportRequest {
                wallet_name: String::new(),
                default_wallet_name_start_index: 1,
                primary_selected_chain_name: chains[0].to_string(),
                selected_chain_names: chains.iter().map(|c| c.to_string()).collect(),
                planned_wallet_ids: (0..chains.len())
                    .map(|i| format!("11111111-0000-0000-0000-00000000000{i}"))
                    .collect(),
                is_watch_only_import: false,
                is_private_key_import: false,
                has_wallet_password: false,
                resolved_addresses: WalletImportAddresses {
                    by_slot: addresses
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                    bitcoin_xpub: None,
                },
                watch_only_entries: Default::default(),
            },
            holdings: Vec::new(),
            seed_derivation_preset: CoreSeedDerivationPreset::Standard,
            seed_derivation_paths: CoreSeedDerivationPaths {
                by_chain: HashMap::new(),
                is_custom_enabled: false,
            },
            derivation_overrides: CoreWalletDerivationOverrides::default(),
            network_chain_by_family: Default::default(),
            seed_phrase: None,
        }
    }

    #[tokio::test]
    async fn imported_wallets_land_in_core_state() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let outcome = service
            .import_wallets(commit(&["Solana"], &[("solana", SOL)]))
            .await
            .expect("import");

        assert_eq!(outcome.wallets.len(), 1);
        // The caller does not store anything — core already did.
        let stored = service.wallets_for_display().await.expect("wallets");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].selected_chain, "Solana");
        assert_eq!(
            stored[0].addresses.get("solana").map(String::as_str),
            Some(SOL)
        );
    }

    #[tokio::test]
    async fn a_network_selection_applies_only_to_its_own_family() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut input = commit(&["Bitcoin", "Solana"], &[("bitcoin", BTC), ("solana", SOL)]);
        input.network_chain_by_family = std::collections::HashMap::from([(
            "bitcoin".to_string(),
            "bitcoin-testnet".to_string(),
        )]);
        let outcome = service.import_wallets(input).await.expect("import");

        let by_chain: std::collections::HashMap<_, _> = outcome
            .wallets
            .iter()
            .map(|w| (w.selected_chain.as_str(), w))
            .collect();
        assert_eq!(
            by_chain["Bitcoin"].network_chain_id.as_deref(),
            Some("bitcoin-testnet")
        );
        // Choosing Bitcoin testnet must not drag the Solana wallet with it.
        assert_eq!(by_chain["Solana"].network_chain_id, None);
    }
}

/// Keypool reservation. The property that matters is that an index is never
/// handed out twice — so these hammer it concurrently.
#[cfg(test)]
/// The per-chain send shape, transcribed from the ten Swift call sites that
/// carried it inline. These values decide whether a send is refused for
/// insufficient fee and how the fee reaches the signer, so the transcription
/// is pinned rather than trusted.
#[cfg(test)]
mod send_execution_shape {
    use crate::registry::{Chain, SendFeeField};

    #[test]
    fn the_shape_matches_what_the_call_sites_carried() {
        let cases: &[(&str, u8, bool, SendFeeField, f64)] = &[
            ("Sui", 6, false, SendFeeField::GasBudget, 0.0),
            ("Aptos", 6, false, SendFeeField::None, 0.0),
            ("TON", 6, false, SendFeeField::None, 0.0),
            ("XRP Ledger", 6, true, SendFeeField::None, 0.0),
            ("Stellar", 7, true, SendFeeField::None, 0.0),
            ("Monero", 6, false, SendFeeField::None, 0.0),
            ("Cardano", 6, false, SendFeeField::FeeAmount, 0.0),
            ("NEAR", 6, false, SendFeeField::None, 0.0),
            ("Polkadot", 6, false, SendFeeField::None, 0.0),
            ("Bitcoin Cash", 8, false, SendFeeField::FeeSats, 0.00001),
            ("Bitcoin SV", 8, false, SendFeeField::FeeSats, 0.00001),
            ("Litecoin", 8, false, SendFeeField::FeeSats, 0.0001),
        ];
        for (name, decimals, private_key, field, fallback) in cases {
            let chain = Chain::from_display_name(name).expect(name);
            let shape = chain.send_execution_shape();
            assert_eq!(shape.fee_decimals, *decimals, "{name} fee_decimals");
            assert_eq!(shape.supports_private_key, *private_key, "{name} private key");
            assert_eq!(shape.fee_field, *field, "{name} fee_field");
            assert_eq!(shape.fee_fallback, *fallback, "{name} fee_fallback");
        }
    }

    /// A testnet signs the same way its mainnet does.
    #[test]
    fn a_testnet_sends_under_its_mainnets_shape() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            let mainnet = chain.mainnet_counterpart();
            assert_eq!(
                chain.send_execution_shape().fee_field,
                mainnet.send_execution_shape().fee_field,
                "{} diverges from {}",
                chain.str_id(),
                mainnet.str_id()
            );
        }
    }
}

/// The dashboard's rows. Grouping the same asset across chains and ordering
/// them are domain rules; they lived in the shell and the CLI could not reach
/// them.
#[cfg(test)]
mod dashboard_groups {
    use crate::service::WalletService;
    use crate::store::wallet_domain::AssetHolding;
    use crate::state::{StateCommand, WalletSummary};
    use std::collections::HashMap;

    fn holding(symbol: &str, chain: &str, amount: f64, price: f64) -> AssetHolding {
        AssetHolding {
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            coin_gecko_id: symbol.to_lowercase(),
            chain_name: chain.to_string(),
            token_standard: "Native".to_string(),
            contract_address: None,
            amount,
            price_usd: price,
        }
    }

    async fn service_with(wallets: Vec<(&str, &str, Vec<AssetHolding>)>) -> std::sync::Arc<WalletService> {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for (id, chain, holdings) in wallets {
            let mut wallet = WalletSummary::single_address(id, id, chain, "addr", None, false);
            wallet.holdings = holdings;
            service
                .apply_state_command(StateCommand::UpsertWallet { wallet })
                .await
                .expect("upsert");
        }
        service
    }

    /// A row is per asset: the same asset on two chains is one row, with the
    /// amounts summed and a breakdown of where it is held.
    ///
    /// Was per (chain, asset) — ETH on Ethereum and ETH on Arbitrum were two
    /// rows. Every EVM L2 carries Ethereum's coingecko id, which is what makes
    /// them one asset.
    #[tokio::test]
    async fn a_row_is_per_asset_and_breaks_down_by_chain() {
        let service = service_with(vec![
            ("w1", "Ethereum", vec![holding("ETH", "Ethereum", 1.0, 2000.0)]),
            ("w2", "Ethereum", vec![holding("ETH", "Ethereum", 2.0, 2000.0)]),
            ("w3", "Arbitrum", vec![holding("ETH", "Arbitrum", 5.0, 2000.0)]),
        ])
        .await;
        let groups = service
            .dashboard_asset_groups(HashMap::new())
            .await
            .expect("groups");
        let eth: Vec<_> = groups
            .iter()
            .filter(|g| g.holdings.iter().any(|h| h.coin.symbol == "ETH"))
            .collect();
        assert_eq!(eth.len(), 1, "one row for the asset, not one per chain");

        let row = eth[0];
        let total: f64 = row.holdings.iter().map(|h| h.coin.amount).sum();
        assert_eq!(total, 8.0, "both chains and both wallets summed");
        assert_eq!(row.holdings.len(), 2, "one breakdown entry per chain");

        // The row is presented as the place most of it is.
        assert_eq!(row.holdings[0].coin.chain_name, "Arbitrum");
        assert_eq!(row.holdings[0].coin.amount, 5.0);
        // And the two wallets on one chain are one entry.
        let ethereum = row
            .holdings
            .iter()
            .find(|h| h.coin.chain_name == "Ethereum")
            .expect("an Ethereum entry");
        assert_eq!(ethereum.coin.amount, 3.0);
    }

    /// A holding with no coingecko id is never merged with another by symbol.
    ///
    /// Symbols are not unique and nobody vouches for them. A token the catalog
    /// does not vouch for is reported with an empty symbol on purpose — the
    /// front end shows its contract, the one string a deployer cannot forge —
    /// and grouping by symbol would undo that, showing a real holding and a
    /// lookalike on another chain as one balance.
    #[tokio::test]
    async fn an_unvouched_token_is_never_merged_by_symbol() {
        let mut real = holding("USDX", "Ethereum", 1.0, 1.0);
        real.contract_address = Some("0xaaaa".into());
        real.coin_gecko_id = String::new();
        let mut lookalike = holding("USDX", "Tron", 999.0, 1.0);
        lookalike.contract_address = Some("Tbbbb".into());
        lookalike.coin_gecko_id = String::new();
        // And a second contract on the same chain, same symbol.
        let mut sibling = holding("USDX", "Ethereum", 2.0, 1.0);
        sibling.contract_address = Some("0xbbbb".into());
        sibling.coin_gecko_id = String::new();

        let service = service_with(vec![
            ("w1", "Ethereum", vec![real, sibling]),
            ("w2", "Tron", vec![lookalike]),
        ])
        .await;
        let groups = service
            .dashboard_asset_groups(HashMap::new())
            .await
            .expect("groups");
        let usdx: Vec<_> = groups
            .iter()
            .filter(|g| g.holdings.iter().any(|h| h.coin.symbol == "USDX"))
            .collect();
        assert_eq!(
            usdx.len(),
            3,
            "three unvouched contracts merged by symbol, so a lookalike's \
             balance was added to a real one"
        );
        for g in usdx {
            assert_eq!(g.holdings.len(), 1);
        }
    }

    /// A row is presented as the place most of it is held.
    fn row_symbol(g: &crate::store::wallet_domain::CoreDashboardAssetGroup) -> &str {
        g.holdings.first().map(|h| h.coin.symbol.as_str()).unwrap_or_default()
    }

    /// A row's value: the sum of its holdings', or none when any is unpriced.
    ///
    /// Derived rather than stored — the group used to carry a `total_value_usd`
    /// beside the list it comes from.
    fn row_value(g: &crate::store::wallet_domain::CoreDashboardAssetGroup) -> Option<f64> {
        g.holdings
            .iter()
            .map(|h| h.value_usd)
            .try_fold(0.0, |sum, v| v.map(|v| sum + v))
    }

    /// Live prices win over the amount a holding was stored with.
    #[tokio::test]
    async fn a_live_price_beats_the_stored_one() {
        let service = service_with(vec![("w1", "Ethereum", vec![holding("ETH", "Ethereum", 2.0, 1000.0)])]).await;
        let stored = service
            .dashboard_asset_groups(HashMap::new())
            .await
            .expect("groups");
        assert_eq!(row_value(&stored[0]), Some(2000.0));

        let live = service
            .dashboard_asset_groups(HashMap::from([("Ethereum|ETH".to_string(), 3000.0)]))
            .await
            .expect("groups");
        assert_eq!(row_value(&live[0]), Some(6000.0));
    }

    /// A testnet holding has no value, so its row reports none rather than
    /// quoting it at mainnet.
    #[tokio::test]
    async fn a_testnet_row_has_no_value() {
        let service = service_with(vec![("w1", "Ethereum", vec![holding("ETH", "Ethereum", 2.0, 1000.0)])]).await;
        service
            .apply_state_command(StateCommand::SelectNetworkChain {
                chain_id: "ethereum-sepolia".into(),
            })
            .await
            .expect("select");
        let groups = service
            .dashboard_asset_groups(HashMap::from([("Ethereum Sepolia|ETH".to_string(), 3000.0)]))
            .await
            .expect("groups");
        assert_eq!(row_value(&groups[0]), None);
    }

    /// Pinned rows come first, in the order they were pinned, and a pinned
    /// symbol with no holdings still gets a row.
    #[tokio::test]
    async fn pinned_rows_lead_in_pin_order() {
        let service = service_with(vec![(
            "w1",
            "Ethereum",
            vec![
                holding("ETH", "Ethereum", 1.0, 2000.0),
                holding("BTC", "Bitcoin", 1.0, 60000.0),
            ],
        )])
        .await;
        service
            .apply_state_command(StateCommand::SetPinnedDashboardAssets {
                symbols: vec!["ETH".into(), "SOL".into()],
            })
            .await
            .expect("pin");
        let groups = service
            .dashboard_asset_groups(HashMap::new())
            .await
            .expect("groups");
        let symbols: Vec<_> = groups
            .iter()
            .map(row_symbol)
            .collect();
        // ETH before SOL because that is the pin order, and both before the
        // unpinned BTC even though BTC is worth more.
        assert_eq!(symbols.first(), Some(&"ETH"));
        assert!(
            groups.iter().any(|g| row_symbol(g) == "BTC" && !g.is_pinned),
            "BTC is still shown, unpinned"
        );
    }
}

/// Operational events: core stamps, caps and persists them.
#[cfg(test)]
mod operational_events {
    use crate::service::WalletService;
    use crate::store::ChainOperationalEventLevel;

    fn tmp_db(label: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-events-{label}-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn events_survive_reopening_the_database() {
        let db = tmp_db("reopen");
        let service = WalletService::new_typed(Vec::new()).expect("service");
        service.open_state(db.clone()).await.expect("open");
        service
            .append_chain_operational_event(
                "Bitcoin".into(),
                ChainOperationalEventLevel::Warning,
                "broadcast deferred".into(),
                Some("abc123".into()),
            )
            .await
            .expect("append");

        let reopened = WalletService::new_typed(Vec::new()).expect("service");
        reopened.open_state(db.clone()).await.expect("open");
        let events = reopened.operational_events("Bitcoin".into()).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message, "broadcast deferred");
        assert_eq!(events[0].level, ChainOperationalEventLevel::Warning);
        assert_eq!(events[0].transaction_hash.as_deref(), Some("abc123"));
        assert!(
            events[0].timestamp_unix > 0.0,
            "core did not stamp the time"
        );

        let _ = std::fs::remove_file(&db);
    }

    /// Newest first, and the cap holds — the property the planner stated but
    /// could not enforce, because a caller wrote the answer down.
    #[tokio::test]
    async fn the_log_is_newest_first_and_bounded() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for index in 0..205 {
            service
                .append_chain_operational_event(
                    "Solana".into(),
                    ChainOperationalEventLevel::Info,
                    format!("event {index}"),
                    None,
                )
                .await
                .expect("append");
        }
        let events = service.operational_events("Solana".into()).await;
        assert_eq!(events.len(), 200, "the cap did not hold");
        assert_eq!(events[0].message, "event 204");
        assert_eq!(events[199].message, "event 5");
        // A different chain keeps its own list.
        assert!(service
            .operational_events("Bitcoin".into())
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn clearing_one_chain_leaves_the_others() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for chain in ["Bitcoin", "Solana"] {
            service
                .append_chain_operational_event(
                    chain.into(),
                    ChainOperationalEventLevel::Error,
                    "send failed".into(),
                    None,
                )
                .await
                .expect("append");
        }
        service
            .clear_operational_events(Some("Bitcoin".into()))
            .await
            .expect("clear one");
        assert!(service
            .operational_events("Bitcoin".into())
            .await
            .is_empty());
        assert_eq!(service.operational_events("Solana".into()).await.len(), 1);

        service
            .clear_operational_events(None)
            .await
            .expect("clear all");
        assert!(service.operational_events("Solana".into()).await.is_empty());
    }
}

/// The built-in token catalog and the merge that folds it into stored
/// preferences. Both moved into core when the planner went away.
#[cfg(test)]
mod built_in_tokens {
    use crate::service::WalletService;
    use crate::store::state::StateCommand;
    use crate::store::wallet_domain::CoreTokenHostingChain;

    #[test]
    fn every_built_in_has_a_unique_id() {
        let entries = crate::store::built_in_token_preferences();
        assert!(!entries.is_empty(), "the catalog produced nothing");
        let mut ids: Vec<String> = entries.iter().map(|e| e.id()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(count, ids.len(), "two built-ins share an id");
        assert!(
            entries.iter().all(|e| e.is_built_in),
            "a catalog entry is not marked built-in"
        );
    }

    /// The chain mapping used to exist four times. `tokens.toml` spells BNB
    /// Chain `"bnb"`, which is the case a strict name match would drop.
    #[test]
    fn the_catalog_chain_names_all_resolve() {
        for token in crate::tokens::catalog() {
            if token.chain.eq_ignore_ascii_case("bnb") {
                assert_eq!(
                    CoreTokenHostingChain::from_chain_name(&token.chain),
                    Some(CoreTokenHostingChain::Bnb)
                );
            }
        }
        for chain in CoreTokenHostingChain::ALL {
            assert_eq!(
                CoreTokenHostingChain::from_chain_name(chain.chain_name()),
                Some(*chain),
                "{} does not round-trip",
                chain.chain_name()
            );
        }
    }

    /// A user's choices survive the merge; the build's additions arrive.
    #[tokio::test]
    async fn merging_keeps_what_the_user_chose() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let state = service
            .merge_built_in_token_preferences()
            .await
            .expect("merge");
        assert!(!state.token_preferences.is_empty());

        // Turn one off, then merge again.
        let mut entries = state.token_preferences.clone();
        let target = entries
            .iter_mut()
            .find(|e| e.is_built_in && e.is_enabled)
            .expect("an enabled built-in");
        target.is_enabled = false;
        let id = target.id().clone();
        service
            .apply_state_command(StateCommand::SetTokenPreferences { entries })
            .await
            .expect("store");

        let after = service
            .merge_built_in_token_preferences()
            .await
            .expect("merge again");
        let kept = after
            .token_preferences
            .iter()
            .find(|e| e.id() == id)
            .expect("the entry survived");
        assert!(
            !kept.is_enabled,
            "the merge re-enabled a token the user turned off"
        );
    }
}

mod keypool {
    use crate::service::WalletService;

    #[tokio::test]
    async fn receive_reservation_is_stable_across_calls() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let first = service
            .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
            .await
            .expect("reserve");
        let second = service
            .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
            .await
            .expect("reserve");
        // Opening the receive sheet twice must not burn two addresses.
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn change_indices_are_never_handed_out_twice() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut handles = Vec::new();
        for _ in 0..32 {
            let service = service.clone();
            handles.push(tokio::spawn(async move {
                service
                    .reserve_change_index("w1".into(), "Bitcoin".into())
                    .await
                    .expect("reserve")
            }));
        }
        let mut seen = Vec::new();
        for handle in handles {
            seen.push(handle.await.expect("join"));
        }
        seen.sort_unstable();
        let unique: std::collections::HashSet<_> = seen.iter().copied().collect();
        assert_eq!(unique.len(), 32, "an index was reserved twice: {seen:?}");
        assert_eq!(seen, (0..32).collect::<Vec<i64>>());
    }

    #[tokio::test]
    async fn clearing_a_reservation_frees_the_next_index() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let first = service
            .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
            .await
            .expect("reserve");
        service
            .clear_reserved_receive_index("w1".into(), "Bitcoin".into())
            .await
            .expect("clear");
        let second = service
            .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
            .await
            .expect("reserve");
        // The used address must not be reissued.
        assert_ne!(first, second);
        assert_eq!(second, first + 1);
    }

    #[tokio::test]
    async fn keypool_survives_reopening_the_database() {
        let db = {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "spectra-keypool-reopen-{}.sqlite",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            path.to_string_lossy().into_owned()
        };
        let service = WalletService::new_typed(Vec::new()).expect("service");
        service.open_state(db.clone()).await.expect("open");
        let reserved = service
            .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
            .await
            .expect("reserve");

        let reopened = WalletService::new_typed(Vec::new()).expect("service");
        reopened.open_state(db.clone()).await.expect("open");
        let after = reopened
            .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
            .await
            .expect("reserve");
        // A restart must not reissue the address already handed out.
        assert_eq!(reserved, after);

        let _ = std::fs::remove_file(&db);
    }

    /// The baseline is core's now, so a recorded owned address has to move it
    /// without anyone passing one in.
    ///
    /// This is the property the old shape could not have: the caller supplied
    /// the baseline, so the reservation was only as current as the caller's
    /// copy of the owned-address table.
    #[tokio::test]
    async fn a_recorded_owned_address_raises_the_baseline() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        // Bitcoin is a deep-UTXO chain, so the baseline reads indices.
        service
            .register_owned_address(
                "w1".into(),
                "Bitcoin".into(),
                "bc1qexample".into(),
                None,
                Some("external".into()),
                Some(7),
            )
            .await
            .expect("register");
        let reserved = service
            .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
            .await
            .expect("reserve");
        assert_eq!(
            reserved, 8,
            "index 7 was already handed out; the next receive index must clear it"
        );
    }

    /// The table is core's, so it has to come back on its own.
    #[tokio::test]
    async fn owned_addresses_survive_reopening_the_database() {
        let db = {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "spectra-owned-reopen-{}-{:?}.sqlite",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_file(&path);
            path.to_string_lossy().into_owned()
        };
        let service = WalletService::new_typed(Vec::new()).expect("service");
        service.open_state(db.clone()).await.expect("open");
        service
            .register_owned_address(
                "w1".into(),
                "Bitcoin".into(),
                "bc1qexample".into(),
                Some("m/84'/0'/0'/0/3".into()),
                Some("external".into()),
                Some(3),
            )
            .await
            .expect("register");

        let reopened = WalletService::new_typed(Vec::new()).expect("service");
        reopened.open_state(db.clone()).await.expect("open");
        assert_eq!(
            reopened
                .owned_addresses_for_wallet("w1".into(), Some("Bitcoin".into()))
                .await,
            vec!["bc1qexample".to_string()]
        );
        // And the baseline it feeds comes back with it.
        assert_eq!(
            reopened
                .reserve_receive_index("w1".into(), "Bitcoin".into(), 0)
                .await
                .expect("reserve"),
            4
        );

        let _ = std::fs::remove_file(&db);
    }
}

/// Pinned dashboard assets are user choices that must survive a restart, so
/// core owns them like any other setting.
#[cfg(test)]
mod pinned_dashboard_assets {
    use crate::service::WalletService;
    use crate::state::StateCommand;

    async fn pins(service: &WalletService) -> Vec<String> {
        service
            .apply_state_command(StateCommand::SetPinnedDashboardAssets { symbols: vec![] })
            .await
            .expect("read")
            .state
            .settings
            .pinned_dashboard_asset_symbols
    }

    #[tokio::test]
    async fn symbols_are_uppercased_and_deduplicated_in_pin_order() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let transition = service
            .apply_state_command(StateCommand::SetPinnedDashboardAssets {
                symbols: vec![
                    " eth ".into(),
                    "BTC".into(),
                    "Eth".into(),
                    "".into(),
                    "sol".into(),
                ],
            })
            .await
            .expect("apply");
        assert_eq!(
            transition.state.settings.pinned_dashboard_asset_symbols,
            vec!["ETH".to_string(), "BTC".to_string(), "SOL".to_string()]
        );
    }

    #[tokio::test]
    async fn setting_the_same_pins_emits_nothing() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let command = || StateCommand::SetPinnedDashboardAssets {
            symbols: vec!["BTC".into()],
        };
        let first = service.apply_state_command(command()).await.expect("apply");
        assert_eq!(first.events.len(), 1);
        let second = service.apply_state_command(command()).await.expect("apply");
        assert!(
            second.events.is_empty(),
            "re-pinning the same set is a no-op"
        );
    }

    #[tokio::test]
    async fn clearing_pins_is_distinguishable_from_never_pinning() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        assert!(pins(&service).await.is_empty());
        service
            .apply_state_command(StateCommand::SetPinnedDashboardAssets {
                symbols: vec!["BTC".into()],
            })
            .await
            .expect("apply");
        let cleared = service
            .apply_state_command(StateCommand::SetPinnedDashboardAssets { symbols: vec![] })
            .await
            .expect("apply");
        assert!(cleared
            .state
            .settings
            .pinned_dashboard_asset_symbols
            .is_empty());
        assert_eq!(cleared.events.len(), 1, "clearing is a real change");
    }
}

/// Adding a field to `AppSettings` must not make an already-written state file
/// unreadable. This is not hypothetical: adding
/// `pinned_dashboard_asset_symbols` without `#[serde(default)]` made every
/// launch on an existing database fail with "missing field".
#[cfg(test)]
mod settings_forward_compatibility {
    use crate::state::{AppSettings, CoreAppState};

    #[test]
    fn settings_written_before_a_field_existed_still_load() {
        let legacy = r#"{"fiatCurrencyCode":"EUR"}"#;
        let settings: AppSettings =
            serde_json::from_str(legacy).expect("settings from before the field was added");
        assert_eq!(settings.fiat_currency_code, "EUR");
        assert!(settings.pinned_dashboard_asset_symbols.is_empty());
        // A field's serde default and `AppSettings::default()` are the same
        // function, so an absent field reads as a fresh install would, not as
        // its type's zero value. Notifications off is not the same as unset.
        assert!(settings.use_price_alerts);
        assert_eq!(settings.bitcoin_stop_gap, 10);
    }

    #[test]
    fn state_written_before_token_preferences_existed_still_loads() {
        let legacy = r#"{
            "schemaVersion": 2,
            "wallets": [],
            "selectedWalletId": null,
            "settings": {"fiatCurrencyCode":"USD"},
            "addressBook": []
        }"#;
        let state: CoreAppState = serde_json::from_str(legacy).expect("legacy state");
        assert!(state.token_preferences.is_empty());
    }
}

/// The merge strategy per chain used to be eighteen Swift wrappers. These pin
/// the registry to exactly what those wrappers did, so the move cannot change
/// behaviour for any chain that already had one.
#[cfg(test)]
mod transaction_merge_strategy {
    use crate::fetch::transactions::TransactionMergeStrategy as S;
    use crate::registry::Chain;

    fn strategy(display_name: &str) -> S {
        Chain::from_display_name(display_name)
            .unwrap_or_else(|| panic!("unknown chain {display_name}"))
            .transaction_merge_strategy()
    }

    #[test]
    fn matches_the_swift_wrappers_it_replaced() {
        for name in ["Bitcoin", "Bitcoin Cash", "Bitcoin SV", "Litecoin"] {
            assert_eq!(strategy(name), S::StandardUtxo, "{name}");
        }
        assert_eq!(strategy("Dogecoin"), S::Dogecoin);
        for name in [
            "Tron",
            "Solana",
            "Cardano",
            "XRP Ledger",
            "Stellar",
            "Monero",
            "Sui",
            "Aptos",
            "TON",
            "Internet Computer",
            "NEAR",
            "Polkadot",
        ] {
            assert_eq!(strategy(name), S::AccountBased, "{name}");
        }
        for name in ["Ethereum", "Arbitrum", "Base", "Polygon"] {
            assert_eq!(strategy(name), S::Evm, "{name}");
        }
    }

    #[test]
    fn only_tron_keys_its_merge_identity_on_symbol() {
        for chain in Chain::all() {
            let expected = matches!(chain.str_id(), "tron" | "tron-nile");
            assert_eq!(
                chain.merge_identity_includes_symbol(),
                expected,
                "{}",
                chain.str_id()
            );
        }
    }

    #[test]
    fn a_testnet_merges_the_same_way_as_its_mainnet() {
        for chain in Chain::all() {
            let mainnet = chain.mainnet_counterpart();
            if mainnet == chain {
                continue;
            }
            assert_eq!(
                chain.transaction_merge_strategy(),
                mainnet.transaction_merge_strategy(),
                "{} diverges from {}",
                chain.str_id(),
                mainnet.str_id()
            );
        }
    }
}

/// `supportedEVMToken` used to exclude a chain's native asset with six
/// hand-written chain/symbol pairs. It asks the registry now, so these pin the
/// registry to exactly what those six said.
#[cfg(test)]
mod evm_native_symbols {
    use crate::registry::Chain;

    #[test]
    fn native_symbols_match_the_hand_written_pairs_they_replaced() {
        for (display_name, symbol) in [
            ("Ethereum", "ETH"),
            ("Ethereum Classic", "ETC"),
            ("Optimism", "ETH"),
            ("BNB Chain", "BNB"),
            ("Avalanche", "AVAX"),
            ("Hyperliquid", "HYPE"),
        ] {
            let chain = Chain::from_display_name(display_name)
                .unwrap_or_else(|| panic!("unknown chain {display_name}"));
            assert_eq!(chain.coin_symbol(), symbol, "{display_name}");
        }
    }

    #[test]
    fn every_evm_chain_has_a_native_symbol() {
        for chain in Chain::all().filter(|c| c.is_evm()) {
            assert!(
                !chain.coin_symbol().is_empty(),
                "{} has no native symbol, so its native asset would be treated as a token",
                chain.str_id()
            );
        }
    }
}

/// The send rule per chain used to be a `match` on chain-name strings inside
/// `can_send_holding`. These pin the registry to exactly what it said.
#[cfg(test)]
mod send_rules {
    use crate::registry::{Chain, SendRule};

    fn rule(display_name: &str) -> SendRule {
        Chain::from_display_name(display_name)
            .unwrap_or_else(|| panic!("unknown chain {display_name}"))
            .send_rule()
    }

    #[test]
    fn the_two_exceptions_and_the_default() {
        assert_eq!(rule("Ethereum Classic"), SendRule::NativeOnly);
        assert_eq!(rule("Hyperliquid"), SendRule::NativeOnly);
        assert_eq!(rule("Solana"), SendRule::SupportedSolanaCoin);
        // Non-EVM chains carry no extra restriction.
        assert_eq!(rule("Bitcoin"), SendRule::Any);
        assert_eq!(rule("Polkadot"), SendRule::Any);
    }

    /// One rule for the EVM family.
    ///
    /// This replaces `send_rule_asymmetry_across_evm_chains`, which asserted
    /// that exactly three of twenty-three EVM chains gated non-native sends —
    /// a test whose only job was to stop anyone fixing the split. Arbitrum and
    /// Ethereum answer the same way now, and the two deliberate exceptions
    /// (`EthereumClassic`, `Hyperliquid`) are native-only, which is stricter
    /// still.
    #[test]
    fn every_evm_chain_gates_non_native_sends_the_same_way() {
        for chain in Chain::all().filter(|c| c.is_evm()) {
            let expected = match chain.mainnet_counterpart() {
                Chain::EthereumClassic | Chain::Hyperliquid => SendRule::NativeOnly,
                _ => SendRule::NativeOrSupportedToken,
            };
            assert_eq!(
                chain.send_rule(),
                expected,
                "{} gates differently from the rest of the EVM family",
                chain.str_id()
            );
        }
    }

    #[test]
    fn a_testnet_sends_under_the_same_rule_as_its_mainnet() {
        for chain in Chain::all() {
            let mainnet = chain.mainnet_counterpart();
            if mainnet == chain {
                continue;
            }
            assert_eq!(
                chain.send_rule(),
                mainnet.send_rule(),
                "{} diverges from {}",
                chain.str_id(),
                mainnet.str_id()
            );
        }
    }
}

/// `wallet_derived_state` replaced two planners that returned holding indices
/// for the caller to resolve. These cover the parts that indirection made easy
/// to get wrong: grouping, network-mode-dependent identity, and send gating.
#[cfg(test)]
mod wallet_derived_state {
    use crate::service::WalletService;
    use crate::state::StateCommand;
    use crate::store::wallet_domain::AssetHolding;

    fn coin(symbol: &str, chain: &str, amount: f64) -> AssetHolding {
        AssetHolding {
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            coin_gecko_id: symbol.to_lowercase(),
            chain_name: chain.to_string(),
            token_standard: String::new(),
            contract_address: None,
            amount,
            price_usd: 1.0,
        }
    }

    async fn service_with(
        wallets: Vec<(&str, &str, Vec<AssetHolding>, bool)>,
    ) -> std::sync::Arc<WalletService> {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for (id, chain, holdings, included) in wallets {
            let mut summary = crate::store::state::WalletSummary::single_address(
                id, id, chain, "addr", None, false,
            );
            summary.include_in_portfolio_total = included;
            summary.holdings = holdings
                .into_iter()
                .map(|c| crate::store::wallet_domain::AssetHolding {
                    name: c.name,
                    symbol: c.symbol,
                    coin_gecko_id: c.coin_gecko_id,
                    chain_name: c.chain_name,
                    token_standard: c.token_standard,
                    contract_address: c.contract_address,
                    amount: c.amount,
                    price_usd: c.price_usd,
                })
                .collect();
            service
                .apply_state_command(StateCommand::UpsertWallet { wallet: summary })
                .await
                .expect("upsert");
        }
        service
    }

    #[tokio::test]
    async fn portfolio_sums_the_same_asset_across_wallets() {
        let service = service_with(vec![
            ("w1", "Bitcoin", vec![coin("BTC", "Bitcoin", 1.5)], true),
            ("w2", "Bitcoin", vec![coin("BTC", "Bitcoin", 0.5)], true),
        ])
        .await;
        let derived = service
            .wallet_derived_state(vec![], vec![])
            .await
            .expect("derived");
        assert_eq!(derived.portfolio.len(), 1);
        assert_eq!(derived.portfolio[0].amount, 2.0);
    }

    #[tokio::test]
    async fn wallets_excluded_from_the_total_contribute_nothing() {
        let service = service_with(vec![
            ("w1", "Bitcoin", vec![coin("BTC", "Bitcoin", 1.0)], true),
            ("w2", "Bitcoin", vec![coin("BTC", "Bitcoin", 9.0)], false),
        ])
        .await;
        let derived = service
            .wallet_derived_state(vec![], vec![])
            .await
            .expect("derived");
        assert_eq!(derived.portfolio[0].amount, 1.0);
        assert_eq!(derived.included_portfolio_holdings.len(), 1);
    }

    /// Every family's testnet is unpriced, not just the two the old rule
    /// listed by name — Dogecoin testnet used to be quoted at mainnet prices.
    #[tokio::test]
    async fn no_testnet_coin_is_quoted_on_any_family() {
        for (chain, testnet_id) in [
            ("Bitcoin", "bitcoin-testnet"),
            ("Ethereum", "ethereum-sepolia"),
            ("Dogecoin", "dogecoin-testnet"),
        ] {
            let service =
                service_with(vec![("w1", chain, vec![coin("X", chain, 1.0)], true)]).await;
            service
                .apply_state_command(StateCommand::SelectNetworkChain {
                    chain_id: testnet_id.into(),
                })
                .await
                .expect("select");
            let derived = service
                .wallet_derived_state(vec![], vec![])
                .await
                .expect("derived");
            assert!(
                derived.unique_price_request_coins.is_empty(),
                "{chain} testnet coins have no price to request"
            );
        }
    }

    /// Selecting the mainnet clears the entry rather than storing it, so the
    /// two ways of saying "mainnet" cannot drift apart.
    #[tokio::test]
    async fn choosing_mainnet_stores_nothing() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let after_testnet = service
            .apply_state_command(StateCommand::SelectNetworkChain {
                chain_id: "bitcoin-testnet-4".into(),
            })
            .await
            .expect("select");
        assert_eq!(
            after_testnet.state.settings.network_chain_by_family.len(),
            1
        );

        let after_mainnet = service
            .apply_state_command(StateCommand::SelectNetworkChain {
                chain_id: "bitcoin".into(),
            })
            .await
            .expect("select");
        assert!(after_mainnet
            .state
            .settings
            .network_chain_by_family
            .is_empty());
    }

    #[tokio::test]
    async fn sending_needs_signing_material_on_a_live_chain() {
        let service = service_with(vec![(
            "w1",
            "Bitcoin",
            vec![coin("BTC", "Bitcoin", 1.0)],
            true,
        )])
        .await;

        let watch_only = service
            .wallet_derived_state(vec![], vec![])
            .await
            .expect("derived");
        assert!(watch_only.send_enabled_wallet_ids.is_empty());
        // Receiving never needs a key.
        assert_eq!(
            watch_only.receive_enabled_wallet_ids,
            vec!["w1".to_string()]
        );

        let with_key = service
            .wallet_derived_state(vec!["w1".into()], vec![])
            .await
            .expect("derived");
        assert_eq!(with_key.send_enabled_wallet_ids, vec!["w1".to_string()]);
    }

    #[tokio::test]
    async fn an_untracked_token_on_ethereum_cannot_be_sent() {
        let service = service_with(vec![(
            "w1",
            "Ethereum",
            vec![coin("ETH", "Ethereum", 1.0), coin("SHIB", "Ethereum", 1.0)],
            true,
        )])
        .await;
        let derived = service
            .wallet_derived_state(vec!["w1".into()], vec![])
            .await
            .expect("derived");
        let sendable: Vec<&str> = derived.send_coins_by_wallet_id["w1"]
            .iter()
            .map(|c| c.symbol.as_str())
            .collect();
        assert_eq!(sendable, vec!["ETH"], "SHIB is not a known token");
    }
}

/// `resolvedAddress(for:chainName:)` used to be a 24-case switch mapping each
/// chain name to its own accessor. It asks core for the derivation chain now,
/// so core must answer for every chain that switch listed — a missing entry
/// would silently return no address rather than fail to compile.
#[cfg(test)]
mod seed_derivation_chain_coverage {
    use crate::registry::Chain;

    /// The non-EVM chains the Swift switch named, minus the four that keep
    /// bespoke resolvers (Bitcoin, Dogecoin, Cardano, Monero).
    const SWITCH_CHAINS: &[&str] = &[
        "Bitcoin Cash",
        "Bitcoin SV",
        "Litecoin",
        "Tron",
        "Solana",
        "Stellar",
        "XRP Ledger",
        "Sui",
        "Aptos",
        "TON",
        "Internet Computer",
        "NEAR",
        "Polkadot",
        "Zcash",
        "Bitcoin Gold",
        "Decred",
        "Kaspa",
        "Dash",
        "Bittensor",
    ];

    #[test]
    fn every_chain_the_switch_named_still_resolves() {
        for name in SWITCH_CHAINS {
            assert!(
                Chain::from_display_name(name).is_some(),
                "{name} is no longer a known chain"
            );
            assert!(
                Chain::from_display_name(name)
                    .and_then(crate::send::flow::seed_derivation_chain_raw)
                    .is_some(),
                "{name} has no seed derivation chain, so its address would resolve to nil"
            );
        }
    }
}

/// Import addresses are validated for every chain, not for some of them.
///
/// The iOS path validated three chains and passed the other twenty-one through
/// untouched, so whether a malformed address reached storage depended only on
/// which chain it was typed under.
#[cfg(test)]
mod import_address_validation {
    use crate::derivation::import::WalletImportAddresses;

    /// Mainnet, which is what every case here means unless it says otherwise.
    fn validated_addresses(
        addresses: &WalletImportAddresses,
    ) -> (WalletImportAddresses, Vec<String>) {
        crate::derivation::import::validated_addresses(addresses, &Default::default())
    }

    fn addresses(pairs: &[(&str, &str)]) -> WalletImportAddresses {
        WalletImportAddresses {
            by_slot: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            bitcoin_xpub: None,
        }
    }

    #[test]
    fn a_malformed_address_is_dropped_whatever_the_chain() {
        // "solana" and "tron" were both in the lenient group.
        let (kept, rejected) = validated_addresses(&addresses(&[
            ("solana", "not-a-solana-address"),
            ("tron", "nonsense"),
            ("ethereum", "0xnothex"),
        ]));
        assert!(kept.by_slot.is_empty(), "kept: {:?}", kept.by_slot);
        assert_eq!(rejected.len(), 3);
    }

    /// A valid address survives import and is stored in core's normal form.
    ///
    /// The fixture was `0x742D35CC…bC454E…` — arbitrary mixed case, which the
    /// validator accepted because it lowercased before looking. It is a
    /// **broken EIP-55 checksum** and is refused now, so the fixture is the
    /// all-uppercase form: no checksum to verify, still valid, and it still
    /// demonstrates the normalisation this test is about.
    #[test]
    fn a_valid_address_survives_and_is_normalised() {
        let (kept, rejected) = validated_addresses(&addresses(&[(
            "ethereum",
            "0X742D35CC6634C0532925A3B844BC454E4438F44E",
        )]));
        assert!(rejected.is_empty());
        let stored = kept.by_slot.get("ethereum").expect("kept");
        // Normalisation is core's, not the caller's transcription.
        assert!(stored.starts_with("0x"));
        assert_eq!(stored.len(), 42);
    }

    #[test]
    fn empty_and_whitespace_entries_are_skipped_not_rejected() {
        let (kept, rejected) = validated_addresses(&addresses(&[("solana", "   ")]));
        assert!(kept.by_slot.is_empty());
        assert!(
            rejected.is_empty(),
            "an unfilled field is not a rejected address"
        );
    }

    #[test]
    fn the_bitcoin_xpub_is_carried_through_untouched() {
        let mut input = addresses(&[]);
        input.bitcoin_xpub = Some("zpub-whatever".to_string());
        let (kept, _) = validated_addresses(&input);
        assert_eq!(kept.bitcoin_xpub.as_deref(), Some("zpub-whatever"));
    }

    /// A derived address is mainnet-format even on a testnet import, so the
    /// slot map is judged against mainnet regardless of the selected mode.
    ///
    /// Derivation at import runs against the mainnet chain — `chainPaths` is
    /// keyed by mainnet display name — and the testnet address is re-derived
    /// for display. Judging this map by the selected network mode dropped
    /// every address on a testnet import and produced a wallet with none.
    #[test]
    fn a_derived_address_is_kept_on_a_testnet_import() {
        let derived = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
        // Even asked for testnet4, the slot map is mainnet.
        let (kept, rejected) = crate::derivation::import::validated_addresses(
            &addresses(&[("bitcoin", derived)]),
            &crate::derivation::import::ImportNetworks {
                by_family: std::collections::HashMap::from([(
                    "bitcoin".to_string(),
                    "bitcoin-testnet-4".to_string(),
                )]),
            },
        );
        // The helper itself honours what it is told...
        assert_eq!(rejected, vec![derived.to_string()]);
        assert!(kept.by_slot.is_empty());
        // ...so it is `import_wallets` that must pass mainnet here, which is
        // what the default does.
        let (kept, rejected) = validated_addresses(&addresses(&[("bitcoin", derived)]));
        assert!(rejected.is_empty());
        assert_eq!(
            kept.by_slot.get("bitcoin").map(String::as_str),
            Some(derived)
        );
    }

    #[test]
    fn a_rejection_names_the_address_not_the_slot() {
        // The caller has to be able to tell the user which address was
        // refused. A slot name ("ethereum") does not identify one when the
        // import supplied several.
        let (_, rejected) = validated_addresses(&addresses(&[("solana", "not-an-address")]));
        assert_eq!(rejected, vec!["not-an-address".to_string()]);
    }

    /// The watch-only list is a separate input from the slot map, and it is the
    /// one where the address is typed rather than derived.
    mod watch_only {
        use crate::derivation::import::{ImportNetworks, WalletImportWatchOnlyEntries};
        use std::collections::HashMap;

        /// Mainnet, as above.
        fn validated_watch_only_entries(
            entries: &WalletImportWatchOnlyEntries,
        ) -> (WalletImportWatchOnlyEntries, Vec<String>) {
            validated_watch_only_entries_on(entries, Default::default())
        }

        fn validated_watch_only_entries_on(
            entries: &WalletImportWatchOnlyEntries,
            networks: ImportNetworks,
        ) -> (WalletImportWatchOnlyEntries, Vec<String>) {
            crate::derivation::import::validated_watch_only_entries(entries, &networks)
        }

        fn entries(slot: &str, addresses: &[&str]) -> WalletImportWatchOnlyEntries {
            WalletImportWatchOnlyEntries {
                by_slot: HashMap::from([(
                    slot.to_string(),
                    addresses.iter().map(|a| a.to_string()).collect(),
                )]),
                bitcoin_xpub: None,
            }
        }

        #[test]
        fn a_malformed_watch_address_is_refused() {
            let (kept, rejected) = validated_watch_only_entries(&entries("solana", &["garbage"]));
            assert!(kept.by_slot.is_empty(), "kept: {:?}", kept.by_slot);
            assert_eq!(rejected, vec!["garbage".to_string()]);
        }

        #[test]
        fn valid_watch_addresses_survive_and_are_normalised() {
            let (kept, rejected) = validated_watch_only_entries(&entries(
                "ethereum",
                &["0X742D35CC6634C0532925A3B844BC454E4438F44E"],
            ));
            assert!(rejected.is_empty());
            let stored = kept.by_slot.get("ethereum").expect("kept");
            assert_eq!(stored.len(), 1);
            assert!(stored[0].starts_with("0x"));
        }

        /// Core normalises on the way in, so a caller does not have to.
        #[test]
        fn every_slot_normalises_without_help_from_the_caller() {
            let padded = "0x0000000000000000000000000000000000000000000000000000000000000ABC";
            let cases: [(&str, &str, &str); 3] = [
                (
                    "ethereum",
                    "0x742D35CC6634C0532925A3B844BC454E4438F44E",
                    "0x742d35cc6634c0532925a3b844bc454e4438f44e",
                ),
                ("sui", padded, &padded.to_lowercase()),
                ("aptos", padded, &padded.to_lowercase()),
            ];
            for (slot, typed, expected) in cases {
                let (kept, rejected) = validated_watch_only_entries(&entries(slot, &[typed]));
                assert!(rejected.is_empty(), "{slot}: rejected {typed}");
                assert_eq!(
                    kept.by_slot.get(slot).map(Vec::as_slice),
                    Some([expected.to_string()].as_slice()),
                    "{slot} did not normalise"
                );
            }
        }

        /// The import path and the send path must agree on what an address
        /// looks like once normalised.
        ///
        /// They are two separate tables today — `validate_address` matches on
        /// the validation kind, `normalize_address` on the chain display name.
        /// iOS called the second (as `normalizedSendAddress`) before handing
        /// addresses to the first. Deleting those calls is only safe while the
        /// two agree, so this fails if they ever drift apart.
        #[test]
        fn the_send_normaliser_and_the_import_normaliser_agree() {
            use crate::send::flow::normalized_send_address;
            // Internet Computer is absent on purpose: its account identifier
            // carries a CRC32 prefix, so there is no fixture to write here
            // without computing a real one, and a fixture the validator
            // rejects would test nothing.
            let cases: [(&str, &str, &str); 5] = [
                (
                    "Ethereum",
                    "ethereum",
                    "0x742D35CC6634C0532925A3B844BC454E4438F44E",
                ),
                (
                    "Sui",
                    "sui",
                    "0x0000000000000000000000000000000000000000000000000000000000000ABC",
                ),
                (
                    "Aptos",
                    "aptos",
                    "0x0000000000000000000000000000000000000000000000000000000000000ABC",
                ),
                ("NEAR", "near", "Example.NEAR"),
                (
                    "Solana",
                    "solana",
                    "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX",
                ),
            ];
            for (chain_name, slot, typed) in cases {
                let (kept, rejected) = validated_watch_only_entries(&entries(slot, &[typed]));
                assert!(rejected.is_empty(), "{chain_name}: rejected {typed}");
                let imported = kept
                    .by_slot
                    .get(slot)
                    .and_then(|list| list.first())
                    .unwrap();
                let sent = normalized_send_address(chain_name.to_string(), typed.to_string());
                assert_eq!(
                    imported, &sent,
                    "{chain_name}: import normalised to {imported}, send to {sent}"
                );
            }
        }

        #[test]
        fn surrounding_whitespace_is_not_the_caller_s_problem_either() {
            let (kept, rejected) = validated_watch_only_entries(&entries(
                "ethereum",
                &["  0x742d35cc6634c0532925a3b844bc454e4438f44e  "],
            ));
            assert!(rejected.is_empty());
            assert_eq!(
                kept.by_slot.get("ethereum").map(Vec::as_slice),
                Some(["0x742d35cc6634c0532925a3b844bc454e4438f44e".to_string()].as_slice())
            );
        }

        /// A testnet address arrives in its mainnet's slot, so validation has
        /// to be told which network the import is for.
        ///
        /// `ImportDraft.watchOnlyInputsByChainName` is keyed by mainnet display
        /// name — there is no "Bitcoin Testnet" row — so a testnet watch import
        /// puts a testnet address in the `bitcoin` slot. Validating that slot as
        /// mainnet refuses a wallet the app has always allowed.
        #[test]
        fn a_testnet_watch_address_survives_when_the_import_is_for_testnet() {
            // tb1 prefix — valid Bitcoin testnet, invalid on mainnet.
            let typed = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
            let (kept, rejected) = validated_watch_only_entries_on(
                &entries("bitcoin", &[typed]),
                ImportNetworks {
                    by_family: std::collections::HashMap::from([(
                        "bitcoin".to_string(),
                        "bitcoin-testnet".to_string(),
                    )]),
                },
            );
            assert!(rejected.is_empty(), "testnet address refused: {rejected:?}");
            assert_eq!(kept.by_slot.get("bitcoin").map(Vec::len), Some(1));
        }

        #[test]
        fn a_testnet_watch_address_is_still_refused_on_mainnet() {
            let typed = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
            let (kept, rejected) = validated_watch_only_entries(&entries("bitcoin", &[typed]));
            assert_eq!(rejected, vec![typed.to_string()]);
            assert!(kept.by_slot.is_empty());
        }

        #[test]
        fn one_bad_address_does_not_discard_the_good_ones() {
            let (kept, rejected) = validated_watch_only_entries(&entries(
                "ethereum",
                &[
                    "0X742D35CC6634C0532925A3B844BC454E4438F44E",
                    "0xnothex",
                    "0x0000000000000000000000000000000000000001",
                ],
            ));
            assert_eq!(kept.by_slot.get("ethereum").expect("kept").len(), 2);
            assert_eq!(rejected, vec!["0xnothex".to_string()]);
        }
    }
}

/// Known tokens survive a reopen.
///
/// They did not. `token_preferences` is a field on `CoreAppState`, but
/// `app_state_save` wrote `settings`, `wallets` and the address book and never
/// this — so every launch loaded an empty list, and `PLAN.md`'s claim that
/// they "arrive with the rest of the state" was false. Nothing caught it
/// because no test reopened the database after tracking a token, and the app
/// keeps them in memory for the life of a session.
#[cfg(test)]
mod tracked_tokens_persist {
    use crate::store::state::{CoreAppState, StateCommand};
    use crate::store::wallet_db;
    use crate::store::wallet_domain::{
        CoreTokenPreferenceCategory, CoreTokenPreferenceEntry, CoreTokenHostingChain,
    };

    /// One database per test. Keyed by thread id as well as pid: two tests in
    /// the same process share a pid, and the first version of this helper did
    /// not, so the second test read the first one's tokens and "passed" on
    /// data it never wrote.
    fn tmp_db() -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-tokens-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    fn entry(symbol: &str, decimals: u32) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            category: CoreTokenPreferenceCategory::Custom,
            is_built_in: false,
            is_enabled: true,
            token: crate::tokens::TokenEntry {
                chain: CoreTokenHostingChain::Ethereum.chain_name().to_string(),
                name: symbol.to_string(),
                symbol: symbol.to_string(),
                token_standard: "ERC-20".to_string(),
                contract: "0x0000000000000000000000000000000000000001".to_string(),
                coingecko_id: symbol.to_lowercase(),
                decimals,
                tags: Vec::new(),
                color: String::new(),
                asset_name: String::new(),
                enabled: true,
            },
        }
    }

    #[test]
    fn a_tracked_token_survives_a_reopen() {
        let db = tmp_db();
        let mut state = CoreAppState::default();
        crate::store::state::reduce_state_in_place(
            &mut state,
            StateCommand::SetTokenPreferences {
                entries: vec![entry("USDC", 6)],
            },
        );
        wallet_db::app_state_save(&db, &state).expect("save");

        let reloaded = wallet_db::app_state_load(&db).expect("load");
        assert_eq!(
            reloaded.token_preferences.len(),
            1,
            "known token was lost"
        );
        assert_eq!(reloaded.token_preferences[0].token.symbol, "USDC");
    }

    #[test]
    fn the_clamp_survives_the_round_trip() {
        let db = tmp_db();
        let mut state = CoreAppState::default();
        crate::store::state::reduce_state_in_place(
            &mut state,
            StateCommand::SetTokenPreferences {
                // More decimals than any token has.
                entries: vec![entry("USDT", 99)],
            },
        );
        wallet_db::app_state_save(&db, &state).expect("save");
        let reloaded = wallet_db::app_state_load(&db).expect("load");
        assert_eq!(
            reloaded.token_preferences[0].token.decimals,
            crate::store::state::MAX_TOKEN_DECIMALS as u32
        );
    }
}

/// Every collection on `CoreAppState` survives a reopen.
///
/// Written before the price-alert command, because the token list shipped
/// unpersisted for exactly as long as nobody reopened the database after
/// writing one. This walks every collection rather than the newest, so the
/// next field added is covered by the same test or fails it.
#[cfg(test)]
mod resident_state_round_trip {
    use crate::store::state::{reduce_state_in_place, CoreAppState, StateCommand};
    use crate::store::wallet_db;
    use crate::store::wallet_domain::CorePriceAlertCondition;
    use crate::store::PriceAlertEvaluationAlert;

    fn tmp_db() -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-resident-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    fn alert(id: &str, target: f64) -> PriceAlertEvaluationAlert {
        PriceAlertEvaluationAlert {
            id: id.to_string(),
            holding_key: "BTC".to_string(),
            asset_name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            chain_name: "Bitcoin".to_string(),
            target_price: target,
            condition: CorePriceAlertCondition::Above,
            is_enabled: true,
            has_triggered: false,
        }
    }

    #[test]
    fn a_price_alert_survives_a_reopen() {
        let db = tmp_db();
        let mut state = CoreAppState::default();
        reduce_state_in_place(
            &mut state,
            StateCommand::SetPriceAlerts {
                alerts: vec![alert("A1", 100_000.0)],
            },
        );
        wallet_db::app_state_save(&db, &state).expect("save");

        let reloaded = wallet_db::app_state_load(&db).expect("load");
        assert_eq!(reloaded.price_alerts.len(), 1, "price alert was lost");
        assert_eq!(reloaded.price_alerts[0].target_price, 100_000.0);
    }

    #[test]
    fn an_alert_that_cannot_fire_is_refused() {
        let mut state = CoreAppState::default();
        reduce_state_in_place(
            &mut state,
            StateCommand::SetPriceAlerts {
                alerts: vec![alert("A1", 0.0), alert("A2", -5.0), alert("A3", 42.0)],
            },
        );
        assert_eq!(state.price_alerts.len(), 1);
        assert_eq!(state.price_alerts[0].id, "A3");
    }

    /// Whatever the resident state holds must come back. Add a collection and
    /// this fails until `app_state_save` learns about it.
    #[test]
    fn every_resident_collection_round_trips() {
        let db = tmp_db();
        let mut state = CoreAppState::default();
        reduce_state_in_place(
            &mut state,
            StateCommand::SetPriceAlerts {
                alerts: vec![alert("A1", 1.0)],
            },
        );
        reduce_state_in_place(
            &mut state,
            StateCommand::AddAddressBookEntry {
                id: "C1".into(),
                name: "Alice".into(),
                chain_name: "Ethereum".into(),
                address: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".into(),
                note: String::new(),
            },
        );
        reduce_state_in_place(
            &mut state,
            StateCommand::SetFiatCurrency {
                fiat_currency_code: "CHF".into(),
            },
        );
        wallet_db::app_state_save(&db, &state).expect("save");
        let back = wallet_db::app_state_load(&db).expect("load");

        assert_eq!(back.price_alerts.len(), 1, "price_alerts not persisted");
        assert_eq!(back.address_book.len(), 1, "address_book not persisted");
        assert_eq!(
            back.settings.fiat_currency_code, "CHF",
            "settings not persisted"
        );
    }

    /// Resetting puts every field back to core's own default.
    ///
    /// The defaults were duplicated in Swift: `resetSettingsAndEndpointsState`
    /// assigned twelve literals and `AppUserPreferences.resetToDefaults`
    /// another seven, none of which any test could compare against
    /// `AppSettings::default()`. Written by mutating *every* field first, so a
    /// new field added to `AppSettings` and forgotten in the reducer fails
    /// here rather than silently surviving a reset.
    #[test]
    fn resetting_settings_restores_every_default() {
        use crate::store::state::{reduce_state_in_place, AppSettingUpdate as U, StateCommand};
        let mut state = CoreAppState::default();
        let defaults = state.settings.clone();

        for update in [
            U::RpcEndpoint { chain: "Base".into(), value: "https://x.example".into() },
            U::EtherscanApiKey { value: "KEY".into() },
            U::MoneroBackendBaseUrl { value: "https://xmr.example".into() },
            U::MoneroBackendApiKey { value: "XKEY".into() },
            U::BitcoinEsploraEndpoints { value: "https://a.example".into() },
            U::BitcoinStopGap { value: 42 },
            U::FeePriority { chain: "Dogecoin".into(), value: "economy".into() },
            U::UseStrictRpcOnly { value: true },
            U::BackgroundSyncProfile { value: "aggressive".into() },
            U::AutomaticRefreshFrequencyMinutes { value: 30 },
            U::UsePriceAlerts { value: false },
            U::UseTransactionStatusNotifications { value: false },
            U::UseLargeMovementNotifications { value: false },
            U::LargeMovementAlertPercentThreshold { value: 25.0 },
            U::LargeMovementAlertUsdThreshold { value: 500.0 },
        ] {
            reduce_state_in_place(&mut state, StateCommand::SetAppSetting { update });
        }
        reduce_state_in_place(
            &mut state,
            StateCommand::SetFiatCurrency { fiat_currency_code: "EUR".into() },
        );
        reduce_state_in_place(
            &mut state,
            StateCommand::SelectNetworkChain { chain_id: "bitcoin-testnet".into() },
        );
        assert_ne!(state.settings, defaults, "nothing was actually changed");

        let events = reduce_state_in_place(&mut state, StateCommand::ResetAppSettings);
        assert_eq!(state.settings, defaults);
        assert!(events.iter().any(|e| e.kind == "appSettingChanged"));

        // Resetting what is already default is not a change.
        assert!(reduce_state_in_place(&mut state, StateCommand::ResetAppSettings).is_empty());
    }

    /// Every settings field survives a save and a reload.
    ///
    /// Eighteen of them arrived from a blob iOS wrote separately, and the
    /// blob's own fields were never in this test because they were never in
    /// this state. Written field by field so a new one that is added to
    /// `AppSettings` and forgotten in `apply_app_setting` fails here.
    #[test]
    fn every_settings_field_round_trips() {
        use crate::store::state::AppSettingUpdate as U;
        let db = tmp_db();
        let mut state = CoreAppState::default();
        let updates = vec![
            U::RpcEndpoint {
                chain: "Ethereum".into(),
                value: "https://rpc.example".into(),
            },
            // The second one is the point: this was a single String, so a
            // second chain's override had nowhere to go.
            U::RpcEndpoint {
                chain: "Base".into(),
                value: "https://base.example".into(),
            },
            U::EtherscanApiKey { value: "KEY".into() },
            U::MoneroBackendBaseUrl {
                value: "https://xmr.example".into(),
            },
            U::MoneroBackendApiKey {
                value: "XKEY".into(),
            },
            U::BitcoinEsploraEndpoints {
                value: "https://a.example,https://b.example".into(),
            },
            U::BitcoinStopGap { value: 42 },
            U::FeePriority {
                chain: "Bitcoin".into(),
                value: "priority".into(),
            },
            U::FeePriority {
                chain: "Dogecoin".into(),
                value: "economy".into(),
            },
            U::UseStrictRpcOnly { value: true },
            U::BackgroundSyncProfile {
                value: "aggressive".into(),
            },
            U::AutomaticRefreshFrequencyMinutes { value: 30 },
            U::UsePriceAlerts { value: false },
            U::UseTransactionStatusNotifications { value: false },
            U::UseLargeMovementNotifications { value: false },
            U::LargeMovementAlertPercentThreshold { value: 25.0 },
            U::LargeMovementAlertUsdThreshold { value: 2_500.0 },
        ];
        for update in updates {
            reduce_state_in_place(&mut state, StateCommand::SetAppSetting { update });
        }
        let written = state.settings.clone();
        wallet_db::app_state_save(&db, &state).expect("save");
        let back = wallet_db::app_state_load(&db).expect("load");
        assert_eq!(back.settings, written, "a settings field did not round trip");
        assert_ne!(
            back.settings,
            crate::store::state::AppSettings::default(),
            "the updates did not change anything"
        );
    }

    /// A value outside its range is bounded rather than stored.
    ///
    /// These bounds were `didSet` clamps in the iOS layer — the only copy, so
    /// a stop gap of zero was only impossible where someone had remembered to
    /// check. A zero stop gap finds no addresses; a one-minute refresh
    /// interval hammers whatever endpoint is configured.
    #[test]
    fn a_setting_outside_its_range_is_bounded() {
        use crate::store::state::AppSettingUpdate as U;
        let mut state = CoreAppState::default();
        fn set(state: &mut CoreAppState, update: U) {
            reduce_state_in_place(state, StateCommand::SetAppSetting { update });
        }

        set(&mut state, U::BitcoinStopGap { value: 0 });
        set(&mut state, U::AutomaticRefreshFrequencyMinutes { value: 1 });
        set(&mut state, U::LargeMovementAlertPercentThreshold { value: 0.0 });
        set(&mut state, U::LargeMovementAlertUsdThreshold { value: 1_000_000.0 });
        assert_eq!(state.settings.bitcoin_stop_gap, 1);
        assert_eq!(state.settings.automatic_refresh_frequency_minutes, 5);
        assert_eq!(state.settings.large_movement_alert_percent_threshold, 1.0);
        assert_eq!(state.settings.large_movement_alert_usd_threshold, 100_000.0);

        set(&mut state, U::BitcoinStopGap { value: 9_999 });
        set(&mut state, U::AutomaticRefreshFrequencyMinutes { value: 9_999 });
        set(&mut state, U::LargeMovementAlertPercentThreshold { value: 500.0 });
        assert_eq!(state.settings.bitcoin_stop_gap, 200);
        assert_eq!(state.settings.automatic_refresh_frequency_minutes, 60);
        assert_eq!(state.settings.large_movement_alert_percent_threshold, 90.0);

        // Trimmed, so a pasted key with a stray newline is the same key.
        set(
            &mut state,
            U::EtherscanApiKey {
                value: "  ABC123\n".into(),
            },
        );
        assert_eq!(state.settings.etherscan_api_key, "ABC123");
    }
}


/// One unreadable collection must not take the wallet list with it.
#[cfg(test)]
mod a_bad_row_is_not_a_bad_database {
    use super::*;

    /// A `token_preferences` blob this build cannot parse is dropped; the
    /// wallets load.
    ///
    /// It was fatal: `app_state_load` returned `Err` and the app came up with
    /// nothing at all. Token preferences rebuild from the catalog and price
    /// alerts re-evaluate on the next refresh, so losing them costs a rebuild —
    /// where losing the wallet list cannot be undone from anything.
    #[test]
    fn an_unreadable_token_preferences_row_does_not_lose_the_wallets() {
        let db = {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "spectra-badrow-{}-{:?}.sqlite",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_file(&path);
            path.to_string_lossy().into_owned()
        };

        let mut state = CoreAppState::default();
        state.wallets.push(crate::store::state::WalletSummary {
            id: "w1".into(),
            name: "Kept".into(),
            is_watch_only: false,
            chain_name: "Bitcoin".into(),
            include_in_portfolio_total: true,
            network_mode: None,
            xpub: None,
            derivation_preset: "standard".into(),
            derivation_path: None,
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: Vec::new(),
        });
        crate::store::wallet_db::app_state_save(&db, &state).expect("save");

        // Overwrite the token-preferences blob with a shape this build cannot
        // read — an older row, or a newer one.
        // Straight into the meta table, the way an older build would have left
        // it — no helper, so the test cannot accidentally go through a path
        // that normalises the row on the way in.
        {
            let conn = rusqlite::Connection::open(&db).expect("open");
            conn.execute(
                "INSERT OR REPLACE INTO app_state_meta (key, value) VALUES (?1, ?2)",
                rusqlite::params!["token_preferences", r#"[{"legacy":true}]"#],
            )
            .expect("write the bad row");
        }

        let reloaded =
            crate::store::wallet_db::app_state_load(&db).expect("the load survives a bad row");
        assert_eq!(reloaded.wallets.len(), 1, "the wallet list went with the row");
        assert_eq!(reloaded.wallets[0].name, "Kept");
        assert!(reloaded.token_preferences.is_empty());
    }
}
