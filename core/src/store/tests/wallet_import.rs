use crate::derivation::import::{WalletImportCommit, WalletImportRequest};
use crate::service::WalletService;
use crate::store::wallet_domain::{CoreSeedDerivationPreset, CoreWalletDerivationOverrides};

const MNEMONIC: &str = "test test test test test test test test test test test junk";

fn commit(chains: &[&str]) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request: WalletImportRequest {
            wallet_name: String::new(),
            selected_chain_names: chains.iter().map(|c| c.to_string()).collect(),
            is_watch_only_import: false,
            is_private_key_import: false,
            watch_only_entries: Default::default(),
        },
        seed_derivation_preset: CoreSeedDerivationPreset::Standard,
        seed_derivation_paths: crate::app_core::seed_derivation_paths_for_account(0).unwrap(),
        derivation_overrides: CoreWalletDerivationOverrides::default(),
        seed_phrase: Some(MNEMONIC.into()),
        private_key: None,
    }
}

#[tokio::test]
async fn imported_wallets_land_in_core_state() {
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let service = WalletService::new(Vec::new()).expect("service");
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(temp.join("state.db").to_string_lossy().into())
        .await
        .unwrap();
    let outcome = service
        .import_wallets(commit(&["Solana"]))
        .await
        .expect("import");

    assert_eq!(outcome.wallets.len(), 1);
    // The caller does not store anything — core already did.
    let stored = service
        .portfolio_snapshot()
        .await
        .expect("snapshot")
        .wallets;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].selected_chain, "Solana");
    assert_eq!(
        stored[0].addresses.get("solana").map(String::as_str),
        Some(
            crate::derivation::import::derive_import_addresses(
                MNEMONIC,
                &["Solana".into()],
                &crate::app_core::seed_derivation_paths_for_account(0).unwrap(),
                &CoreWalletDerivationOverrides::default()
            )["Solana"]
                .as_str()
        )
    );
}

/// A seed import stores an address for every network of the family, so
/// switching to a testnet is a read rather than a derivation.
///
/// The app used to re-derive the testnet address from the seed on every
/// read, which a password-sealed wallet cannot do — it fell back to the
/// mainnet address and showed that on testnet instead. The addresses
/// survive into the stored record, which is what makes the switch work
/// after a restart.
#[tokio::test]
async fn a_seed_import_stores_one_address_per_network_of_its_family() {
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let service = WalletService::new(Vec::new()).expect("service");
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(temp.join("state.db").to_string_lossy().into())
        .await
        .unwrap();
    let mut commit = commit(&["Bitcoin"]);
    commit.seed_phrase = Some(MNEMONIC.to_string());
    commit.seed_derivation_paths =
        crate::app_core::seed_derivation_paths_for_account(0).expect("default paths");
    service.import_wallets(commit).await.expect("import");

    let stored = service
        .portfolio_snapshot()
        .await
        .expect("snapshot")
        .wallets;
    assert_eq!(stored.len(), 1);
    let addresses = &stored[0].addresses;
    for network in crate::registry::Chain::Bitcoin.network_choices() {
        let address = addresses
            .get(network.address_slot())
            .unwrap_or_else(|| panic!("no address for {}", network.chain_display_name()));
        let verdict = crate::validation::address::validate_address(
            crate::validation::address::AddressValidationRequest {
                kind: network.address_validation_kind().to_string(),
                value: address.clone(),
            },
        );
        assert!(
            verdict.is_valid,
            "{} stored {address}, which its own validator refuses",
            network.chain_display_name()
        );
    }
    // The networks are different keys, so the mainnet address is not the
    // testnet one.
    assert_ne!(
        addresses.get(crate::registry::Chain::Bitcoin.address_slot()),
        addresses.get(crate::registry::Chain::BitcoinTestnet4.address_slot())
    );
}

#[tokio::test]
async fn a_network_selection_applies_only_to_its_own_family() {
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let service = WalletService::new(Vec::new()).expect("service");
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(temp.join("state.db").to_string_lossy().into())
        .await
        .unwrap();
    // The selection is core's own setting, not something the commit carries.
    service
        .apply_state_command(crate::store::state::StateCommand::SelectChainForFamily {
            chain_id: "bitcoin-testnet".into(),
        })
        .await
        .unwrap();
    let outcome = service
        .import_wallets(commit(&["Bitcoin", "Solana"]))
        .await
        .expect("import");

    let by_chain: std::collections::HashMap<_, _> = outcome
        .wallets
        .iter()
        .map(|w| (w.selected_chain.as_str(), w))
        .collect();
    assert_eq!(by_chain["Bitcoin"].chain_id, "bitcoin-testnet");
    // Choosing Bitcoin testnet must not drag the Solana wallet with it.
    assert_eq!(by_chain["Solana"].chain_id, "solana");
    // Each wallet starts with its own network's native holding, and no other.
    let holdings = |chain: &str| {
        by_chain[chain]
            .holdings
            .iter()
            .map(|h| h.chain_name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(holdings("Bitcoin"), vec!["Bitcoin Testnet".to_string()]);
    assert_eq!(holdings("Solana"), vec!["Solana".to_string()]);
}

#[derive(Default)]
struct FailingSecrets {
    inner: crate::store::secret_backends::InMemorySecretStore,
    writes: std::sync::atomic::AtomicUsize,
}
impl crate::store::secret_store::SecretStore for FailingSecrets {
    fn load_secret(
        &self,
        kind: crate::store::secret_store::SecretClass,
        key: String,
    ) -> Result<String, crate::store::secret_store::SecretStoreError> {
        self.inner.load_secret(kind, key)
    }
    fn save_secret(
        &self,
        kind: crate::store::secret_store::SecretClass,
        key: String,
        value: String,
    ) -> Result<(), crate::store::secret_store::SecretStoreError> {
        if self
            .writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 1
        {
            return Err(crate::store::secret_store::SecretStoreError::Backend {
                message: "injected write failure".into(),
            });
        }
        self.inner.save_secret(kind, key, value)
    }
    fn delete_secret(
        &self,
        kind: crate::store::secret_store::SecretClass,
        key: String,
    ) -> Result<(), crate::store::secret_store::SecretStoreError> {
        self.inner.delete_secret(kind, key)
    }
}

#[tokio::test]
async fn failed_multi_wallet_import_leaves_neither_wallets_nor_partial_secrets_and_retries() {
    let path = std::env::temp_dir().join(format!(
        "spectra-import-{}.db",
        crate::store::new_transaction_id()
    ));
    let service = WalletService::new(vec![]).unwrap();
    let store = std::sync::Arc::new(FailingSecrets::default());
    service.set_secret_store(store.clone());
    service
        .open_state(path.to_string_lossy().into())
        .await
        .unwrap();
    let input = commit(&["Ethereum", "Solana"]);
    assert!(service.import_wallets(input.clone()).await.is_err());
    assert!(service.app_state().await.wallets.is_empty());
    assert_eq!(store.inner.len(), 0);
    assert!(
        crate::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(
            path.to_str().unwrap()
        ))
        .unwrap()
        .wallets
        .is_empty()
    );
    let outcome = service.import_wallets(input).await.unwrap();
    assert_eq!(outcome.wallets.len(), 2);
    for wallet in outcome.wallets {
        assert!(
            service
                .wallet_secret_state(wallet.id)
                .unwrap()
                .has_signing_material
        );
    }
    assert_eq!(
        crate::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(
            path.to_str().unwrap()
        ))
        .unwrap()
        .wallets
        .len(),
        2
    );
}

#[tokio::test]
async fn database_failure_rolls_back_import_secrets_and_missing_material_is_refused() {
    let path = std::env::temp_dir().join(format!(
        "spectra-import-{}.db",
        crate::store::new_transaction_id()
    ));
    let service = WalletService::new(vec![]).unwrap();
    let store = std::sync::Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
    service.set_secret_store(store.clone());
    service
        .open_state(path.to_string_lossy().into())
        .await
        .unwrap();
    let mut missing = commit(&["Solana"]);
    missing.seed_phrase = None;
    assert!(service.import_wallets(missing).await.is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON wallets BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(service
        .import_wallets(commit(&["Ethereum", "Solana"]))
        .await
        .is_err());
    assert_eq!(store.len(), 0);
    assert!(service.app_state().await.wallets.is_empty());
}

#[tokio::test]
async fn default_wallet_names_are_allocated_under_the_import_writer() {
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("state.db").to_string_lossy().into_owned();
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service.open_state(path.clone()).await.unwrap();
    let mut named = commit(&["Solana"]);
    named.request.wallet_name = "Wallet 1".into();
    service.import_wallets(named).await.unwrap();
    let (one, two) = tokio::join!(
        service.import_wallets(commit(&["Solana"])),
        service.import_wallets(commit(&["Solana"]))
    );
    let names: std::collections::HashSet<_> = [
        one.unwrap().wallets[0].name.clone(),
        two.unwrap().wallets[0].name.clone(),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        names,
        ["Wallet 2".into(), "Wallet 3".into()].into_iter().collect()
    );
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    reopened.open_state(path).await.unwrap();
    assert_eq!(
        reopened
            .import_wallets(commit(&["Solana"]))
            .await
            .unwrap()
            .wallets[0]
            .name,
        "Wallet 4"
    );
    std::fs::remove_dir_all(temp).unwrap();
}

#[tokio::test]
async fn raw_mnemonic_is_canonical_before_derivation_and_storage() {
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(temp.join("state.db").to_string_lossy().into())
        .await
        .unwrap();
    let mut raw = commit(&["Ethereum"]);
    raw.seed_phrase = Some(format!(
        "  {}  ",
        MNEMONIC.to_uppercase().replace(' ', "\t\n")
    ));
    let imported = service.import_wallets(raw).await.unwrap();
    let normal = service.import_wallets(commit(&["Ethereum"])).await.unwrap();
    assert_eq!(imported.wallets[0].addresses, normal.wallets[0].addresses);
    assert_eq!(
        service
            .wallet_seed_phrase(imported.wallets[0].id.clone(), None)
            .unwrap(),
        MNEMONIC
    );
}

#[tokio::test]
async fn deep_rescan_reports_provider_failures_and_empty_scope_success() {
    use crate::fetch::refresh::policy::DeviceConditions;
    use crate::service::app_refresh::AppRefreshIntent;
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(temp.join("state.db").to_string_lossy().into())
        .await
        .unwrap();
    let conditions = DeviceConditions {
        app_is_active: true,
        is_network_reachable: true,
        is_constrained_network: false,
        is_expensive_network: false,
        is_low_power_mode: false,
        battery_level: 1.0,
        wants_price_refresh: false,
    };
    let intent = AppRefreshIntent::DeepRescan {
        chain_id: "bitcoin".into(),
    };
    let empty = service
        .refresh_app(intent.clone(), conditions.clone())
        .await
        .unwrap();
    assert!(empty.failures.is_empty());
    service.import_wallets(commit(&["Bitcoin"])).await.unwrap();
    // No configured providers: all network work must fail locally and remain visible.
    let result = service.refresh_app(intent, conditions).await.unwrap();
    assert!(!result.failures.is_empty());
    assert!(result.state.quotes.prices_attempt_at.is_none());
}

#[tokio::test]
async fn testnet_paths_survive_reopen_switching_and_signing() {
    use crate::registry::Chain;
    use crate::store::secret_backends::InMemorySecretStore;
    use crate::store::state::StateCommand;
    use std::sync::Arc;
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let db = temp.join("state.db").to_string_lossy().into_owned();
    let secrets = Arc::new(InMemorySecretStore::new());
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(secrets.clone());
    service.open_state(db.clone()).await.unwrap();
    let mut input = commit(&["Bitcoin"]);
    input.password = Some("test-password".into());
    // Custom mainnet and testnet paths must not overwrite each other.
    input
        .seed_derivation_paths
        .set_path_for(Chain::Bitcoin, "m/84'/0'/2'/0/0");
    input
        .seed_derivation_paths
        .set_path_for(Chain::BitcoinTestnet4, "m/84'/1'/3'/0/0");
    // An explicitly selected mainnet-style path on a testnet remains valid
    // user input; network defaults must never rewrite it.
    input
        .seed_derivation_paths
        .set_path_for(Chain::BitcoinSignet, "m/84'/0'/9'/0/0");
    let outcome = service.import_wallets(input).await.unwrap();
    let wallet_id = outcome.wallets[0].id.clone();
    drop(service);
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(secrets);
    service.open_state(db).await.unwrap();
    for (chain, path) in [
        (Chain::BitcoinTestnet4, "m/84'/1'/3'/0/0"),
        (Chain::BitcoinSignet, "m/84'/0'/9'/0/0"),
        (Chain::BitcoinTestnet, "m/84'/1'/0'/0/0"),
        (Chain::Bitcoin, "m/84'/0'/2'/0/0"),
    ] {
        service
            .apply_state_command(StateCommand::SelectChainForFamily {
                chain_id: chain.str_id().into(),
            })
            .await
            .unwrap();
        let state = service.app_state().await;
        let wallet = &state.wallets[0];
        assert_eq!(wallet.derivation_path.as_deref(), Some(path));
        let expected = crate::derivation::dispatch::derive_for_chain_name(
            chain.chain_display_name(),
            MNEMONIC,
            path,
            None,
            None,
            None,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        assert_eq!(wallet.address_on(chain), Some(expected.as_str()));
        assert_eq!(
            service
                .send_identity_address(
                    wallet_id.clone(),
                    chain.str_id().into(),
                    Some("test-password".into())
                )
                .await
                .unwrap(),
            expected
        );
    }
}

#[test]
fn an_absent_testnet_address_never_falls_back_to_mainnet() {
    let mut wallet = crate::store::state::WalletState::single_address(
        "w",
        "Wallet",
        "Bitcoin",
        "bc1main",
        Some("m/84'/0'/0'/0/0".into()),
        false,
    );
    wallet.chain_id = "bitcoin-testnet-4".into();
    assert!(wallet.active_address().is_none());
}
