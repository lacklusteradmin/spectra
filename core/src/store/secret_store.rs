//! `SecretStore` — UniFFI callback trait for Keychain access.
//!
//! Swift implements `SecretStoreProtocol` by conforming to this interface and
//! passing an instance to `WalletService::set_secret_store`. Rust then drives
//! all Keychain reads and writes through the callbacks, keeping secrets out of
//! Rust-owned memory.
//!
//! Design notes:
//! - All methods are synchronous from Swift's perspective (Keychain I/O is
//!   fast). UniFFI wraps foreign impls in a blocking executor automatically.
//! - Values are opaque `String`s. Callers decide the encoding (base64, hex,
//!   JSON). The store does not inspect them.
//! - `load_secret` returning `None` means the key is absent (distinct from an
//!   empty string value).

/// Keychain / secure-storage abstraction. Implemented in Swift, called from Rust.
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    /// Read the secret stored under `key`. Returns `None` if absent.
    fn load_secret(&self, key: String) -> Option<String>;

    /// Write `value` under `key`. Returns `true` on success.
    fn save_secret(&self, key: String, value: String) -> bool;

    /// Remove the entry for `key`. Returns `true` if the key existed.
    fn delete_secret(&self, key: String) -> bool;

    /// Return all keys whose prefix matches `prefix_filter`. Used to enumerate
    /// e.g. all seed-phrase keys for a specific wallet.
    fn list_keys(&self, prefix_filter: String) -> Vec<String>;
}
