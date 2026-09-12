use crate::derivation::import::{WalletImportAddresses, WalletImportCommit, WalletImportRequest};
use crate::service::WalletService;
use crate::store::wallet_domain::{CoreSeedDerivationPreset, CoreWalletDerivationOverrides};

// Real addresses: core validates every import address now, so a
// placeholder would simply be dropped.
const BTC: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
const SOL: &str = "11111111111111111111111111111111";
const MNEMONIC: &str = "test test test test test test test test test test test junk";

fn commit(chains: &[&str], addresses: &[(&str, &str)]) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
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
        seed_derivation_paths: crate::app_core::seed_derivation_paths_for_account(0).unwrap(),
        derivation_overrides: CoreWalletDerivationOverrides::default(),
        network_chain_by_family: Default::default(),
        seed_phrase: Some(MNEMONIC.into()),
        private_key: None,
    }
}

#[tokio::test]
async fn imported_wallets_land_in_core_state() {
    let temp = std::env::temp_dir().join(crate::store::new_transaction_id());
    std::fs::create_dir_all(&temp).unwrap();
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(temp.join("state.db").to_string_lossy().into())
        .await
        .unwrap();
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
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(temp.join("state.db").to_string_lossy().into())
        .await
        .unwrap();
    let mut commit = commit(&["Bitcoin"], &[]);
    commit.seed_phrase = Some(MNEMONIC.to_string());
    commit.seed_derivation_paths =
        crate::app_core::seed_derivation_paths_for_account(0).expect("default paths");
    service.import_wallets(commit).await.expect("import");

    let stored = service.wallets_for_display().await.expect("wallets");
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
    let service = WalletService::new_typed(Vec::new()).expect("service");
    service.set_secret_store(std::sync::Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(temp.join("state.db").to_string_lossy().into())
        .await
        .unwrap();
    let mut input = commit(&["Bitcoin", "Solana"], &[("bitcoin", BTC), ("solana", SOL)]);
    input.network_chain_by_family =
        std::collections::HashMap::from([("bitcoin".to_string(), "bitcoin-testnet".to_string())]);
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
    fn list_keys(
        &self,
        kind: crate::store::secret_store::SecretClass,
        prefix: String,
    ) -> Result<Vec<String>, crate::store::secret_store::SecretStoreError> {
        self.inner.list_keys(kind, prefix)
    }
}

#[tokio::test]
async fn failed_multi_wallet_import_leaves_neither_wallets_nor_partial_secrets_and_retries() {
    let path = std::env::temp_dir().join(format!(
        "spectra-import-{}.db",
        crate::store::new_transaction_id()
    ));
    let service = WalletService::new_typed(vec![]).unwrap();
    let store = std::sync::Arc::new(FailingSecrets::default());
    service.set_secret_store(store.clone());
    service
        .open_state(path.to_string_lossy().into())
        .await
        .unwrap();
    let input = commit(&["Ethereum", "Solana"], &[]);
    assert!(service.import_wallets(input.clone()).await.is_err());
    assert!(service.app_state().await.wallets.is_empty());
    assert_eq!(store.inner.len(), 0);
    assert!(crate::wallet_db::app_state_load(path.to_str().unwrap())
        .unwrap()
        .wallets
        .is_empty());
    let outcome = service.import_wallets(input).await.unwrap();
    assert_eq!(outcome.wallets.len(), 2);
    for wallet in outcome.wallets {
        assert!(service.wallet_secret_state(wallet.id).has_signing_material);
    }
    assert_eq!(
        crate::wallet_db::app_state_load(path.to_str().unwrap())
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
    let service = WalletService::new_typed(vec![]).unwrap();
    let store = std::sync::Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
    service.set_secret_store(store.clone());
    service
        .open_state(path.to_string_lossy().into())
        .await
        .unwrap();
    let mut missing = commit(&["Solana"], &[("solana", SOL)]);
    missing.seed_phrase = None;
    assert!(service.import_wallets(missing).await.is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON wallets BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(service
        .import_wallets(commit(&["Ethereum", "Solana"], &[]))
        .await
        .is_err());
    assert_eq!(store.len(), 0);
    assert!(service.app_state().await.wallets.is_empty());
}
