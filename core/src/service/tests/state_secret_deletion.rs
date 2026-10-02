use super::*;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::secret_store::{SecretClass, SecretStoreError};
use std::sync::atomic::{AtomicBool, Ordering};

const MNEMONIC: &str = "test test test test test test test test test test test junk";

#[tokio::test]
async fn deleting_stored_secrets_requires_a_durable_state_binding() {
    let service = WalletService::new(vec![]).unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    crate::store::wallet_secrets::store_seed_phrase(&*secrets, "w", MNEMONIC, None).unwrap();
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: crate::store::state::WalletState::single_address(
                "w",
                "W",
                crate::registry::Chain::Ethereum,
                "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                None,
                false,
            ),
        })
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
        &*crate::store::wallet_secrets::load_seed_phrase(&*secrets, "w", None).unwrap(),
        MNEMONIC
    );
}

fn path() -> String {
    std::env::temp_dir()
        .join(format!(
            "delete-secrets-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned()
}

async fn imported_wallet(service: &WalletService, password: Option<&str>) -> String {
    let commit = crate::derivation::import::WalletImportCommit {
        password: password.map(str::to_owned),
        request: crate::derivation::import::WalletImportRequest {
            wallet_name: String::new(),
            selected_chain_ids: vec![crate::registry::Chain::Ethereum],
            is_watch_only_import: false,
            is_private_key_import: false,
            watch_only_entries: Default::default(),
        },
        seed_derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
        seed_derivation_paths: Default::default(),
        derivation_overrides: Default::default(),
        seed_phrase: Some(MNEMONIC.into()),
        private_key: None,
    };
    service.import_wallets(commit).await.unwrap().wallets[0]
        .id
        .clone()
}

fn sql(path: &str, statement: &str) {
    rusqlite::Connection::open(path)
        .unwrap()
        .execute_batch(statement)
        .unwrap();
}

#[tokio::test]
async fn failed_database_deletion_keeps_the_wallet_and_its_signing_material() {
    let service = WalletService::new(vec![]).unwrap();
    let path = path();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    service.open_state(path.clone()).await.unwrap();
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
        reopened
            .open_state(path.clone())
            .await
            .unwrap()
            .wallets
            .len(),
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
                message: "injected cleanup failure".into(),
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
    service.open_state(path.clone()).await.unwrap();
    let id = imported_wallet(&service, Some("test password")).await;
    secrets.fail.store(true, Ordering::SeqCst);
    let wallet = service.app_state().await.wallets[0].clone();
    let transition = service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: id.clone(),
        })
        .await
        .unwrap();
    assert!(transition.state.wallets.is_empty());
    assert!(transition.state.diagnostics.logs.iter().any(|log| {
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
    assert!(
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .is_err()
    );
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.set_secret_store(secrets.clone());
    assert!(
        reopened
            .open_state(path.clone())
            .await
            .unwrap()
            .wallets
            .is_empty()
    );
    secrets.fail.store(false, Ordering::SeqCst);
    assert!(reopened.open_state(path).await.unwrap().wallets.is_empty());
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
    service.open_state(path.clone()).await.unwrap();
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
    assert!(reopened.open_state(path).await.unwrap().wallets.is_empty());
    assert!(
        crate::wallet_db::pending_secret_deletions(&database)
            .unwrap()
            .is_empty()
    );
}
