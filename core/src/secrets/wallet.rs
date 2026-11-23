//! Wallet sealing policy: KDF cost, encrypted blobs, and storage keys.
//! Both front ends share this policy; storage backends are platform-owned
//! (Keychain on iOS, files for the CLI).

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rand::RngCore;
use zeroize::Zeroizing;

use super::store::{SecretClass, SecretStore, SecretStoreError};
use crate::store::wallet_domain::{SensitiveOverrides, WalletDerivationOverrides};

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
    /// The password does not open the sealed material.
    #[error("incorrect password")]
    IncorrectPassword,
    /// A blank password was supplied to read a wallet: a request for a
    /// password that has none, not the choice of no password.
    #[error("password cannot be empty")]
    EmptyPassword,
    /// A wallet cannot be sealed with a password shorter than the minimum.
    #[error("a wallet password needs at least {min_chars} characters")]
    PasswordTooShort { min_chars: u32 },
    /// The wallet requires a password and none was supplied.
    #[error("this wallet requires its password")]
    PasswordRequired,
    /// A password was supplied for a wallet without password protection. Reported
    /// rather than ignored: the caller believes it is unlocking something.
    #[error("this wallet has no password")]
    PasswordNotRequired,
    /// Something is stored, but it is not what this module writes.
    #[error("stored secret is corrupt: {message}")]
    Corrupt { message: String },
    /// The platform store itself failed.
    #[error("secret store failure: {message}")]
    Backend { message: String },
}

impl From<WalletSecretError> for crate::error::SpectraBridgeError {
    fn from(error: WalletSecretError) -> Self {
        let message = error.to_string();
        match error {
            WalletSecretError::NotSealed
            | WalletSecretError::IncorrectPassword
            | WalletSecretError::EmptyPassword
            | WalletSecretError::PasswordTooShort { .. }
            | WalletSecretError::PasswordRequired
            | WalletSecretError::PasswordNotRequired => Self::InvalidInput {
                message: message.into(),
            },
            WalletSecretError::Corrupt { .. } => Self::Decode { message },
            WalletSecretError::Backend { .. } => Self::Failure {
                message: message.into(),
            },
        }
    }
}

impl From<super::seed_envelope::EnvelopeError> for WalletSecretError {
    fn from(error: super::seed_envelope::EnvelopeError) -> Self {
        Self::Corrupt {
            message: error.to_string(),
        }
    }
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

/// What is stored for a wallet, split by bucket so the seed and key sit in the
/// platform's strongest one. Every blob is sealed under the device key on the
/// way to the store, so what comes back is what was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blob {
    /// The seed phrase. Sealed wallets hold an AES-GCM envelope here;
    /// unsealed ones hold the phrase itself. Which it is is answered by
    /// [`Blob::Salt`]'s presence, never by looking at this value — key
    /// material is not something to identify by shape.
    Seed,
    /// A raw private key, for a wallet imported from one instead of from a
    /// phrase. Same two states as [`Blob::Seed`].
    PrivateKey,
    /// The derivation overrides a phrase wallet was imported with — a BIP-39
    /// passphrase, a custom HMAC key — as JSON, sealed with the phrase: they
    /// derive its keys as much as it does. Absent when there are none.
    Overrides,
    /// The salt a sealed wallet's key was derived from; present exactly when
    /// the wallet is sealed under a password.
    Salt,
    /// The secret that reads what a wallet receives and spends nothing: a
    /// Monero view key, a Litecoin MWEB scan secret. Never password-sealed,
    /// so a watched wallet can sync, but it reveals every receipt.
    ScanKey,
}

impl Blob {
    fn class(self) -> SecretClass {
        match self {
            Blob::Seed | Blob::Overrides => SecretClass::Seed,
            Blob::PrivateKey => SecretClass::PrivateKey,
            Blob::Salt | Blob::ScanKey => SecretClass::Generic,
        }
    }

    /// Part of the on-disk layout: frozen.
    fn suffix(self) -> &'static str {
        match self {
            Blob::Seed => "seed",
            Blob::PrivateKey => "privatekey",
            Blob::Overrides => "overrides",
            Blob::Salt => "salt",
            Blob::ScanKey => "scan-key",
        }
    }

    fn key(self, wallet_id: &str) -> String {
        format!("{wallet_id}.{}", self.suffix())
    }

    const ALL: [Blob; 5] = [
        Blob::Seed,
        Blob::PrivateKey,
        Blob::Overrides,
        Blob::Salt,
        Blob::ScanKey,
    ];
}

fn write_blob(
    store: &dyn SecretStore,
    wallet_id: &str,
    blob: Blob,
    value: &[u8],
) -> Result<(), WalletSecretError> {
    let encoded = Zeroizing::new(STANDARD.encode(value));
    let stored = super::device_key::seal(store, encoded.as_bytes())?;
    store
        .save_secret(blob.class(), blob.key(wallet_id), stored)
        .map_err(WalletSecretError::from)
}

fn read_blob(
    store: &dyn SecretStore,
    wallet_id: &str,
    blob: Blob,
) -> Result<Vec<u8>, WalletSecretError> {
    let sealed = Zeroizing::new(store.load_secret(blob.class(), blob.key(wallet_id))?);
    let raw = super::device_key::open(store, &sealed)?;
    STANDARD
        .decode(raw.trim())
        .map_err(|e| WalletSecretError::Corrupt {
            message: format!("{} is not base64: {e}", blob.suffix()),
        })
}

/// `Zeroizing` so the key is wiped rather than left in the caller's frame.
fn derive_master_key(password: &str, salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0u8; 32]);
    crate::secrets::kdf::pbkdf2_sha256(
        password.trim().as_bytes(),
        salt,
        PBKDF2_ITERATIONS,
        &mut *key,
    );
    key
}

/// A password a wallet may be sealed under: at least the minimum length.
fn sealing_password(password: &str) -> Result<(), WalletSecretError> {
    if crate::validation::wallet_password_is_too_short(password) {
        return Err(WalletSecretError::PasswordTooShort {
            min_chars: crate::validation::MIN_WALLET_PASSWORD_CHARS,
        });
    }
    Ok(())
}

/// Write a wallet's material: each blob's text, or `None` to remove the blob.
/// Sealed under `password` when one is given, with one salt and one key
/// derivation for every blob, and refused below the minimum length; for
/// `None` only device-sealed, with any salt removed so the wallet does not
/// read as sealed. List the signing material last: it is written last.
///
/// One key derivation seals them all and one opens them: the salt sits beside
/// the envelopes, and the device seal every blob passes through tells a wrong
/// password from changed data, so no separate verifier is kept.
fn write_material(
    store: &dyn SecretStore,
    wallet_id: &str,
    blobs: &[(Blob, Option<&str>)],
    password: Option<&str>,
) -> Result<(), WalletSecretError> {
    let Some(password) = password else {
        store.delete_secret(Blob::Salt.class(), Blob::Salt.key(wallet_id))?;
        for (blob, text) in blobs {
            match text {
                Some(text) => write_blob(store, wallet_id, *blob, text.trim().as_bytes())?,
                None => store.delete_secret(blob.class(), blob.key(wallet_id))?,
            }
        }
        return Ok(());
    };
    sealing_password(password)?;
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    let master_key = derive_master_key(password, &salt);
    let envelopes = blobs
        .iter()
        .map(|(blob, text)| {
            let envelope = text
                .map(|text| super::seed_envelope::encrypt(text.as_bytes(), &*master_key))
                .transpose()?;
            Ok((*blob, envelope))
        })
        .collect::<Result<Vec<_>, WalletSecretError>>()?;
    write_blob(store, wallet_id, Blob::Salt, &salt)?;
    for (blob, envelope) in envelopes {
        match envelope {
            Some(envelope) => write_blob(store, wallet_id, blob, &envelope)?,
            None => store.delete_secret(blob.class(), blob.key(wallet_id))?,
        }
    }
    Ok(())
}

/// Open a wallet's material: `read` is handed a reader for its blobs, which
/// opens each under `password` when the wallet is sealed — one key derivation
/// for all of them — and reads it as stored when it is not.
///
/// `password` is required exactly when the wallet is sealed. Supplying one for
/// an unsealed wallet is an error rather than something to ignore: a caller
/// that thinks it is unlocking something is a caller with a wrong idea of what
/// it is holding. A blank password is refused.
fn read_material<T>(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: Option<&str>,
    read: impl FnOnce(
        &dyn Fn(Blob) -> Result<Zeroizing<String>, WalletSecretError>,
    ) -> Result<T, WalletSecretError>,
) -> Result<T, WalletSecretError> {
    let password = supplied_password(password)?;
    if !is_sealed(store, wallet_id)? {
        if password.is_some() {
            return Err(WalletSecretError::PasswordNotRequired);
        }
        return read(&|blob| {
            String::from_utf8(read_blob(store, wallet_id, blob)?)
                .map(Zeroizing::new)
                .map_err(|e| WalletSecretError::Corrupt {
                    message: format!("{} is not utf-8: {e}", blob.suffix()),
                })
        });
    }
    let Some(password) = password else {
        return Err(WalletSecretError::PasswordRequired);
    };
    let salt = read_blob(store, wallet_id, Blob::Salt)?;
    let master_key = derive_master_key(password, &salt);
    read(&|blob| {
        let envelope = read_blob(store, wallet_id, blob)?;
        match super::seed_envelope::decrypt(&envelope, &*master_key) {
            Ok(material) => Ok(Zeroizing::new(material)),
            // The device seal opened the blob, so it is as written: an
            // envelope that does not open under this key was sealed under
            // another password.
            Err(super::seed_envelope::EnvelopeError::Decrypt) => {
                Err(WalletSecretError::IncorrectPassword)
            }
            Err(error) => Err(error.into()),
        }
    })
}

/// Keep a wallet's scan key, device-sealed and never under its password.
pub fn store_scan_key(
    store: &dyn SecretStore,
    wallet_id: &str,
    scan_key: &str,
) -> Result<(), WalletSecretError> {
    write_blob(store, wallet_id, Blob::ScanKey, scan_key.as_bytes())
}

/// A wallet's scan key, or `None` when it has not been kept.
pub fn load_scan_key(
    store: &dyn SecretStore,
    wallet_id: &str,
) -> Result<Option<Zeroizing<String>>, WalletSecretError> {
    match read_blob(store, wallet_id, Blob::ScanKey) {
        Ok(raw) => String::from_utf8(raw)
            .map(|key| Some(Zeroizing::new(key)))
            .map_err(|e| WalletSecretError::Corrupt {
                message: format!("scan-key is not utf-8: {e}"),
            }),
        Err(WalletSecretError::NotSealed) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Idempotent, like the underlying store.
pub fn delete(store: &dyn SecretStore, wallet_id: &str) -> Result<(), WalletSecretError> {
    for blob in Blob::ALL {
        store.delete_secret(blob.class(), blob.key(wallet_id))?;
    }
    Ok(())
}

/// Whether `blob` is stored for this wallet.
///
/// `NotFound` is the only answer that means no. Any other failure is the store
/// failing, and reporting it as absence is how a sealed wallet whose salt
/// could not be read would come to be treated as unsealed.
fn is_stored(
    store: &dyn SecretStore,
    wallet_id: &str,
    blob: Blob,
) -> Result<bool, WalletSecretError> {
    match store.load_secret(blob.class(), blob.key(wallet_id)) {
        Ok(value) => {
            let _value = Zeroizing::new(value);
            Ok(true)
        }
        Err(SecretStoreError::NotFound) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Whether this wallet's material is encrypted under a password.
///
/// Answered off the **salt**, not the seed blob: both states store a seed
/// blob, and only a sealed wallet has a salt beside it. Reading the seed value
/// to decide would mean identifying key material by its shape.
pub fn is_sealed(store: &dyn SecretStore, wallet_id: &str) -> Result<bool, WalletSecretError> {
    is_stored(store, wallet_id, Blob::Salt)
}

/// The caller's password for reading, refused when it is blank.
///
/// `None` is the explicit choice of no password. `Some` is a request for one,
/// and a blank one is refused rather than read as `None`. Storing takes the
/// stricter sealing rule instead, under which a blank password is too short.
fn supplied_password(password: Option<&str>) -> Result<Option<&str>, WalletSecretError> {
    match password.map(str::trim) {
        Some("") => Err(WalletSecretError::EmptyPassword),
        password => Ok(password),
    }
}

/// Store a seed phrase and the derivation overrides it was imported with:
/// sealed under `password` when one is given, unsealed only for `None`. A
/// password shorter than the minimum, blank included, is refused rather than
/// read as `None`: that reading would store a wallet in the clear for a
/// caller that asked for it to be sealed. Empty overrides store nothing.
pub fn store_seed_phrase(
    store: &dyn SecretStore,
    wallet_id: &str,
    seed_phrase: &str,
    overrides: &WalletDerivationOverrides,
    password: Option<&str>,
) -> Result<(), WalletSecretError> {
    let overrides = SensitiveOverrides(overrides.clone().normalized());
    let overrides = (!overrides.0.is_empty())
        .then(|| serde_json::to_string(&overrides.0).map(Zeroizing::new))
        .transpose()
        .map_err(|e| WalletSecretError::Corrupt {
            message: format!("overrides could not be written: {e}"),
        })?;
    write_material(
        store,
        wallet_id,
        &[
            (Blob::Overrides, overrides.as_deref().map(String::as_str)),
            (Blob::Seed, Some(seed_phrase)),
        ],
        password.map(str::trim),
    )
}

/// Store a raw private key. Same password rule as [`store_seed_phrase`]; a
/// key derives with no overrides.
pub fn store_private_key(
    store: &dyn SecretStore,
    wallet_id: &str,
    private_key: &str,
    password: Option<&str>,
) -> Result<(), WalletSecretError> {
    write_material(
        store,
        wallet_id,
        &[
            (Blob::Overrides, None),
            (Blob::PrivateKey, Some(private_key)),
        ],
        password.map(str::trim),
    )
}

/// A wallet's phrase and the derivation overrides it was imported with,
/// opened together: one derives nothing without the other.
pub struct Mnemonic {
    pub phrase: Zeroizing<String>,
    pub overrides: SensitiveOverrides,
}

/// Read a wallet's phrase and its derivation overrides, with one key
/// derivation. Same password rule as [`read_material`].
pub fn load_mnemonic(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: Option<&str>,
) -> Result<Mnemonic, WalletSecretError> {
    let has_overrides = is_stored(store, wallet_id, Blob::Overrides)?;
    read_material(store, wallet_id, password, |open| {
        let phrase = open(Blob::Seed)?;
        let overrides = if has_overrides {
            let json = open(Blob::Overrides)?;
            serde_json::from_str(&json).map_err(|e| WalletSecretError::Corrupt {
                message: format!("overrides are not readable: {e}"),
            })?
        } else {
            WalletDerivationOverrides::default()
        };
        Ok(Mnemonic {
            phrase,
            overrides: SensitiveOverrides(overrides),
        })
    })
}

/// Read a wallet's seed phrase alone: the reveal path.
pub fn load_seed_phrase(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: Option<&str>,
) -> Result<Zeroizing<String>, WalletSecretError> {
    load_mnemonic(store, wallet_id, password).map(|mnemonic| mnemonic.phrase)
}

/// Read a wallet's raw private key. Same password rule as [`read_material`].
pub fn load_private_key(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: Option<&str>,
) -> Result<Zeroizing<String>, WalletSecretError> {
    read_material(store, wallet_id, password, |open| open(Blob::PrivateKey))
}

/// A stored wallet must have exactly one signing source. Never silently prefer
/// one blob when both exist, or turn a backend failure into "no key".
pub(crate) enum SigningMaterial {
    Mnemonic(Mnemonic),
    PrivateKey(Zeroizing<String>),
}

pub(crate) fn load_signing_material(
    store: &dyn SecretStore,
    wallet_id: &str,
    password: Option<&str>,
) -> Result<SigningMaterial, WalletSecretError> {
    match (
        is_stored(store, wallet_id, Blob::Seed)?,
        is_stored(store, wallet_id, Blob::PrivateKey)?,
    ) {
        (true, false) => load_mnemonic(store, wallet_id, password).map(SigningMaterial::Mnemonic),
        (false, true) => {
            load_private_key(store, wallet_id, password).map(SigningMaterial::PrivateKey)
        }
        (false, false) => Err(WalletSecretError::NotSealed),
        (true, true) => Err(WalletSecretError::Corrupt {
            message: "wallet contains both mnemonic and private key".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::backends::InMemorySecretStore;

    /// No derivation overrides.
    const NONE: WalletDerivationOverrides = WalletDerivationOverrides {
        passphrase: None,
        hmac_key: None,
    };

    /// A phrase sealed under `password`, as an import with no overrides seals it.
    fn seal(
        store: &dyn SecretStore,
        wallet_id: &str,
        seed_phrase: &str,
        password: &str,
    ) -> Result<(), WalletSecretError> {
        store_seed_phrase(store, wallet_id, seed_phrase, &NONE, Some(password))
    }

    /// A raw key sealed under `password`.
    fn seal_private_key(
        store: &dyn SecretStore,
        wallet_id: &str,
        private_key_hex: &str,
        password: &str,
    ) -> Result<(), WalletSecretError> {
        store_private_key(store, wallet_id, private_key_hex, Some(password))
    }

    fn is_private_key_backed(
        store: &dyn SecretStore,
        wallet_id: &str,
    ) -> Result<bool, WalletSecretError> {
        is_stored(store, wallet_id, Blob::PrivateKey)
    }

    fn has_signing_material(
        store: &dyn SecretStore,
        wallet_id: &str,
    ) -> Result<bool, WalletSecretError> {
        Ok(is_stored(store, wallet_id, Blob::Seed)? || is_private_key_backed(store, wallet_id)?)
    }

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn seals_and_unlocks_with_the_right_password() {
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        assert_eq!(
            *load_seed_phrase(&store, "W1", Some("hunter2")).unwrap(),
            PHRASE
        );
    }

    #[test]
    fn a_wrong_password_is_reported_as_such() {
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        assert_eq!(
            load_seed_phrase(&store, "W1", Some("hunter3")).unwrap_err(),
            WalletSecretError::IncorrectPassword
        );
    }

    #[test]
    fn an_unsealed_wallet_is_not_a_password_failure() {
        let store = InMemorySecretStore::new();
        assert_ne!(
            load_seed_phrase(&store, "missing", Some("hunter2")).unwrap_err(),
            WalletSecretError::IncorrectPassword
        );
        assert!(!is_sealed(&store, "missing").unwrap());
    }

    /// A wallet with no password stores and reads back without one.
    #[test]
    fn a_wallet_with_no_password_stores_and_reads_back_without_one() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "w", PHRASE, &NONE, None).expect("store");
        assert!(
            !is_sealed(&store, "w").unwrap(),
            "no password means not sealed"
        );
        assert!(has_signing_material(&store, "w").unwrap());
        assert_eq!(&*load_seed_phrase(&store, "w", None).expect("load"), PHRASE);
    }

    /// Sealed and unsealed are told apart by the salt, not by looking at the
    /// seed value. A password promotes a wallet from one to the other and
    /// the old plaintext must not survive it.
    #[test]
    fn adding_a_password_seals_what_was_stored_in_the_clear() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "w", PHRASE, &NONE, None).expect("store");
        let plain = store
            .load_secret(Blob::Seed.class(), Blob::Seed.key("w"))
            .expect("raw");

        store_seed_phrase(&store, "w", PHRASE, &NONE, Some("hunter2")).expect("seal");
        assert!(is_sealed(&store, "w").unwrap());
        let sealed = store
            .load_secret(Blob::Seed.class(), Blob::Seed.key("w"))
            .expect("raw");
        assert_ne!(plain, sealed, "sealing must replace the cleartext blob");
        assert_eq!(
            &*load_seed_phrase(&store, "w", Some("hunter2")).expect("load"),
            PHRASE
        );
    }

    /// And back the other way: dropping the password must not leave the salt
    /// behind, or `is_sealed` would claim material is encrypted when it is not.
    #[test]
    fn dropping_the_password_clears_the_salt() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "w", PHRASE, &NONE, Some("hunter2")).expect("seal");
        assert!(is_sealed(&store, "w").unwrap());

        store_seed_phrase(&store, "w", PHRASE, &NONE, None).expect("unseal");
        assert!(
            !is_sealed(&store, "w").unwrap(),
            "a stale salt would lie here"
        );
        assert!(
            store
                .load_secret(Blob::Salt.class(), Blob::Salt.key("w"))
                .is_err()
        );
        assert_eq!(&*load_seed_phrase(&store, "w", None).expect("load"), PHRASE);
    }

    /// Neither direction is allowed to guess.
    #[test]
    fn the_password_is_required_exactly_when_the_wallet_is_sealed() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "sealed", PHRASE, &NONE, Some("hunter2")).expect("seal");
        store_seed_phrase(&store, "open", PHRASE, &NONE, None).expect("store");

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
        assert!(!is_sealed(&store, "w").unwrap());
        assert!(is_private_key_backed(&store, "w").unwrap());
        assert_eq!(
            &*load_private_key(&store, "w", None).expect("load"),
            "0xabc"
        );
    }

    /// No password is no reason to store a phrase or key in the clear: both
    /// are sealed under the device key on the way to the store.
    #[test]
    fn material_without_a_password_is_still_device_sealed() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "w", PHRASE, &NONE, None).unwrap();
        let key = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        store_private_key(&store, "k", key, None).unwrap();
        let seed = store
            .load_secret(SecretClass::Seed, Blob::Seed.key("w"))
            .unwrap();
        let private = store
            .load_secret(SecretClass::PrivateKey, Blob::PrivateKey.key("k"))
            .unwrap();
        assert!(!seed.contains("abandon") && !seed.contains(&STANDARD.encode(PHRASE)));
        assert!(!private.contains(&key[..16]) && !private.contains(&STANDARD.encode(key)));
        assert_eq!(&*load_seed_phrase(&store, "w", None).unwrap(), PHRASE);
        assert_eq!(&*load_private_key(&store, "k", None).unwrap(), key);
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
        assert_eq!(
            *load_seed_phrase(&store, "W2", Some("hunter2")).unwrap(),
            PHRASE
        );
    }

    #[test]
    fn resealing_replaces_every_blob() {
        // A reseal that left the old salt behind would derive the master key
        // from one salt and decrypt an envelope sealed under another.
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "first").unwrap();
        seal(&store, "W1", PHRASE, "second").unwrap();
        assert_eq!(
            *load_seed_phrase(&store, "W1", Some("second")).unwrap(),
            PHRASE
        );
        assert_eq!(
            load_seed_phrase(&store, "W1", Some("first")).unwrap_err(),
            WalletSecretError::IncorrectPassword
        );
    }

    #[test]
    fn delete_removes_every_blob() {
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        store_scan_key(&store, "W1", "view").unwrap();
        delete(&store, "W1").unwrap();
        assert!(!is_sealed(&store, "W1").unwrap());
        for blob in Blob::ALL {
            assert!(store.load_secret(blob.class(), blob.key("W1")).is_err());
        }
    }

    /// A password shorter than the minimum is refused, blank included: a
    /// blank one is a request for a password that has none, not the choice of
    /// no password, and must not store the material in the clear.
    #[test]
    fn a_short_password_stores_nothing_rather_than_storing_unsealed() {
        let store = InMemorySecretStore::new();
        let too_short = WalletSecretError::PasswordTooShort {
            min_chars: crate::validation::MIN_WALLET_PASSWORD_CHARS,
        };
        for short in ["", "   ", "\t\n", "a", "abc", "  abc  "] {
            assert_eq!(
                store_seed_phrase(&store, "w", PHRASE, &NONE, Some(short)).unwrap_err(),
                too_short
            );
            assert_eq!(
                store_private_key(&store, "k", "0xabc", Some(short)).unwrap_err(),
                too_short
            );
            assert_eq!(seal(&store, "w", PHRASE, short).unwrap_err(), too_short);
            assert_eq!(
                seal_private_key(&store, "k", "0xabc", short).unwrap_err(),
                too_short
            );
        }
        for id in ["w", "k"] {
            assert!(!is_sealed(&store, id).unwrap());
            assert!(!has_signing_material(&store, id).unwrap());
        }
        seal(&store, "w", PHRASE, "abcd").expect("the minimum is long enough");
        assert!(is_sealed(&store, "w").unwrap());
    }

    /// Reading takes the same rule: a blank password is neither "no password"
    /// for an unsealed wallet nor a missing one for a sealed wallet.
    #[test]
    fn a_blank_password_is_refused_when_reading() {
        let store = InMemorySecretStore::new();
        store_seed_phrase(&store, "sealed", PHRASE, &NONE, Some("hunter2")).expect("seal");
        store_seed_phrase(&store, "open", PHRASE, &NONE, None).expect("store");
        for id in ["sealed", "open"] {
            assert_eq!(
                load_seed_phrase(&store, id, Some("  ")).unwrap_err(),
                WalletSecretError::EmptyPassword
            );
        }
    }

    /// A scan key is kept device-sealed, never in the clear, and reads back
    /// as it was kept; a wallet that kept none has none.
    #[test]
    fn a_scan_key_is_device_sealed_and_reads_back() {
        let store = InMemorySecretStore::new();
        assert_eq!(load_scan_key(&store, "W1").unwrap(), None);
        store_scan_key(&store, "W1", "ab".repeat(32).as_str()).unwrap();
        let stored = store
            .load_secret(Blob::ScanKey.class(), Blob::ScanKey.key("W1"))
            .unwrap();
        assert!(!stored.contains(&"ab".repeat(32)));
        assert!(!stored.contains(&STANDARD.encode("ab".repeat(32))));
        assert_eq!(
            load_scan_key(&store, "W1")
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("ab".repeat(32).as_str())
        );
    }

    /// Derivation overrides are sealed with the phrase, under the same
    /// password and salt, and come back with it; storing the phrase again
    /// without them leaves none behind.
    #[test]
    fn overrides_are_sealed_with_the_phrase() {
        let store = InMemorySecretStore::new();
        let overrides = WalletDerivationOverrides {
            passphrase: Some("extra words".into()),
            hmac_key: None,
        };
        store_seed_phrase(&store, "w", PHRASE, &overrides, Some("hunter2")).unwrap();
        let raw = store
            .load_secret(Blob::Overrides.class(), Blob::Overrides.key("w"))
            .unwrap();
        assert!(!raw.contains("extra words"));
        let mnemonic = load_mnemonic(&store, "w", Some("hunter2")).unwrap();
        assert_eq!(&*mnemonic.phrase, PHRASE);
        assert_eq!(mnemonic.overrides.passphrase(), Some("extra words"));
        assert!(matches!(
            load_mnemonic(&store, "w", Some("hunter3")),
            Err(WalletSecretError::IncorrectPassword)
        ));

        store_seed_phrase(&store, "w", PHRASE, &NONE, None).unwrap();
        assert!(
            store
                .load_secret(Blob::Overrides.class(), Blob::Overrides.key("w"))
                .is_err()
        );
        assert_eq!(
            load_mnemonic(&store, "w", None)
                .unwrap()
                .overrides
                .passphrase(),
            None
        );
    }

    #[test]
    fn a_corrupt_blob_is_not_reported_as_a_wrong_password() {
        let store = InMemorySecretStore::new();
        seal(&store, "W1", PHRASE, "hunter2").unwrap();
        store
            .save_secret(SecretClass::Generic, "W1.salt".to_string(), "!!!".into())
            .unwrap();
        assert!(matches!(
            load_seed_phrase(&store, "W1", Some("hunter2")).unwrap_err(),
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

        assert_eq!(
            &*load_private_key(&store, "w1", Some("hunter2")).expect("unlock"),
            key
        );
        assert!(matches!(
            load_private_key(&store, "w1", Some("wrong")),
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
            load_private_key(&store, "w1", None),
            Err(WalletSecretError::NotSealed)
        ));
    }

    /// A store that fails to read one bucket, and answers normally otherwise.
    struct UnreadableGeneric(InMemorySecretStore);

    impl SecretStore for UnreadableGeneric {
        fn load_secret(&self, kind: SecretClass, key: String) -> Result<String, SecretStoreError> {
            if kind == SecretClass::Generic {
                return Err(SecretStoreError::Backend {
                    message: "device locked".into(),
                });
            }
            self.0.load_secret(kind, key)
        }
        fn save_secret(
            &self,
            kind: SecretClass,
            key: String,
            value: String,
        ) -> Result<(), SecretStoreError> {
            self.0.save_secret(kind, key, value)
        }
        fn delete_secret(&self, kind: SecretClass, key: String) -> Result<(), SecretStoreError> {
            self.0.delete_secret(kind, key)
        }
        fn wrap_device_key(&self, key: Vec<u8>) -> Result<Vec<u8>, SecretStoreError> {
            self.0.wrap_device_key(key)
        }
        fn unwrap_device_key(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, SecretStoreError> {
            self.0.unwrap_device_key(wrapped)
        }
    }

    /// A salt that cannot be read is not a salt that is absent.
    ///
    /// `is_sealed` answering `false` for any failure would treat a sealed
    /// wallet read while its salt was unreadable as unsealed: the reveal path
    /// would take the sealed envelope for the phrase itself.
    #[test]
    fn an_unreadable_store_is_an_error_not_an_unsealed_wallet() {
        let inner = InMemorySecretStore::new();
        seal(&inner, "W1", PHRASE, "hunter2").unwrap();
        let store = UnreadableGeneric(inner);

        assert!(matches!(
            is_sealed(&store, "W1"),
            Err(WalletSecretError::Backend { .. })
        ));
        assert!(matches!(
            load_seed_phrase(&store, "W1", None),
            Err(WalletSecretError::Backend { .. })
        ));
        // The seed bucket itself is readable, so the material is there.
        assert!(has_signing_material(&store, "W1").unwrap());
    }
}
