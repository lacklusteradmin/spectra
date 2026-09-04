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
    /// The wallet is sealed and no password was supplied.
    #[error("this wallet is sealed and needs its password")]
    PasswordRequired,
    /// A password was supplied for a wallet that is not sealed. Reported
    /// rather than ignored: the caller believes it is unlocking something.
    #[error("this wallet is not sealed and takes no password")]
    PasswordNotRequired,
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
    /// The seed phrase. Sealed wallets hold an AES-GCM envelope here;
    /// unsealed ones hold the phrase itself. Which it is is answered by
    /// [`Blob::Verifier`]'s presence, never by looking at this value — key
    /// material is not something to identify by shape.
    Seed,
    /// A raw private key, for a wallet imported from one instead of from a
    /// phrase. Same two states as [`Blob::Seed`].
    PrivateKey,
    /// The salt the master key was derived from.
    Salt,
    /// The password verifier, for checking a password without decrypting.
    Verifier,
}

impl Blob {
    fn class(self) -> SecretClass {
        match self {
            Blob::Seed => SecretClass::Seed,
            Blob::PrivateKey => SecretClass::PrivateKey,
            Blob::Salt | Blob::Verifier => SecretClass::Generic,
        }
    }

    /// Part of the on-disk layout: frozen.
    fn suffix(self) -> &'static str {
        match self {
            Blob::Seed => "seed",
            Blob::PrivateKey => "privatekey",
            Blob::Salt => "salt",
            Blob::Verifier => "password",
        }
    }

    fn key(self, wallet_id: &str) -> String {
        format!("{wallet_id}.{}", self.suffix())
    }

    const ALL: [Blob; 4] = [Blob::Seed, Blob::PrivateKey, Blob::Salt, Blob::Verifier];
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
    write_blob(store, wallet_id, Blob::Seed, &envelope)?;
    Ok(())
}

/// Seal a raw private key for a wallet imported from one.
///
/// The same envelope, salt and verifier as [`seal`] — a private-key wallet
/// simply has no phrase to store. Sealing a wallet twice replaces whichever
/// blob it had, so a wallet is one or the other and never both.
pub fn seal_private_key(
    store: &dyn SecretStore,
    wallet_id: &str,
    private_key_hex: &str,
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
    let envelope = super::seed_envelope::encrypt(private_key_hex.as_bytes(), &*master_key)
        .map_err(|message| WalletSecretError::Corrupt { message })?;
    let verifier = super::password_verifier::create_verifier(password)
        .map_err(|message| WalletSecretError::Corrupt { message })?;

    write_blob(store, wallet_id, Blob::Salt, &salt)?;
    write_blob(store, wallet_id, Blob::Verifier, &verifier)?;
    write_blob(store, wallet_id, Blob::PrivateKey, &envelope)?;
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
    let envelope = read_blob(store, wallet_id, Blob::Seed)?;
    let master_key = derive_master_key(password, &salt);
    super::seed_envelope::decrypt(&envelope, &*master_key)
        .map(Zeroizing::new)
        .map_err(|message| WalletSecretError::Corrupt { message })
}

/// Unseal a private-key wallet's key.
pub fn unlock_private_key(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: &str,
) -> Result<Zeroizing<String>, WalletSecretError> {
    let verifier = read_blob(store, wallet_id, Blob::Verifier)?;
    if !super::password_verifier::verify(password, &verifier) {
        return Err(WalletSecretError::IncorrectPassword);
    }
    let salt = read_blob(store, wallet_id, Blob::Salt)?;
    let envelope = read_blob(store, wallet_id, Blob::PrivateKey)?;
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

/// Whether this wallet signs from a stored private key rather than a phrase.
///
/// Answered from the store rather than from a field on the wallet: the two
/// could disagree, and the store is the one that decides whether a signature
/// is possible.
pub fn is_private_key_backed(store: &dyn SecretStore, wallet_id: &str) -> bool {
    store
        .load_secret(
            Blob::PrivateKey.class(),
            Blob::PrivateKey.key(wallet_id),
        )
        .is_ok()
}

/// Whether this wallet's material is encrypted under a password.
///
/// Answered off the **verifier**, not the seed blob: both states store a seed
/// blob, and only a sealed wallet has a verifier and a salt beside it. Reading
/// the seed value to decide would mean identifying key material by its shape.
pub fn is_sealed(store: &dyn SecretStore, wallet_id: &str) -> bool {
    store
        .load_secret(Blob::Verifier.class(), Blob::Verifier.key(wallet_id))
        .is_ok()
}

/// Whether this wallet has any signing material at all.
pub fn has_signing_material(store: &dyn SecretStore, wallet_id: &str) -> bool {
    store
        .load_secret(Blob::Seed.class(), Blob::Seed.key(wallet_id))
        .is_ok()
        || is_private_key_backed(store, wallet_id)
}

/// Store material for a wallet with no password.
///
/// The other half of [`seal`]. The app has always had two kinds of wallet —
/// one with a password and one without — and only the first had a home here,
/// so the second was written by the front end under a key scheme of its own.
/// Salt and verifier are removed rather than left behind: a stale verifier
/// would make [`is_sealed`] answer yes for material that is not encrypted.
fn store_unsealed(
    store: &dyn SecretStore,
    wallet_id: &str,
    blob: Blob,
    value: &str,
) -> Result<(), WalletSecretError> {
    store.delete_secret(Blob::Salt.class(), Blob::Salt.key(wallet_id))?;
    store.delete_secret(Blob::Verifier.class(), Blob::Verifier.key(wallet_id))?;
    write_blob(store, wallet_id, blob, value.trim().as_bytes())
}

/// Store a seed phrase, sealed under `password` when one is given.
pub fn store_seed_phrase(
    store: &dyn SecretStore,
    wallet_id: &str,
    seed_phrase: &str,
    password: Option<&str>,
) -> Result<(), WalletSecretError> {
    match password.map(str::trim).filter(|p| !p.is_empty()) {
        Some(password) => seal(store, wallet_id, seed_phrase, password),
        None => store_unsealed(store, wallet_id, Blob::Seed, seed_phrase),
    }
}

/// Store a raw private key, sealed under `password` when one is given.
pub fn store_private_key(
    store: &dyn SecretStore,
    wallet_id: &str,
    private_key: &str,
    password: Option<&str>,
) -> Result<(), WalletSecretError> {
    match password.map(str::trim).filter(|p| !p.is_empty()) {
        Some(password) => seal_private_key(store, wallet_id, private_key, password),
        None => store_unsealed(store, wallet_id, Blob::PrivateKey, private_key),
    }
}

/// Read a wallet's seed phrase.
///
/// `password` is required exactly when the wallet is sealed. Supplying one for
/// an unsealed wallet is an error rather than something to ignore: a caller
/// that thinks it is unlocking something is a caller with a wrong idea of what
/// it is holding.
pub fn load_seed_phrase(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: Option<&str>,
) -> Result<Zeroizing<String>, WalletSecretError> {
    load_material(store, wallet_id, Blob::Seed, password)
}

/// Read a wallet's raw private key. Same password rule as
/// [`load_seed_phrase`].
pub fn load_private_key(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: Option<&str>,
) -> Result<Zeroizing<String>, WalletSecretError> {
    load_material(store, wallet_id, Blob::PrivateKey, password)
}

fn load_material(
    store: &dyn SecretStore,
    wallet_id: &str,
    blob: Blob,
    password: Option<&str>,
) -> Result<Zeroizing<String>, WalletSecretError> {
    let password = password.map(str::trim).filter(|p| !p.is_empty());
    if !is_sealed(store, wallet_id) {
        if password.is_some() {
            return Err(WalletSecretError::PasswordNotRequired);
        }
        let raw = read_blob(store, wallet_id, blob)?;
        return String::from_utf8(raw)
            .map(Zeroizing::new)
            .map_err(|e| WalletSecretError::Corrupt {
                message: format!("{} is not utf-8: {e}", blob.suffix()),
            });
    }
    let Some(password) = password else {
        return Err(WalletSecretError::PasswordRequired);
    };
    match blob {
        Blob::Seed => unlock(store, wallet_id, password),
        Blob::PrivateKey => unlock_private_key(store, wallet_id, password),
        Blob::Salt | Blob::Verifier => Err(WalletSecretError::Corrupt {
            message: "salt and verifier are not material".to_string(),
        }),
    }
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

    /// The state the app has always had and core did not.
    #[test]
    fn a_wallet_with_no_password_stores_and_reads_back_without_one() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "w", PHRASE, None).expect("store");
        assert!(!is_sealed(&store, "w"), "no password means not sealed");
        assert!(has_signing_material(&store, "w"));
        assert_eq!(&*load_seed_phrase(&store, "w", None).expect("load"), PHRASE);
    }

    /// Sealed and unsealed are told apart by the verifier, not by looking at
    /// the seed value. A password promotes a wallet from one to the other and
    /// the old plaintext must not survive it.
    #[test]
    fn adding_a_password_seals_what_was_stored_in_the_clear() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "w", PHRASE, None).expect("store");
        let plain = store
            .load_secret(Blob::Seed.class(), Blob::Seed.key("w"))
            .expect("raw");

        store_seed_phrase(&store, "w", PHRASE, Some("hunter2")).expect("seal");
        assert!(is_sealed(&store, "w"));
        let sealed = store
            .load_secret(Blob::Seed.class(), Blob::Seed.key("w"))
            .expect("raw");
        assert_ne!(plain, sealed, "sealing must replace the cleartext blob");
        assert_eq!(
            &*load_seed_phrase(&store, "w", Some("hunter2")).expect("load"),
            PHRASE
        );
    }

    /// And back the other way: dropping the password must not leave a verifier
    /// behind, or `is_sealed` would claim material is encrypted when it is not.
    #[test]
    fn dropping_the_password_clears_the_verifier_and_salt() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "w", PHRASE, Some("hunter2")).expect("seal");
        assert!(is_sealed(&store, "w"));

        store_seed_phrase(&store, "w", PHRASE, None).expect("unseal");
        assert!(!is_sealed(&store, "w"), "a stale verifier would lie here");
        assert!(store
            .load_secret(Blob::Salt.class(), Blob::Salt.key("w"))
            .is_err());
        assert_eq!(&*load_seed_phrase(&store, "w", None).expect("load"), PHRASE);
    }

    /// Neither direction is allowed to guess.
    #[test]
    fn the_password_is_required_exactly_when_the_wallet_is_sealed() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "sealed", PHRASE, Some("hunter2")).expect("seal");
        store_seed_phrase(&store, "open", PHRASE, None).expect("store");

        assert_eq!(
            load_seed_phrase(&store, "sealed", None).unwrap_err(),
            WalletSecretError::PasswordRequired
        );
        assert_eq!(
            load_seed_phrase(&store, "open", Some("hunter2")).unwrap_err(),
            WalletSecretError::PasswordNotRequired
        );
    }

    /// A private key takes the same two states as a phrase.
    #[test]
    fn a_private_key_stores_unsealed_too() {
        let store = InMemorySecretStore::new();
        store_private_key(&store, "w", "0xabc", None).expect("store");
        assert!(!is_sealed(&store, "w"));
        assert!(is_private_key_backed(&store, "w"));
        assert_eq!(&*load_private_key(&store, "w", None).expect("load"), "0xabc");
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
    /// A private-key wallet's key is sealed exactly like a seed.
    ///
    /// It matters most where there is no Keychain: the CLI's store is files on
    /// disk, so a key written in the clear would be a key on disk.
    #[test]
    fn a_private_key_seals_and_unlocks_with_the_right_password() {
        let store = InMemorySecretStore::default();
        let key = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        seal_private_key(&store, "w1", key, "hunter2").expect("seal");

        assert_eq!(&*unlock_private_key(&store, "w1", "hunter2").expect("unlock"), key);
        assert!(matches!(
            unlock_private_key(&store, "w1", "wrong"),
            Err(WalletSecretError::IncorrectPassword)
        ));

        // And nothing readable is left behind on disk.
        let raw = store
            .load_secret(SecretClass::PrivateKey, "w1.privatekey".into())
            .expect("a stored blob");
        assert!(!raw.contains(key), "the key is stored in the clear");
    }

    #[test]
    fn deleting_a_wallet_takes_its_private_key_too() {
        let store = InMemorySecretStore::default();
        seal_private_key(&store, "w1", "aa", "hunter2").expect("seal");
        delete(&store, "w1").expect("delete");
        assert!(matches!(
            unlock_private_key(&store, "w1", "hunter2"),
            Err(WalletSecretError::NotSealed)
        ));
    }

}
