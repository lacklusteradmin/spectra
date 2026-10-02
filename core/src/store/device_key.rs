//! The device key: the outer seal on every stored seed phrase and private key.
//!
//! Signing material is sealed in a [`seed_envelope`](super::seed_envelope)
//! under this 32-byte key before it reaches the platform store, whether or not
//! a wallet password also seals it. The key is minted here and reaches the
//! store only as [`SecretStore::wrap_device_key`] returned it: on iOS that is
//! encrypted to a Secure Enclave key, so a copied Keychain opens nothing. A
//! backend with nothing stronger returns the key unchanged, which leaves it
//! beside what it seals; the file store's own docs say what that protects.
//!
//! The rules here were the iOS shell's, and every platform needs the same
//! ones:
//!
//! - A key that cannot be read is not an absent key. Answering "absent" would
//!   mint a new one and replace the key every stored seed is sealed under.
//! - A key is minted only when the store reports none, and one process mints
//!   at most one: two first seals racing would otherwise each write a key and
//!   strand whichever seed was sealed under the loser.
//! - Failing to seal is failing to store. Nothing falls back to plaintext.

use base64::Engine as _;
use rand::RngCore as _;
use zeroize::Zeroizing;

use super::secret_store::{SecretClass, SecretStore, SecretStoreError};
use super::seed_envelope::{self, MASTER_KEY_LEN};
use super::wallet_secrets::WalletSecretError;

/// Where the wrapped key is stored, in [`SecretClass::DeviceKey`]'s bucket.
const ACCOUNT: &str = "signing-material";

/// Held from reading the key to writing a new one.
static MINTING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn unavailable(detail: impl std::fmt::Display) -> WalletSecretError {
    WalletSecretError::Backend {
        message: format!("the device key is unavailable: {detail}"),
    }
}

/// The stored key, unwrapped, or `None` only when the store holds none.
fn stored(store: &dyn SecretStore) -> Result<Option<Zeroizing<Vec<u8>>>, WalletSecretError> {
    let encoded = match store.load_secret(SecretClass::DeviceKey, ACCOUNT.into()) {
        Ok(encoded) => encoded,
        Err(SecretStoreError::NotFound) => return Ok(None),
        Err(error) => return Err(unavailable(error)),
    };
    let wrapped = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| unavailable(format!("the stored key is not base64: {e}")))?;
    let key = Zeroizing::new(store.unwrap_device_key(wrapped).map_err(unavailable)?);
    // Present but the wrong shape is still present: refused, never replaced.
    if key.len() != MASTER_KEY_LEN {
        return Err(unavailable(format!(
            "the stored key is {} bytes, not {MASTER_KEY_LEN}",
            key.len()
        )));
    }
    Ok(Some(key))
}

/// The key to seal with, minted and stored on first use. A key that was not
/// stored is never returned: a seed sealed under it could not be opened after
/// a relaunch.
fn for_sealing(store: &dyn SecretStore) -> Result<Zeroizing<Vec<u8>>, WalletSecretError> {
    let _minting = MINTING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(key) = stored(store)? {
        return Ok(key);
    }
    let mut key = Zeroizing::new(vec![0u8; MASTER_KEY_LEN]);
    rand::rngs::OsRng
        .try_fill_bytes(&mut key)
        .map_err(|e| unavailable(format!("the OS generator failed: {e}")))?;
    let wrapped = store.wrap_device_key(key.to_vec()).map_err(unavailable)?;
    store
        .save_secret(
            SecretClass::DeviceKey,
            ACCOUNT.into(),
            base64::engine::general_purpose::STANDARD.encode(wrapped),
        )
        .map_err(unavailable)?;
    Ok(key)
}

/// Seal `material` for the store.
pub(super) fn seal(store: &dyn SecretStore, material: &[u8]) -> Result<String, WalletSecretError> {
    let key = for_sealing(store)?;
    let envelope =
        seed_envelope::encrypt(material, &key).map_err(|e| WalletSecretError::Backend {
            message: format!("signing material could not be sealed: {e}"),
        })?;
    Ok(String::from_utf8(envelope).expect("an envelope is JSON"))
}

/// Open what [`seal`] stored. Never mints a key: sealed material with no key
/// to open it is unreadable, not absent.
pub(super) fn open(
    store: &dyn SecretStore,
    sealed: &str,
) -> Result<Zeroizing<String>, WalletSecretError> {
    let key = stored(store)?.ok_or_else(|| unavailable("none is stored for sealed material"))?;
    seed_envelope::decrypt(sealed.as_bytes(), &key)
        .map(Zeroizing::new)
        .map_err(|e| WalletSecretError::Backend {
            message: format!("signing material could not be opened: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::secret_backends::InMemorySecretStore;

    /// The key is stored only as the backend wrapped it, and reused after.
    #[test]
    fn the_key_is_minted_once_and_stored_wrapped() {
        struct Xor(InMemorySecretStore);
        impl SecretStore for Xor {
            fn load_secret(&self, k: SecretClass, key: String) -> Result<String, SecretStoreError> {
                self.0.load_secret(k, key)
            }
            fn save_secret(
                &self,
                k: SecretClass,
                key: String,
                v: String,
            ) -> Result<(), SecretStoreError> {
                self.0.save_secret(k, key, v)
            }
            fn delete_secret(&self, k: SecretClass, key: String) -> Result<(), SecretStoreError> {
                self.0.delete_secret(k, key)
            }
            fn wrap_device_key(&self, key: Vec<u8>) -> Result<Vec<u8>, SecretStoreError> {
                Ok(key.iter().map(|b| b ^ 0x5a).collect())
            }
            fn unwrap_device_key(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, SecretStoreError> {
                self.wrap_device_key(wrapped)
            }
        }
        let store = Xor(InMemorySecretStore::new());
        let first = seal(&store, b"secret").unwrap();
        let wrapped = store
            .load_secret(SecretClass::DeviceKey, ACCOUNT.into())
            .unwrap();
        let second = seal(&store, b"secret").unwrap();
        assert_eq!(
            store
                .load_secret(SecretClass::DeviceKey, ACCOUNT.into())
                .unwrap(),
            wrapped,
            "a second seal reuses the key"
        );
        let key = stored(&store).unwrap().unwrap();
        assert_ne!(
            base64::engine::general_purpose::STANDARD
                .decode(&wrapped)
                .unwrap(),
            *key,
            "the raw key is never stored"
        );
        assert_eq!(&*open(&store, &first).unwrap(), "secret");
        assert_eq!(&*open(&store, &second).unwrap(), "secret");
    }

    /// A key that cannot be read or unwrapped is an error, and is left in
    /// place rather than replaced by a fresh one.
    #[test]
    fn an_unreadable_key_is_never_replaced() {
        struct Locked(InMemorySecretStore);
        impl SecretStore for Locked {
            fn load_secret(&self, k: SecretClass, key: String) -> Result<String, SecretStoreError> {
                self.0.load_secret(k, key)
            }
            fn save_secret(
                &self,
                k: SecretClass,
                key: String,
                v: String,
            ) -> Result<(), SecretStoreError> {
                self.0.save_secret(k, key, v)
            }
            fn delete_secret(&self, k: SecretClass, key: String) -> Result<(), SecretStoreError> {
                self.0.delete_secret(k, key)
            }
            fn wrap_device_key(&self, key: Vec<u8>) -> Result<Vec<u8>, SecretStoreError> {
                Ok(key)
            }
            fn unwrap_device_key(&self, _: Vec<u8>) -> Result<Vec<u8>, SecretStoreError> {
                Err(SecretStoreError::Backend {
                    message: "the enclave refused".into(),
                })
            }
        }
        let store = Locked(InMemorySecretStore::new());
        let wrapped = base64::engine::general_purpose::STANDARD.encode([7u8; MASTER_KEY_LEN]);
        store
            .save_secret(SecretClass::DeviceKey, ACCOUNT.into(), wrapped.clone())
            .unwrap();
        assert!(matches!(
            seal(&store, b"secret"),
            Err(WalletSecretError::Backend { .. })
        ));
        assert_eq!(
            store
                .load_secret(SecretClass::DeviceKey, ACCOUNT.into())
                .unwrap(),
            wrapped
        );

        // A key of the wrong length is present, not absent.
        let short = InMemorySecretStore::new();
        short
            .save_secret(
                SecretClass::DeviceKey,
                ACCOUNT.into(),
                base64::engine::general_purpose::STANDARD.encode([7u8; 16]),
            )
            .unwrap();
        assert!(matches!(
            seal(&short, b"secret"),
            Err(WalletSecretError::Backend { .. })
        ));
    }

    /// Opening never mints: with no key stored, sealed material is unreadable.
    #[test]
    fn opening_without_a_key_does_not_mint_one() {
        let sealing = InMemorySecretStore::new();
        let sealed = seal(&sealing, b"secret").unwrap();
        let empty = InMemorySecretStore::new();
        assert!(matches!(
            open(&empty, &sealed),
            Err(WalletSecretError::Backend { .. })
        ));
        assert!(empty.is_empty());
    }
}
