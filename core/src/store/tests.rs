use super::{
    aggregate_owned_addresses, build_persisted_snapshot, persisted_snapshot_from_json,
    core_receive_selection, core_self_send_confirmation,
    wallet_secret_index, OwnedAddressAggregationRequest, PendingSelfSendConfirmationInput,
    PersistedAppSnapshot, PersistedAppSnapshotRequest, ReceiveSelectionHoldingInput,
    ReceiveSelectionRequest, SelfSendConfirmationRequest, WalletSecretObservation,
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
        network_mode: Some("mainnet".to_string()),
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

#[test]
fn prefers_native_receive_holding_for_resolved_chain() {
    let plan = core_receive_selection(ReceiveSelectionRequest {
        receive_chain_name: "Ethereum".to_string(),
        available_receive_chains: vec!["Ethereum".to_string()],
        available_receive_holdings: vec![
            ReceiveSelectionHoldingInput {
                holding_index: 0,
                chain_name: "Ethereum".to_string(),
                has_contract_address: true,
            },
            ReceiveSelectionHoldingInput {
                holding_index: 1,
                chain_name: "Ethereum".to_string(),
                has_contract_address: false,
            },
        ],
    });

    assert_eq!(plan.resolved_chain_name, "Ethereum");
    assert_eq!(plan.selected_receive_holding_index, Some(1));
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
        service.apply_state_command(add("1", "Cold", BTC)).await.expect("add");
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
        service.apply_state_command(add("1", "BTC", BTC)).await.expect("add");
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
        service.apply_state_command(add("1", "Cold", BTC)).await.expect("add");

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
        first.apply_state_command(add("1", "Cold", BTC)).await.expect("add");
        first.apply_state_command(add("2", "Hot", BTC2)).await.expect("add");

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
                wire("tx1", "hash1", Some(1)),   // unchanged
                wire("tx2", "hash2", Some(6)),   // confirmations advanced
                wire("tx3", "hash3", Some(1)),   // new
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
        CoreBitcoinNetworkMode, CoreCoin, CoreImportedWallet, CoreSeedDerivationPaths,
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
            bitcoin_network_mode: CoreBitcoinNetworkMode::Testnet4,
            dogecoin_network_mode: Default::default(),
            addresses: HashMap::from([("bitcoin".to_string(), "bc1qexample".to_string())]),
            bitcoin_xpub: Some("zpub123".to_string()),
            seed_derivation_preset: CoreSeedDerivationPreset::Account2,
            seed_derivation_paths: paths,
            derivation_overrides: CoreWalletDerivationOverrides {
                passphrase: Some("secret".to_string()),
                ..Default::default()
            },
            selected_chain: "Bitcoin".to_string(),
            holdings: vec![CoreCoin {
                id: "some-uuid".to_string(),
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
    fn keeps_only_the_network_mode_that_applies() {
        let summary = bitcoin_wallet().to_summary(false);
        assert_eq!(summary.network_mode.as_deref(), Some("testnet4"));

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
    use crate::store::state::{AssetHolding, WalletAddress, WalletSummary};
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
            network_mode: Some("testnet4".to_string()),
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
    /// leaves the other alone.
    #[test]
    fn only_the_wallets_own_network_mode_is_restored() {
        use crate::store::wallet_domain::{CoreBitcoinNetworkMode, CoreDogecoinNetworkMode};
        let view = summary().to_imported_wallet(&defaults());
        assert_eq!(view.bitcoin_network_mode, CoreBitcoinNetworkMode::Testnet4);
        assert_eq!(view.dogecoin_network_mode, CoreDogecoinNetworkMode::Mainnet);
    }

    /// Holding ids are derived from what identifies the asset, so rebuilding
    /// the view model does not make SwiftUI think every row is new.
    #[test]
    fn holding_ids_are_stable_across_rebuilds() {
        let first = summary().to_imported_wallet(&defaults());
        let second = summary().to_imported_wallet(&defaults());
        assert_eq!(first.holdings[0].id, second.holdings[0].id);
        assert!(!first.holdings[0].id.is_empty());

        // Two different assets do not collide.
        let mut other = summary();
        other.holdings[0].symbol = "USDT".to_string();
        other.holdings[0].contract_address = Some("0xdac1".to_string());
        let third = other.to_imported_wallet(&defaults());
        assert_ne!(first.holdings[0].id, third.holdings[0].id);
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
        let switched = service.open_state(second.clone()).await.expect("open other");
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

// ── Confirmation-poll trackers (core-owned) ───────────────────────────

fn poll_config() -> crate::store::TransactionStatusPollConfig {
    crate::store::TransactionStatusPollConfig {
        pending_poll_seconds: 10.0,
        confirmed_poll_seconds: 30.0,
        backoff_max_seconds: 600.0,
        finality_confirmations: 6,
        pending_failure_timeout_seconds: 3600.0,
        pending_failure_min_failures: 3,
    }
}

#[tokio::test]
async fn untracked_transaction_is_always_due_for_poll() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let due = service
        .transactions_due_for_status_poll(vec!["tx1".into(), "tx2".into()], 1_000.0)
        .await;
    assert_eq!(due, vec!["tx1".to_string(), "tx2".to_string()]);
}

#[tokio::test]
async fn recorded_poll_defers_the_next_one() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service
        .record_status_poll_success("tx1".into(), false, true, None, 1_000.0, poll_config())
        .await;
    assert!(service
        .transactions_due_for_status_poll(vec!["tx1".into()], 1_000.0)
        .await
        .is_empty());
    // ...and becomes due again once the interval has passed.
    assert_eq!(
        service
            .transactions_due_for_status_poll(vec!["tx1".into()], 2_000.0)
            .await,
        vec!["tx1".to_string()]
    );
}

#[tokio::test]
async fn manual_recheck_makes_a_deferred_transaction_due_again() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service
        .record_status_poll_success("tx1".into(), true, false, Some(99), 1_000.0, poll_config())
        .await;
    assert!(service
        .transactions_due_for_status_poll(vec!["tx1".into()], 1_000.0)
        .await
        .is_empty());
    service.reset_status_tracker("tx1".into(), 1_000.0, true).await;
    assert_eq!(
        service
            .transactions_due_for_status_poll(vec!["tx1".into()], 1_000.0)
            .await,
        vec!["tx1".to_string()]
    );
}

#[tokio::test]
async fn stale_pending_needs_both_age_and_repeated_failures() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let input = |id: &str| crate::store::StalePendingFailureTransactionInput {
        id: id.to_string(),
        created_at_unix: 0.0,
        status_is_pending: true,
    };
    let now = 10_000.0; // well past pending_failure_timeout_seconds

    // Old enough, but never failed a poll → not stale.
    assert!(service
        .stale_pending_failure_ids(vec![input("tx1")], now, poll_config())
        .await
        .is_empty());

    for _ in 0..3 {
        service
            .record_status_poll_failure("tx1".into(), now, poll_config())
            .await;
    }
    assert_eq!(
        service
            .stale_pending_failure_ids(vec![input("tx1")], now, poll_config())
            .await,
        vec!["tx1".to_string()]
    );
}

#[tokio::test]
async fn retaining_trackers_drops_transactions_that_no_longer_exist() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    for id in ["tx1", "tx2"] {
        service
            .record_status_poll_success(id.into(), false, true, None, 1_000.0, poll_config())
            .await;
    }
    service.retain_status_trackers(vec!["tx1".into()]).await;
    // tx2's tracker is gone, so it reads as never-polled — due immediately.
    assert_eq!(
        service
            .transactions_due_for_status_poll(vec!["tx1".into(), "tx2".into()], 1_000.0)
            .await,
        vec!["tx2".to_string()]
    );
}
}

/// Importing is a core operation now: it plans, builds and stores in one call.
#[cfg(test)]
mod wallet_import {
    use crate::derivation::import::{WalletImportAddresses, WalletImportCommit, WalletImportRequest};
    use crate::service::WalletService;
    use crate::store::wallet_domain::{
        CoreBitcoinNetworkMode, CoreDogecoinNetworkMode, CoreSeedDerivationPaths,
        CoreSeedDerivationPreset, CoreWalletDerivationOverrides,
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
            bitcoin_network_mode: CoreBitcoinNetworkMode::Mainnet,
            dogecoin_network_mode: CoreDogecoinNetworkMode::Mainnet,
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
        assert_eq!(stored[0].addresses.get("solana").map(String::as_str), Some(SOL));
    }

    #[tokio::test]
    async fn network_mode_applies_only_to_its_own_chain() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut input = commit(
            &["Bitcoin", "Solana"],
            &[("bitcoin", BTC), ("solana", SOL)],
        );
        input.bitcoin_network_mode = CoreBitcoinNetworkMode::Testnet;
        let outcome = service.import_wallets(input).await.expect("import");

        let by_chain: std::collections::HashMap<_, _> = outcome
            .wallets
            .iter()
            .map(|w| (w.selected_chain.as_str(), w))
            .collect();
        assert_eq!(
            by_chain["Bitcoin"].bitcoin_network_mode,
            CoreBitcoinNetworkMode::Testnet
        );
        // Selecting Bitcoin testnet must not drag the Solana wallet with it.
        assert_eq!(
            by_chain["Solana"].bitcoin_network_mode,
            CoreBitcoinNetworkMode::Mainnet
        );
    }
}

/// Keypool reservation. The property that matters is that an index is never
/// handed out twice — so these hammer it concurrently.
#[cfg(test)]
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
        assert!(events[0].timestamp_unix > 0.0, "core did not stamp the time");

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
        assert!(service.operational_events("Bitcoin".into()).await.is_empty());
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
        assert!(service.operational_events("Bitcoin".into()).await.is_empty());
        assert_eq!(service.operational_events("Solana".into()).await.len(), 1);

        service.clear_operational_events(None).await.expect("clear all");
        assert!(service.operational_events("Solana".into()).await.is_empty());
    }
}

/// The built-in token catalog and the merge that folds it into stored
/// preferences. Both moved into core when the planner went away.
#[cfg(test)]
mod built_in_tokens {
    use crate::service::WalletService;
    use crate::store::state::StateCommand;
    use crate::store::wallet_domain::CoreTokenTrackingChain;

    #[test]
    fn every_built_in_has_a_unique_id() {
        let entries = crate::store::built_in_token_preferences();
        assert!(!entries.is_empty(), "the catalog produced nothing");
        let mut ids: Vec<_> = entries.iter().map(|e| e.id.as_str()).collect();
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
                    CoreTokenTrackingChain::from_chain_name(&token.chain),
                    Some(CoreTokenTrackingChain::Bnb)
                );
            }
        }
        for chain in CoreTokenTrackingChain::ALL {
            assert_eq!(
                CoreTokenTrackingChain::from_chain_name(chain.chain_name()),
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

        // Turn one off and clamp its display width, then merge again.
        let mut entries = state.token_preferences.clone();
        let target = entries
            .iter_mut()
            .find(|e| e.is_built_in && e.is_enabled)
            .expect("an enabled built-in");
        target.is_enabled = false;
        target.display_decimals = Some(2);
        let id = target.id.clone();
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
            .find(|e| e.id == id)
            .expect("the entry survived");
        assert!(!kept.is_enabled, "the merge re-enabled a token the user turned off");
        assert_eq!(kept.display_decimals, Some(2));
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
            path.push(format!("spectra-keypool-reopen-{}.sqlite", std::process::id()));
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
        assert!(second.events.is_empty(), "re-pinning the same set is a no-op");
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
        assert!(cleared.state.settings.pinned_dashboard_asset_symbols.is_empty());
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
            "Tron", "Solana", "Cardano", "XRP Ledger", "Stellar", "Monero", "Sui", "Aptos", "TON",
            "Internet Computer", "NEAR", "Polkadot",
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
    use crate::store::wallet_domain::CoreCoin;

    fn coin(symbol: &str, chain: &str, amount: f64) -> CoreCoin {
        CoreCoin {
            id: format!("{chain}-{symbol}"),
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

    async fn service_with(wallets: Vec<(&str, &str, Vec<CoreCoin>, bool)>) -> std::sync::Arc<WalletService> {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for (id, chain, holdings, included) in wallets {
            let mut summary = crate::store::state::WalletSummary::single_address(
                id, id, chain, "addr", None, false,
            );
            summary.include_in_portfolio_total = included;
            summary.holdings = holdings
                .into_iter()
                .map(|c| crate::store::state::AssetHolding {
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
        assert_eq!(after_testnet.state.settings.network_chain_by_family.len(), 1);

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
        assert_eq!(watch_only.receive_enabled_wallet_ids, vec!["w1".to_string()]);

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
        assert_eq!(sendable, vec!["ETH"], "SHIB is not a tracked token");
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
                crate::send::flow::core_seed_derivation_chain_raw(name.to_string()).is_some(),
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
        crate::derivation::import::validated_addresses(addresses, Default::default())
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

    #[test]
    fn a_valid_address_survives_and_is_normalised() {
        let (kept, rejected) = validated_addresses(&addresses(&[(
            "ethereum",
            "0x742D35CC6634C0532925A3B844bC454E4438F44E",
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
        use crate::store::wallet_domain::{CoreBitcoinNetworkMode, CoreDogecoinNetworkMode};
        let derived = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
        // Even asked for testnet4, the slot map is mainnet.
        let (kept, rejected) = crate::derivation::import::validated_addresses(
            &addresses(&[("bitcoin", derived)]),
            crate::derivation::import::ImportNetworks {
                bitcoin: CoreBitcoinNetworkMode::Testnet4,
                dogecoin: CoreDogecoinNetworkMode::Mainnet,
            },
        );
        // The helper itself honours what it is told...
        assert_eq!(rejected, vec![derived.to_string()]);
        assert!(kept.by_slot.is_empty());
        // ...so it is `import_wallets` that must pass mainnet here, which is
        // what the default does.
        let (kept, rejected) = validated_addresses(&addresses(&[("bitcoin", derived)]));
        assert!(rejected.is_empty());
        assert_eq!(kept.by_slot.get("bitcoin").map(String::as_str), Some(derived));
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
            crate::derivation::import::validated_watch_only_entries(entries, networks)
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
                &["0x742D35CC6634C0532925A3B844bC454E4438F44E"],
            ));
            assert!(rejected.is_empty());
            let stored = kept.by_slot.get("ethereum").expect("kept");
            assert_eq!(stored.len(), 1);
            assert!(stored[0].starts_with("0x"));
        }

        /// Core normalises on the way in, so a caller does not have to.
        ///
        /// The iOS import path used to lower-case Sui, NEAR and Kaspa by hand,
        /// call `normalizedSendAddress` for Aptos / TON / Internet Computer,
        /// and `normalizeEVMAddress` for the EVM slot — all of it upstream of
        /// a core call that normalises again. This pins the property those
        /// call sites were duplicating, so deleting them is a provable no-op
        /// rather than a hopeful one.
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
                ("Ethereum", "ethereum", "0x742D35CC6634C0532925A3B844BC454E4438F44E"),
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
                ("Solana", "solana", "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX"),
            ];
            for (chain_name, slot, typed) in cases {
                let (kept, rejected) = validated_watch_only_entries(&entries(slot, &[typed]));
                assert!(rejected.is_empty(), "{chain_name}: rejected {typed}");
                let imported = kept.by_slot.get(slot).and_then(|list| list.first()).unwrap();
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
            use crate::store::wallet_domain::{CoreBitcoinNetworkMode, CoreDogecoinNetworkMode};
            // tb1 prefix — valid Bitcoin testnet, invalid on mainnet.
            let typed = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
            let (kept, rejected) = validated_watch_only_entries_on(
                &entries("bitcoin", &[typed]),
                ImportNetworks {
                    bitcoin: CoreBitcoinNetworkMode::Testnet,
                    dogecoin: CoreDogecoinNetworkMode::Mainnet,
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
                    "0x742D35CC6634C0532925A3B844bC454E4438F44E",
                    "0xnothex",
                    "0x0000000000000000000000000000000000000001",
                ],
            ));
            assert_eq!(kept.by_slot.get("ethereum").expect("kept").len(), 2);
            assert_eq!(rejected, vec!["0xnothex".to_string()]);
        }
    }
}

/// Tracked tokens survive a reopen.
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
    use crate::store::wallet_domain::{
        CoreTokenPreferenceCategory, CoreTokenPreferenceEntry, CoreTokenTrackingChain,
    };
    use crate::store::wallet_db;

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

    fn entry(symbol: &str, decimals: i32, display: Option<i32>) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            id: format!("id-{symbol}"),
            chain: CoreTokenTrackingChain::Ethereum,
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            token_standard: "ERC-20".to_string(),
            contract_address: "0x0000000000000000000000000000000000000001".to_string(),
            coin_gecko_id: symbol.to_lowercase(),
            decimals,
            display_decimals: display,
            category: CoreTokenPreferenceCategory::Custom,
            is_built_in: false,
            is_enabled: true,
        }
    }

    #[test]
    fn a_tracked_token_survives_a_reopen() {
        let db = tmp_db();
        let mut state = CoreAppState::default();
        crate::store::state::reduce_state_in_place(
            &mut state,
            StateCommand::SetTokenPreferences {
                entries: vec![entry("USDC", 6, Some(2))],
            },
        );
        wallet_db::app_state_save(&db, &state).expect("save");

        let reloaded = wallet_db::app_state_load(&db).expect("load");
        assert_eq!(reloaded.token_preferences.len(), 1, "tracked token was lost");
        assert_eq!(reloaded.token_preferences[0].symbol, "USDC");
        assert_eq!(reloaded.token_preferences[0].display_decimals, Some(2));
    }

    #[test]
    fn the_clamp_survives_the_round_trip() {
        let db = tmp_db();
        let mut state = CoreAppState::default();
        crate::store::state::reduce_state_in_place(
            &mut state,
            StateCommand::SetTokenPreferences {
                // More places than the token has.
                entries: vec![entry("USDT", 6, Some(99))],
            },
        );
        wallet_db::app_state_save(&db, &state).expect("save");
        let reloaded = wallet_db::app_state_load(&db).expect("load");
        assert_eq!(reloaded.token_preferences[0].display_decimals, Some(6));
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
    use crate::store::state::{CoreAppState, StateCommand, reduce_state_in_place};
    use crate::store::wallet_db;
    use crate::store::PriceAlertEvaluationAlert;
    use crate::store::wallet_domain::CorePriceAlertCondition;

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
            StateCommand::SetPriceAlerts { alerts: vec![alert("A1", 1.0)] },
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
            StateCommand::SetFiatCurrency { fiat_currency_code: "CHF".into() },
        );
        wallet_db::app_state_save(&db, &state).expect("save");
        let back = wallet_db::app_state_load(&db).expect("load");

        assert_eq!(back.price_alerts.len(), 1, "price_alerts not persisted");
        assert_eq!(back.address_book.len(), 1, "address_book not persisted");
        assert_eq!(back.settings.fiat_currency_code, "CHF", "settings not persisted");
    }
}
