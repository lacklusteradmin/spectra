use crate::derivation::import::{WalletImportCommit, WalletImportRequest};
use crate::service::WalletService;
use crate::store::wallet_domain::{CoreSeedDerivationPreset, CoreWalletDerivationOverrides};

const MNEMONIC: &str = "test test test test test test test test test test test junk";

fn commit(chains: &[crate::registry::Chain]) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request: WalletImportRequest {
            wallet_name: String::new(),
            selected_chain_ids: chains.to_vec(),
            is_watch_only_import: false,
            is_private_key_import: false,
            watch_only_entries: Default::default(),
        },
        seed_derivation_preset: CoreSeedDerivationPreset::Standard,
        seed_derivation_paths: crate::derivation::path::seed_derivation_paths_for_account(0)
            .unwrap(),
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
        .import_wallets(commit(&[crate::registry::Chain::Solana]))
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
            crate::derivation::import::derive_import_addresses(
                MNEMONIC,
                &[crate::registry::Chain::Solana],
                &crate::derivation::path::seed_derivation_paths_for_account(0).unwrap(),
                &CoreWalletDerivationOverrides::default()
            )[&crate::registry::Chain::Solana]
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
    let mut commit = commit(&[crate::registry::Chain::Bitcoin]);
    commit.seed_phrase = Some(MNEMONIC.to_string());
    commit.seed_derivation_paths =
        crate::derivation::path::seed_derivation_paths_for_account(0).expect("default paths");
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
    let outcome = service
        .import_wallets(commit(&[
            crate::registry::Chain::BitcoinTestnet,
            crate::registry::Chain::Solana,
        ]))
        .await
        .expect("import");

    let by_chain: std::collections::HashMap<_, _> = outcome
        .wallets
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
    let input = commit(&[
        crate::registry::Chain::Ethereum,
        crate::registry::Chain::Solana,
    ]);
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
    assert_eq!(outcome.wallets.len(), 2);
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
        2
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
        let mut seed = commit(&[crate::registry::Chain::Solana]);
        seed.password = Some(blank.into());
        let mut key = commit(&[crate::registry::Chain::Ethereum]);
        key.request.is_private_key_import = true;
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
    let mut missing = commit(&[crate::registry::Chain::Solana]);
    missing.seed_phrase = None;
    assert!(service.import_wallets(missing).await.is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON wallets BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(
        service
            .import_wallets(commit(&[
                crate::registry::Chain::Ethereum,
                crate::registry::Chain::Solana
            ]))
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
    let mut named = commit(&[crate::registry::Chain::Solana]);
    named.request.wallet_name = "Wallet 1".into();
    service.import_wallets(named).await.unwrap();
    let (one, two) = tokio::join!(
        service.import_wallets(commit(&[crate::registry::Chain::Solana])),
        service.import_wallets(commit(&[crate::registry::Chain::Solana]))
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
            .import_wallets(commit(&[crate::registry::Chain::Solana]))
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
    let mut raw = commit(&[crate::registry::Chain::Ethereum]);
    raw.seed_phrase = Some(format!(
        "  {}  ",
        MNEMONIC.to_uppercase().replace(' ', "\t\n")
    ));
    let imported = service.import_wallets(raw).await.unwrap();
    let normal = service
        .import_wallets(commit(&[crate::registry::Chain::Ethereum]))
        .await
        .unwrap();
    assert_eq!(imported.wallets[0].addresses, normal.wallets[0].addresses);
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
        .import_wallets(commit(&[crate::registry::Chain::Bitcoin]))
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
    let mut input = commit(&[
        Chain::BitcoinTestnet4,
        Chain::BitcoinSignet,
        Chain::BitcoinTestnet,
        Chain::Bitcoin,
    ]);
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
    service.import_wallets(input).await.unwrap();
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
