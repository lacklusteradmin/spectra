//! `SecretStore` — UniFFI callback trait for platform-backed secret storage.
//!
//! Swift (and future Android) implements this interface and hands an instance
//! to `WalletService::set_secret_store`. Rust then drives all secret I/O through
//! it, keeping raw key material out of Rust-owned memory and letting each
//! platform apply its strongest available protection (Secure Enclave / StrongBox).
//!
//! Design notes:
//! - Callers classify every secret with a `SecretClass` so the platform layer
//!   can route to the correct Keychain service / Android keystore and apply the
//!   right accessibility attributes. Classification lives on the Rust side so
//!   policy is uniform across platforms; the adapter only maps class → backend.
//! - Methods return `Result` with a structured `SecretStoreError`, separating
//!   "key absent" (`NotFound`) from backend failures (`Backend`) and the
//!   pre-registration state (`Unavailable`).
//! - Values are opaque `String`s. Callers decide encoding (base64/hex/JSON).
//! - All methods are synchronous from the foreign side; UniFFI wraps them in a
//!   blocking executor. Keychain I/O is fast enough that blocking is fine.

use thiserror::Error;

/// Classification hint that tells the platform layer *what kind* of secret is
/// being stored. The adapter maps each class to the correct backend bucket and
/// the appropriate accessibility / encryption policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SecretClass {
    /// BIP39 seed phrase, already sealed under the device key by core. The
    /// platform stores it in its strongest bucket (on iOS, with
    /// `kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly`).
    Seed,
    /// Raw private-key material (hex/WIF), sealed like a seed. Stored in a
    /// bucket distinct from the seed one.
    PrivateKey,
    /// Non-seed secrets: API tokens, password verifiers, per-wallet config.
    Generic,
    /// The device key that seals seeds and private keys, as
    /// [`SecretStore::wrap_device_key`] returned it.
    DeviceKey,
}

impl SecretClass {
    /// Stable, filesystem- and identifier-safe name for this class's bucket.
    ///
    /// Native backends (see [`secret_backends`](super::secret_backends)) use it
    /// as a directory / map key. It is part of the on-disk layout, so treat
    /// these strings as frozen.
    pub fn bucket(self) -> &'static str {
        match self {
            SecretClass::Seed => "seed",
            SecretClass::PrivateKey => "private_key",
            SecretClass::Generic => "generic",
            SecretClass::DeviceKey => "device_key",
        }
    }
}

/// Error surface returned across the FFI. Keeps "not found" distinct from
/// infrastructure failures so callers can react correctly.
#[derive(Debug, Error, uniffi::Error)]
pub enum SecretStoreError {
    /// No entry exists under the requested key.
    #[error("secret not found")]
    NotFound,
    /// `WalletService::set_secret_store` has not been called yet.
    #[error("secret store not registered")]
    Unavailable,
    /// Platform backend (Keychain / Keystore) reported a failure.
    #[error("secret store backend error: {message}")]
    Backend { message: String },
}

impl From<SecretStoreError> for crate::SpectraBridgeError {
    fn from(error: SecretStoreError) -> Self {
        Self::failure(error)
    }
}

/// Platform-backed secret store. Implemented in Swift/Kotlin, called from Rust.
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    /// Read the secret stored under `key` within the `kind` bucket.
    fn load_secret(&self, kind: SecretClass, key: String) -> Result<String, SecretStoreError>;

    /// Write `value` under `key` within the `kind` bucket, replacing any
    /// existing entry.
    fn save_secret(
        &self,
        kind: SecretClass,
        key: String,
        value: String,
    ) -> Result<(), SecretStoreError>;

    /// Remove the entry for `key` within the `kind` bucket. Succeeds whether
    /// or not the key existed (idempotent delete).
    fn delete_secret(&self, kind: SecretClass, key: String) -> Result<(), SecretStoreError>;

    /// Protect the device key with something only this device holds — on iOS,
    /// encryption to a Secure Enclave key. Core stores what this returns and
    /// mints, seals and refuses by its own rules; see
    /// [`device_key`](super::device_key). A backend with nothing stronger
    /// returns `key` unchanged.
    fn wrap_device_key(&self, key: Vec<u8>) -> Result<Vec<u8>, SecretStoreError>;

    /// Undo [`wrap_device_key`](Self::wrap_device_key). A failure is an error,
    /// never an empty key: core would otherwise take it for no key at all.
    fn unwrap_device_key(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, SecretStoreError>;
}
