use crate::derivation::import::{WalletImportCommit, WalletImportKind, WalletImportRequest};
use crate::service::WalletService;
use crate::store::wallet_domain::WalletDerivationOverrides;

const MNEMONIC: &str = "test test test test test test test test test test test junk";

#[tokio::test]
async fn evm_imports_keep_each_networks_address_and_derivation_path() {
    use crate::registry::Chain;
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("state.db").to_string_lossy().into_owned();
    let secrets = std::sync::Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(secrets.clone());
    service.open_state(path.clone()).await.unwrap();
    let paths = [
        (Chain::Ethereum, "m/44'/60'/0'/0/0"),
        (Chain::Arbitrum, "m/44'/60'/1'/0/0"),
        (Chain::Base, "m/44'/60'/2'/0/0"),
    ];
    for (chain, path) in paths {
        let mut input = commit(chain);
        input.derivation_path = Some(path.into());
        service.import_wallets(input).await.unwrap();
    }
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.set_secret_store(secrets);
    let state = reopened.open_state(path).await.unwrap();
    for (chain, path) in paths {
        let wallet = state.wallets.iter().find(|w| w.chain_id == chain).unwrap();
        let expected = crate::derivation::dispatch::derive_for_chain(
            chain, MNEMONIC, path, None, None, None, true, false, false,
        )
        .unwrap()
        .address
        .unwrap();
        let expected = crate::send::flow::normalized_send_address(chain, expected);
        assert_eq!(wallet.active_address(), Some(expected.as_str()), "{chain}");
        assert_eq!(wallet.derivation_path.as_deref(), Some(path));
        let address = wallet
            .addresses
            .iter()
            .find(|a| a.chain_id == chain)
            .unwrap();
        assert_eq!(address.derivation_path.as_deref(), Some(path));
        assert_eq!(
            reopened
                .send_identity_address(wallet.id.clone(), chain, None)
                .await
                .unwrap(),
            expected,
        );
    }
    std::fs::remove_dir_all(temp).unwrap();
}

#[tokio::test]
async fn evm_identity_reuses_the_wallets_path_without_duplicate_address_records() {
    use crate::registry::Chain;
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("state.db").to_string_lossy().into_owned();
    let secrets = std::sync::Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(secrets.clone());
    service.open_state(path.clone()).await.unwrap();
    for (chain, path) in [
        (Chain::Ethereum, "m/44'/60'/3'/0/0"),
        (Chain::EthereumClassic, "m/44'/61'/2'/0/0"),
        (Chain::EthereumSepolia, "m/44'/60'/4'/0/0"),
        (Chain::EthereumClassicMordor, "m/44'/61'/5'/0/0"),
    ] {
        let mut input = commit(chain);
        input.derivation_path = Some(path.into());
        service.import_wallets(input).await.unwrap();
    }
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.set_secret_store(secrets);
    let state = reopened.open_state(path).await.unwrap();
    for wallet in state.wallets {
        assert_eq!(wallet.addresses.len(), 1);
        assert_eq!(wallet.addresses[0].chain_id, wallet.chain_id);
        let expected = wallet.active_address().unwrap();
        let view = wallet.to_wallet_view();
        assert_eq!(view.addresses.len(), 1);
        assert_eq!(
            view.addresses.get("ethereum").map(String::as_str),
            Some(expected)
        );
        let round_tripped = view.to_wallet_state().unwrap();
        assert_eq!(round_tripped.chain_id, wallet.chain_id);
        assert_eq!(round_tripped.derivation_path, wallet.derivation_path);
        assert_eq!(round_tripped.addresses, wallet.addresses);
        for chain in Chain::all().filter(|c| c.is_evm()) {
            assert_eq!(wallet.address_on(chain), Some(expected));
            assert_eq!(view.address_for(chain), Some(expected));
            assert_eq!(
                reopened
                    .send_identity_address(wallet.id.clone(), chain, None)
                    .await
                    .unwrap(),
                expected,
                "{} identity on {chain}",
                wallet.chain_id,
            );
        }
    }
    std::fs::remove_dir_all(temp).unwrap();
}

fn commit(chain: crate::registry::Chain) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request: WalletImportRequest {
            wallet_name: String::new(),
            chain,
            kind: WalletImportKind::Phrase,
        },
        derivation_path: None,
        derivation_overrides: WalletDerivationOverrides::default(),
        seed_phrase: Some(MNEMONIC.into()),
        private_key: None,
        restore_height: None,
        named_account: None,
        ton_wallet_version: None,
        upgrade_wallet_id: None,
    }
}

#[tokio::test]
async fn account_utxo_import_refuses_wrong_network_paths_before_storing() {
    use crate::registry::Chain;
    for chain in Chain::all().filter(|chain| chain.uses_account_utxo()) {
        let directory = std::env::temp_dir().join(crate::store::new_event_id());
        std::fs::create_dir_all(&directory).unwrap();
        let service = WalletService::new(vec![]).unwrap();
        service.set_secret_store(std::sync::Arc::new(
            crate::store::secret_backends::InMemorySecretStore::new(),
        ));
        service
            .open_state(directory.join("state.db").to_string_lossy().into_owned())
            .await
            .unwrap();
        // The other network's coin type (Kaspa's test network shares its
        // mainnet's, so no coin type is the other's there), a hardened
        // address index, and a branch past change.
        let default = crate::derivation::path::default_path_from_catalog(chain).unwrap();
        let segments: Vec<&str> = default.split('/').collect();
        let purpose = segments[1];
        let coin: u32 = segments[2].trim_end_matches('\'').parse().unwrap();
        let other_coin = if chain.is_testnet() { 0 } else { 1 };
        let wrong_network = format!("m/{purpose}/{other_coin}'/0'/0/0");
        let paths = [
            (other_coin != coin).then_some(wrong_network),
            Some(format!("m/{purpose}/{coin}'/0'/0'/0")),
            Some(format!("m/{purpose}/{coin}'/0'/2/0")),
        ];
        for path in paths.into_iter().flatten() {
            let path = path.as_str();
            let mut input = commit(chain);
            input.derivation_path = Some(path.into());
            assert!(
                service.import_wallets(input).await.is_err(),
                "{chain} accepted {path}"
            );
            assert!(
                service.app_state().await.wallets.is_empty(),
                "{chain} stored an invalid account"
            );
        }
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[tokio::test]
async fn imported_protected_peercoin_accounts_receive_after_restart_without_secrets() {
    use crate::registry::Chain;
    let directory = std::env::temp_dir().join(crate::store::new_event_id());
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("state.db").to_string_lossy().into_owned();
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service.open_state(database.clone()).await.unwrap();
    let mut imported = Vec::new();
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        let mut input = commit(chain);
        input.password = Some("public account fixture".into());
        input.derivation_overrides.passphrase = Some("Peercoin passphrase".into());
        imported.extend(service.import_wallets(input).await.unwrap().wallets);
    }
    drop(service);
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.open_state(database).await.unwrap();
    for wallet in imported {
        let state = reopened.app_state().await;
        let stored = state
            .wallets
            .iter()
            .find(|stored| stored.id == wallet.id)
            .unwrap();
        assert!(stored.xpub.as_deref().is_some_and(|xpub| !xpub.is_empty()));
        let received = reopened
            .receive_address(wallet.id, stored.chain_id, true)
            .await
            .unwrap()
            .unwrap();
        let base = crate::derivation::path::default_path_from_catalog(stored.chain_id).unwrap();
        let expected_path =
            crate::derivation::path::derivation_path_replacing_last_two(base.clone(), 0, 1, base);
        let expected = crate::derivation::dispatch::derive_for_chain(
            stored.chain_id,
            MNEMONIC,
            &expected_path,
            Some("Peercoin passphrase"),
            None,
            None,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        assert_eq!(received, expected);
    }
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
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
        .import_wallets(commit(crate::registry::Chain::Solana))
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
    assert_eq!(stored[0].chain_id, crate::registry::Chain::Solana);
    assert_eq!(
        stored[0].addresses.get("solana").map(String::as_str),
        Some(
            crate::derivation::import::derive_import_address(
                MNEMONIC,
                crate::registry::Chain::Solana,
                &crate::derivation::path::default_path_from_catalog(crate::registry::Chain::Solana)
                    .unwrap(),
                &WalletDerivationOverrides::default()
            )
            .unwrap()
            .as_str()
        )
    );
}

/// A seed import stores the address of the network it names, valid by that
/// network's own validator, and no other network's: a wallet never changes
/// network, so another network's address would be data nothing reads.
#[tokio::test]
async fn a_seed_import_stores_only_its_own_networks_address() {
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
    let mut commit = commit(crate::registry::Chain::Bitcoin);
    commit.seed_phrase = Some(MNEMONIC.to_string());
    service.import_wallets(commit).await.expect("import");

    let stored = service
        .portfolio_snapshot()
        .await
        .expect("snapshot")
        .wallets;
    assert_eq!(stored.len(), 1);
    let addresses = &stored[0].addresses;
    let bitcoin = crate::registry::Chain::Bitcoin;
    let address = addresses
        .get(bitcoin.address_slot())
        .expect("no Bitcoin address");
    assert!(crate::derivation::import::is_valid_watch_only_address(
        bitcoin,
        address.clone()
    ));
    assert_eq!(
        addresses.get(crate::registry::Chain::BitcoinTestnet4.address_slot()),
        None
    );
}

#[tokio::test]
async fn each_wallet_is_on_the_network_its_import_named() {
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
    let mut wallets = Vec::new();
    for chain in [
        crate::registry::Chain::BitcoinTestnet,
        crate::registry::Chain::Solana,
    ] {
        let outcome = service.import_wallets(commit(chain)).await.expect("import");
        assert_eq!(outcome.wallets.len(), 1);
        wallets.extend(outcome.wallets);
    }

    let by_chain: std::collections::HashMap<_, _> = wallets
        .iter()
        .map(|w| (w.chain_id.mainnet_counterpart(), w))
        .collect();
    assert_eq!(
        by_chain[&crate::registry::Chain::Bitcoin].chain_id,
        crate::registry::Chain::BitcoinTestnet
    );
    assert_eq!(
        by_chain[&crate::registry::Chain::Solana].chain_id,
        crate::registry::Chain::Solana
    );
    // Each wallet starts with its own network's native holding, and no other.
    let holdings = |chain: crate::registry::Chain| {
        by_chain[&chain]
            .holdings
            .iter()
            .map(|h| h.chain_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        holdings(crate::registry::Chain::Bitcoin),
        vec![crate::registry::Chain::BitcoinTestnet]
    );
    assert_eq!(
        holdings(crate::registry::Chain::Solana),
        vec![crate::registry::Chain::Solana]
    );
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
        // The device key is minted on the first seal and is not a wallet's.
        if kind != crate::store::secret_store::SecretClass::DeviceKey
            && self
                .writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                == 0
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
    fn wrap_device_key(
        &self,
        key: Vec<u8>,
    ) -> Result<Vec<u8>, crate::store::secret_store::SecretStoreError> {
        self.inner.wrap_device_key(key)
    }
    fn unwrap_device_key(
        &self,
        wrapped: Vec<u8>,
    ) -> Result<Vec<u8>, crate::store::secret_store::SecretStoreError> {
        self.inner.unwrap_device_key(wrapped)
    }
}

#[tokio::test]
async fn a_failed_seal_leaves_neither_a_wallet_nor_its_secret_and_retries() {
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
    let input = commit(crate::registry::Chain::Ethereum);
    assert!(service.import_wallets(input.clone()).await.is_err());
    assert!(service.app_state().await.wallets.is_empty());
    // Only the device key is left, for every later seal to reuse.
    assert_eq!(store.inner.len(), 1);
    assert!(
        crate::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(
            path.to_str().unwrap()
        ))
        .unwrap()
        .wallets
        .is_empty()
    );
    let outcome = service.import_wallets(input).await.unwrap();
    assert_eq!(outcome.wallets.len(), 1);
    for wallet in outcome.wallets {
        assert_eq!(
            wallet.signing,
            crate::store::state::WalletSigning::SeedPhrase {
                password_protected: false
            }
        );
        assert!(matches!(
            service.reveal_seed_phrase(wallet.id, None).unwrap(),
            crate::service::SeedPhraseReveal::Phrase { .. }
        ));
    }
    assert_eq!(
        crate::wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(
            path.to_str().unwrap()
        ))
        .unwrap()
        .wallets
        .len(),
        1
    );
}

/// A blank password is refused as bad input, for a seed and for a private
/// key, and nothing is stored: only `None` means "no password".
#[tokio::test]
async fn a_blank_password_is_refused_rather_than_stored_unsealed() {
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
    for blank in ["", "   "] {
        let mut seed = commit(crate::registry::Chain::Solana);
        seed.password = Some(blank.into());
        let mut key = commit(crate::registry::Chain::Ethereum);
        key.request.kind = WalletImportKind::PrivateKey;
        key.seed_phrase = None;
        key.private_key = Some(format!("{:064x}", 1));
        key.password = Some(blank.into());
        for commit in [seed, key] {
            assert!(matches!(
                service.import_wallets(commit).await,
                Err(crate::SpectraBridgeError::InvalidInput { .. })
            ));
        }
    }
    assert_eq!(store.len(), 0);
    assert!(service.app_state().await.wallets.is_empty());
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
    let mut missing = commit(crate::registry::Chain::Solana);
    missing.seed_phrase = None;
    assert!(service.import_wallets(missing).await.is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON wallets BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(
        service
            .import_wallets(commit(crate::registry::Chain::Ethereum))
            .await
            .is_err()
    );
    // Only the device key is left, for every later seal to reuse.
    assert_eq!(store.len(), 1);
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
    // Each on its own account: one wallet per address.
    let on_account = |account: u32| {
        let mut input = commit(crate::registry::Chain::Solana);
        input.derivation_path = Some(format!("m/44'/501'/{account}'/0'"));
        input
    };
    let mut named = on_account(0);
    named.request.wallet_name = "Wallet 1".into();
    service.import_wallets(named).await.unwrap();
    let (one, two) = tokio::join!(
        service.import_wallets(on_account(1)),
        service.import_wallets(on_account(2))
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
            .import_wallets(on_account(3))
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
    let mut raw = commit(crate::registry::Chain::Ethereum);
    raw.seed_phrase = Some(format!(
        "  {}  ",
        MNEMONIC.to_uppercase().replace(' ', "\t\n")
    ));
    let imported = service.import_wallets(raw).await.unwrap();
    assert_eq!(
        imported.wallets[0].primary_address(),
        Some(derived(crate::registry::Chain::Ethereum).as_str())
    );
    // The canonical phrase is the same wallet, so a second import of it is
    // refused as a duplicate.
    assert!(
        service
            .import_wallets(commit(crate::registry::Chain::Ethereum))
            .await
            .is_err()
    );
    assert_eq!(
        service
            .reveal_seed_phrase(imported.wallets[0].id.clone(), None)
            .unwrap(),
        crate::service::SeedPhraseReveal::Phrase {
            phrase: MNEMONIC.to_string()
        }
    );
}

#[tokio::test]
async fn deep_rescan_reports_provider_failures_and_empty_scope_success() {
    use crate::fetch::refresh_policy::DeviceConditions;
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
        chain_id: crate::registry::Chain::Bitcoin,
    };
    let empty = service
        .refresh_app(intent.clone(), conditions.clone())
        .await
        .unwrap();
    assert!(empty.failures.is_empty());
    service
        .import_wallets(commit(crate::registry::Chain::Bitcoin))
        .await
        .unwrap();
    // No configured providers: all network work must fail locally and remain visible.
    let result = service.refresh_app(intent, conditions).await.unwrap();
    assert!(!result.failures.is_empty());
    assert!(result.state.quotes.prices_attempt_at.is_none());
}

#[tokio::test]
async fn testnet_paths_survive_reopen_and_signing() {
    use crate::registry::Chain;
    use crate::store::secret_backends::InMemorySecretStore;
    use std::sync::Arc;
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let db = temp.join("state.db").to_string_lossy().into_owned();
    let secrets = Arc::new(InMemorySecretStore::new());
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(secrets.clone());
    service.open_state(db.clone()).await.unwrap();
    // Each network's account paths, chosen or its default, survive reopening
    // and sign. A mainnet coin type on a test network names no account of
    // it, and is refused rather than stored as one.
    let mut mainnet_style = commit(Chain::BitcoinSignet);
    mainnet_style.derivation_path = Some("m/84'/0'/9'/0/0".into());
    assert!(service.import_wallets(mainnet_style).await.is_err());
    for (chain, path) in [
        (Chain::BitcoinTestnet4, Some("m/84'/1'/3'/0/0")),
        (Chain::BitcoinSignet, Some("m/84'/1'/9'/0/0")),
        (Chain::BitcoinTestnet, None),
        (Chain::Bitcoin, Some("m/84'/0'/2'/0/0")),
    ] {
        let mut input = commit(chain);
        input.password = Some("test-password".into());
        if let Some(path) = path {
            input.derivation_path = Some(path.into());
        }
        service.import_wallets(input).await.unwrap();
    }
    drop(service);
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(secrets);
    service.open_state(db).await.unwrap();
    for (chain, path) in [
        (Chain::BitcoinTestnet4, "m/84'/1'/3'/0/0"),
        (Chain::BitcoinSignet, "m/84'/1'/9'/0/0"),
        (Chain::BitcoinTestnet, "m/84'/1'/0'/0/0"),
        (Chain::Bitcoin, "m/84'/0'/2'/0/0"),
    ] {
        let state = service.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|wallet| wallet.chain_id == chain)
            .expect("one wallet per imported network");
        assert_eq!(wallet.derivation_path.as_deref(), Some(path));
        let expected = crate::derivation::dispatch::derive_for_chain(
            chain, MNEMONIC, path, None, None, None, true, false, false,
        )
        .unwrap()
        .address
        .unwrap();
        assert_eq!(wallet.address_on(chain), Some(expected.as_str()));
        assert_eq!(
            service
                .send_identity_address(wallet.id.clone(), chain, Some("test-password".into()))
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
        crate::registry::Chain::Bitcoin,
        "bc1main",
        Some("m/84'/0'/0'/0/0".into()),
        false,
    );
    wallet.chain_id = crate::registry::Chain::BitcoinTestnet4;
    assert!(wallet.active_address().is_none());
}

async fn fresh_service(
    secrets: std::sync::Arc<dyn crate::store::secret_store::SecretStore>,
) -> (std::sync::Arc<WalletService>, String) {
    let path = std::env::temp_dir()
        .join(format!(
            "spectra-upgrade-{}.db",
            crate::store::new_transaction_id()
        ))
        .to_string_lossy()
        .into_owned();
    let service = WalletService::new(vec![]).unwrap();
    service.set_secret_store(secrets);
    service.open_state(path.clone()).await.unwrap();
    (service, path)
}

fn watch(chain: crate::registry::Chain, name: &str, addresses: &[&str]) -> WalletImportCommit {
    let mut input = commit(chain);
    input.seed_phrase = None;
    input.request.wallet_name = name.into();
    input.request.kind = WalletImportKind::WatchAddresses {
        addresses: addresses.iter().map(|a| a.to_string()).collect(),
    };
    input
}

fn derived(chain: crate::registry::Chain) -> String {
    let path = crate::derivation::path::default_path_from_catalog(chain).unwrap();
    let address = crate::derivation::dispatch::derive_for_chain(
        chain, MNEMONIC, &path, None, None, None, true, false, false,
    )
    .unwrap()
    .address
    .unwrap();
    crate::derivation::import::normalized_import_address(chain, &address).unwrap()
}

/// A phrase whose address a watch-only wallet holds gives that wallet its
/// keys: the same id, name and settings, now signing, its secret sealed
/// under the id — and it reads back so after reopening.
#[tokio::test]
async fn a_signing_import_upgrades_the_watched_wallet_in_place() {
    use crate::registry::Chain;
    let secrets = std::sync::Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
    let (service, path) = fresh_service(secrets.clone()).await;
    let address = derived(Chain::Ethereum);
    let watched = service
        .import_wallets(watch(Chain::Ethereum, "Cold", &[&address]))
        .await
        .unwrap()
        .wallets
        .remove(0);
    service
        .apply_state_command(
            crate::store::state::StateCommand::SetWalletPortfolioInclusion {
                wallet_id: watched.id.clone(),
                included: false,
            },
        )
        .await
        .unwrap();
    let mut phrase = commit(Chain::Ethereum);
    phrase.request.wallet_name = "Ignored".into();
    let preview = service.preview_wallet_import(phrase.clone()).await.unwrap();
    assert_eq!(preview.upgrades_wallet.as_deref(), Some("Cold"));
    let outcome = service.import_wallets(phrase).await.unwrap();
    assert!(outcome.upgraded);
    let wallet = &outcome.wallets[0];
    assert_eq!(wallet.id, watched.id);
    assert_eq!(wallet.name, "Cold");
    assert!(matches!(
        service.reveal_seed_phrase(wallet.id.clone(), None).unwrap(),
        crate::service::SeedPhraseReveal::Phrase { .. }
    ));
    drop(service);
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.set_secret_store(secrets);
    let state = reopened.open_state(path).await.unwrap();
    assert_eq!(state.wallets.len(), 1);
    let stored = &state.wallets[0];
    assert_eq!(stored.id, watched.id);
    assert!(!stored.include_in_portfolio_total);
    assert_eq!(
        stored.signing,
        crate::store::state::WalletSigning::SeedPhrase {
            password_protected: false
        }
    );
    assert_eq!(stored.derivation_path.as_deref(), Some("m/44'/60'/0'/0/0"));
    assert_eq!(
        reopened
            .send_identity_address(stored.id.clone(), Chain::Ethereum, None)
            .await
            .unwrap(),
        address
    );
}

/// A Bitcoin phrase whose account a watched zpub holds upgrades that wallet
/// into the phrase wallet an import stores, and a watched account is one
/// account whatever its key's version bytes.
#[tokio::test]
async fn a_bitcoin_phrase_upgrades_its_watched_account_key() {
    use crate::registry::Chain;
    let secrets = std::sync::Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
    let (service, _) = fresh_service(secrets).await;
    let path = crate::derivation::path::default_path_from_catalog(Chain::Bitcoin).unwrap();
    let xpub = crate::service::address_discovery::UtxoDerivation::account_xpub(
        Chain::Bitcoin,
        MNEMONIC,
        &path,
        &Default::default(),
    )
    .unwrap();
    // The same account key, written as a zpub.
    let (raw, _) = crate::derivation::bitcoin::ExtendedPublicKey::from_xpub_string(&xpub).unwrap();
    let zpub = raw.to_xpub_string([0x04, 0xb2, 0x47, 0x46]);
    let mut account = commit(Chain::Bitcoin);
    account.seed_phrase = None;
    account.request.kind = WalletImportKind::WatchAccountXpub { xpub: zpub.clone() };
    let watched = service
        .import_wallets(account.clone())
        .await
        .unwrap()
        .wallets
        .remove(0);
    // Watched again, the account is refused, naming the wallet.
    let again = service.import_wallets(account).await.unwrap_err();
    assert!(again.to_string().contains(&watched.name), "{again}");
    let mut as_xpub = commit(Chain::Bitcoin);
    as_xpub.seed_phrase = None;
    as_xpub.request.kind = WalletImportKind::WatchAccountXpub { xpub };
    assert!(service.import_wallets(as_xpub).await.is_err());
    let outcome = service
        .import_wallets(commit(Chain::Bitcoin))
        .await
        .unwrap();
    assert!(outcome.upgraded);
    assert_eq!(outcome.wallets[0].id, watched.id);
    assert_eq!(
        outcome.wallets[0].primary_address(),
        Some(derived(Chain::Bitcoin).as_str())
    );
    assert_eq!(service.app_state().await.wallets.len(), 1);
}

/// Every other duplicate is refused and names the wallet that holds it; a
/// multi-line watch reports the held lines and imports the rest.
#[tokio::test]
async fn other_duplicates_are_refused_naming_the_wallet() {
    use crate::registry::Chain;
    let secrets = std::sync::Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
    let (service, _) = fresh_service(secrets).await;
    let mut signing = commit(Chain::Ethereum);
    signing.request.wallet_name = "Hot".into();
    service.import_wallets(signing.clone()).await.unwrap();
    let held = derived(Chain::Ethereum);
    for refused in [
        service.import_wallets(signing).await.unwrap_err(),
        service
            .import_wallets(watch(Chain::Ethereum, "Again", &[&held]))
            .await
            .unwrap_err(),
    ] {
        assert!(refused.to_string().contains("Hot"), "{refused}");
    }
    let other = "0x000000000000000000000000000000000000dead";
    let outcome = service
        .import_wallets(watch(Chain::Ethereum, "Mixed", &[&held, other, other]))
        .await
        .unwrap();
    assert_eq!(outcome.wallets.len(), 1);
    assert_eq!(outcome.wallets[0].name, "Mixed");
    assert_eq!(
        outcome.rejected_addresses,
        [held.clone(), other.to_string()]
    );
    // The same address on another network is another wallet.
    service
        .import_wallets(watch(Chain::Base, "Base", &[&held]))
        .await
        .unwrap();
    assert_eq!(service.app_state().await.wallets.len(), 3);
}

/// The seal and the change commit together: a seal that fails leaves the
/// watched wallet watch-only with no secret, and a retry upgrades it.
#[tokio::test]
async fn a_failed_upgrade_leaves_the_watched_wallet_as_it_was() {
    use crate::registry::Chain;
    let store = std::sync::Arc::new(FailingSecrets::default());
    let (service, _) = fresh_service(store.clone()).await;
    let watched = service
        .import_wallets(watch(Chain::Ethereum, "Cold", &[&derived(Chain::Ethereum)]))
        .await
        .unwrap()
        .wallets
        .remove(0);
    assert!(
        service
            .import_wallets(commit(Chain::Ethereum))
            .await
            .is_err()
    );
    let state = service.app_state().await;
    assert_eq!(state.wallets.len(), 1);
    assert!(state.wallets[0].is_watch_only());
    assert!(!matches!(
        service.reveal_seed_phrase(watched.id.clone(), None),
        Ok(crate::service::SeedPhraseReveal::Phrase { .. })
    ));
    let outcome = service
        .import_wallets(commit(Chain::Ethereum))
        .await
        .unwrap();
    assert!(outcome.upgraded && outcome.wallets[0].id == watched.id);
}

/// A NEAR key import may hold a named account: the preview shows it without
/// contacting the network, the import stores it only once the network lists
/// the key among the account's full-access keys, and a send signs from it.
/// A function-call key, an unknown account, another chain and a malformed
/// name are refused before anything is stored.
#[tokio::test]
async fn a_near_named_account_is_held_once_the_network_lists_the_key() {
    use crate::registry::Chain;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};
    let public_hex = derived(Chain::Near);
    let public_key = format!(
        "ed25519:{}",
        bs58::encode(hex::decode(&public_hex).unwrap()).into_string()
    );
    let server = MockServer::start().await;
    let expected_key = public_key.clone();
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let id = body["id"].clone();
            let reply = |result: serde_json::Value| {
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"jsonrpc":"2.0","id":id,"result":result}))
            };
            match body["method"].as_str().unwrap() {
                "status" => reply(serde_json::json!({"chain_id": "mainnet"})),
                "query" => {
                    let params = &body["params"];
                    let ours = params["public_key"] == expected_key.as_str();
                    match params["account_id"].as_str().unwrap() {
                        "alice.near" if ours => {
                            reply(serde_json::json!({"nonce": 5, "permission": "FullAccess"}))
                        }
                        "bob.near" if ours => reply(serde_json::json!({
                            "nonce": 5,
                            "permission": {"FunctionCall": {"allowance": null,
                                "receiver_id": "app.near", "method_names": []}}
                        })),
                        _ => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                            "jsonrpc": "2.0", "id": id,
                            "error": {"code": -32000, "message": "Server error",
                                "data": "access key does not exist while viewing"}
                        })),
                    }
                }
                other => panic!("unexpected NEAR call {other}"),
            }
        })
        .mount(&server)
        .await;
    let service = WalletService::new(vec![crate::service::ChainEndpoints {
        capabilities: crate::EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Near,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    let path = std::env::temp_dir()
        .join(format!(
            "spectra-named-{}.db",
            crate::store::new_transaction_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(path).await.unwrap();
    let named = |name: &str| {
        let mut input = commit(Chain::Near);
        input.named_account = Some(name.into());
        input
    };
    let preview = service
        .preview_wallet_import(named(" Alice.NEAR "))
        .await
        .unwrap();
    assert_eq!(preview.addresses, ["alice.near"]);
    assert!(server.received_requests().await.unwrap().is_empty());
    for (name, refusal) in [
        ("bob.near", "bob.near"),
        ("carol.near", "carol.near"),
        (&public_hex[..], "Not a NEAR named account"),
        ("-bad-", "Not a NEAR named account"),
    ] {
        let error = service.import_wallets(named(name)).await.unwrap_err();
        assert!(error.to_string().contains(refusal), "{name}: {error}");
    }
    let mut elsewhere = commit(Chain::Ethereum);
    elsewhere.named_account = Some("alice.near".into());
    assert!(service.import_wallets(elsewhere).await.is_err());
    assert!(service.app_state().await.wallets.is_empty());
    let wallet = service
        .import_wallets(named("alice.near"))
        .await
        .unwrap()
        .wallets
        .remove(0);
    assert_eq!(wallet.primary_address(), Some("alice.near"));
    assert_eq!(
        service
            .send_identity_address(wallet.id, Chain::Near, None)
            .await
            .unwrap(),
        "alice.near"
    );
}
