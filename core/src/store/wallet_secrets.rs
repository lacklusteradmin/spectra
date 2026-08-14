//! The *policy* binding [`seed_envelope`](super::seed_envelope)'s cipher to
//! [`password_verifier`](super::password_verifier)'s check: the KDF and its
//! cost, the three blobs a sealed wallet consists of, and their keys.
//!
//! It belongs in core because both front ends must agree on it — it used to
//! live in `cli/src/main.rs`, where a front end could have changed the PBKDF2
//! cost with nothing to disagree. The *backend* stays platform-owned: Keychain
//! on iOS, files for the CLI.

use base64::Engine as _;
use rand::RngCore;
use zeroize::Zeroizing;

use super::secret_store::{SecretClass, SecretStore, SecretStoreError};

/// PBKDF2-HMAC-SHA256 iteration count for the password → master-key step.
///
/// Stated once, here. Raising it is a format change: every sealed wallet was
/// written with the value in force at the time, and nothing records which,
/// so a change makes existing envelopes undecryptable.
pub const PBKDF2_ITERATIONS: u32 = 210_000;

/// Salt length in bytes for the master-key derivation.
const SALT_LEN: usize = 16;

/// Deliberately not a `uniffi::Error`: nothing crosses the FFI for this.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WalletSecretError {
    /// No sealed material is stored for this wallet — a watch-only wallet, or
    /// one whose secrets were deleted.
    #[error("no sealed secret for this wallet")]
    NotSealed,
    /// The password did not match the stored verifier.
    #[error("incorrect password")]
    IncorrectPassword,
    /// Something is stored, but it is not what this module writes.
    #[error("stored secret is corrupt: {message}")]
    Corrupt { message: String },
    /// The platform store itself failed.
    #[error("secret store failure: {message}")]
    Backend { message: String },
}

impl From<SecretStoreError> for WalletSecretError {
    fn from(error: SecretStoreError) -> Self {
        match error {
            SecretStoreError::NotFound => Self::NotSealed,
            other => Self::Backend {
                message: other.to_string(),
            },
        }
    }
}

/// Split by bucket so the seed sits in the platform's strongest one while the
/// salt and verifier — neither secret alone — sit in the generic one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blob {
    /// The AES-GCM envelope holding the seed phrase.
    Envelope,
    /// The salt the master key was derived from.
    Salt,
    /// The password verifier, for checking a password without decrypting.
    Verifier,
}

impl Blob {
    fn class(self) -> SecretClass {
        match self {
            Blob::Envelope => SecretClass::Seed,
            Blob::Salt | Blob::Verifier => SecretClass::Generic,
        }
    }

    /// Part of the on-disk layout: frozen.
    fn suffix(self) -> &'static str {
        match self {
            Blob::Envelope => "seed",
            Blob::Salt => "salt",
            Blob::Verifier => "password",
        }
    }

    fn key(self, wallet_id: &str) -> String {
        format!("{wallet_id}.{}", self.suffix())
    }

    const ALL: [Blob; 3] = [Blob::Envelope, Blob::Salt, Blob::Verifier];
}

fn engine() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn write_blob(
    store: &dyn SecretStore,
    wallet_id: &str,
    blob: Blob,
    value: &[u8],
) -> Result<(), WalletSecretError> {
    store
        .save_secret(blob.class(), blob.key(wallet_id), engine().encode(value))
        .map_err(WalletSecretError::from)
}

fn read_blob(
    store: &dyn SecretStore,
    wallet_id: &str,
    blob: Blob,
) -> Result<Vec<u8>, WalletSecretError> {
    let raw = store.load_secret(blob.class(), blob.key(wallet_id))?;
    engine()
        .decode(raw.trim())
        .map_err(|e| WalletSecretError::Corrupt {
            message: format!("{} is not base64: {e}", blob.suffix()),
        })
}

/// `Zeroizing` so the key is wiped rather than left in the caller's frame.
fn derive_master_key(password: &str, salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0u8; 32]);
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(
        password.trim().as_bytes(),
        salt,
        PBKDF2_ITERATIONS,
        &mut *key,
    );
    key
}

/// Replaces anything already stored for `wallet_id`.
pub fn seal(
    store: &dyn SecretStore,
    wallet_id: &str,
    seed_phrase: &str,
    password: &str,
) -> Result<(), WalletSecretError> {
    if password.trim().is_empty() {
        return Err(WalletSecretError::Corrupt {
            message: "password cannot be empty".to_string(),
        });
    }

    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    let master_key = derive_master_key(password, &salt);

    let envelope = super::seed_envelope::encrypt(seed_phrase.as_bytes(), &*master_key)
        .map_err(|message| WalletSecretError::Corrupt { message })?;
    let verifier = super::password_verifier::create_verifier(password)
        .map_err(|message| WalletSecretError::Corrupt { message })?;

    // Envelope last: a failure part-way through leaves no seed blob paired
    // with a salt it was not derived from.
    write_blob(store, wallet_id, Blob::Salt, &salt)?;
    write_blob(store, wallet_id, Blob::Verifier, &verifier)?;
    write_blob(store, wallet_id, Blob::Envelope, &envelope)?;
    Ok(())
}

/// The verifier is checked first, so a wrong password is reported as such
/// rather than as an AES-GCM tag mismatch.
pub fn unlock(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: &str,
) -> Result<Zeroizing<String>, WalletSecretError> {
    let verifier = read_blob(store, wallet_id, Blob::Verifier)?;
    if !super::password_verifier::verify(password, &verifier) {
        return Err(WalletSecretError::IncorrectPassword);
    }
    let salt = read_blob(store, wallet_id, Blob::Salt)?;
    let envelope = read_blob(store, wallet_id, Blob::Envelope)?;
    let master_key = derive_master_key(password, &salt);
    super::seed_envelope::decrypt(&envelope, &*master_key)
        .map(Zeroizing::new)
        .map_err(|message| WalletSecretError::Corrupt { message })
}

/// Idempotent, like the underlying store.
pub fn delete(store: &dyn SecretStore, wallet_id: &str) -> Result<(), WalletSecretError> {
    for blob in Blob::ALL {
        store.delete_secret(blob.class(), blob.key(wallet_id))?;
    }
    Ok(())
}

/// Answers off the envelope alone: the other two are useless without it.
pub fn is_sealed(store: &dyn SecretStore, wallet_id: &str) -> bool {
    store
        .load_secret(Blob::Envelope.class(), Blob::Envelope.key(wallet_id))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::secret_backends::InMemorySecretStore;

    const PHRASE: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn seals_and_unlocks_with_the_right_password() {
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        assert_eq!(*unlock(&store, "W1", "hunter2").unwrap(), PHRASE);
    }

    #[test]
    fn a_wrong_password_is_reported_as_such() {
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        assert_eq!(
            unlock(&store, "W1", "hunter3").unwrap_err(),
            WalletSecretError::IncorrectPassword
        );
    }

    #[test]
    fn an_unsealed_wallet_is_not_a_password_failure() {
        let store = InMemorySecretStore::new();
        assert_eq!(
            unlock(&store, "missing", "hunter2").unwrap_err(),
            WalletSecretError::NotSealed
        );
        assert!(!is_sealed(&store, "missing"));
    }

    #[test]
    fn each_wallet_gets_its_own_salt() {
        // Two wallets sealed with the same phrase and password must not
        // produce the same envelope — otherwise the salt is not doing its job
        // and one cracked password would open both.
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        seal(&store, "W2", PHRASE, "hunter2").unwrap();
        let first = store
            .load_secret(SecretClass::Seed, "W1.seed".to_string())
            .unwrap();
        let second = store
            .load_secret(SecretClass::Seed, "W2.seed".to_string())
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(*unlock(&store, "W2", "hunter2").unwrap(), PHRASE);
    }

    #[test]
    fn resealing_replaces_every_blob() {
        // A reseal that left the old salt behind would derive the master key
        // from one salt and decrypt an envelope sealed under another.
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "first").unwrap();
        seal(&store, "W1", PHRASE, "second").unwrap();
        assert_eq!(*unlock(&store, "W1", "second").unwrap(), PHRASE);
        assert_eq!(
            unlock(&store, "W1", "first").unwrap_err(),
            WalletSecretError::IncorrectPassword
        );
    }

    #[test]
    fn delete_removes_all_three_blobs() {
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        delete(&store, "W1").unwrap();
        assert!(!is_sealed(&store, "W1"));
        for blob in Blob::ALL {
            assert!(store.load_secret(blob.class(), blob.key("W1")).is_err());
        }
    }

    #[test]
    fn an_empty_password_is_refused() {
        let store = InMemorySecretStore::new();
        assert!(seal(&store, "W1", PHRASE, "   ").is_err());
        assert!(!is_sealed(&store, "W1"));
    }

    #[test]
    fn a_corrupt_blob_is_not_reported_as_a_wrong_password() {
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        store
            .save_secret(SecretClass::Generic, "W1.salt".to_string(), "!!!".into())
            .unwrap();
        assert!(matches!(
            unlock(&store, "W1", "hunter2").unwrap_err(),
            WalletSecretError::Corrupt { .. }
        ));
    }
}
