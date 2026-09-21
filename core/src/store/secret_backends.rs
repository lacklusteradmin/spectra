//! Native [`SecretStore`] implementations.
//!
//! `SecretStore` is a UniFFI callback trait that Swift and Kotlin implement
//! against Keychain / Keystore. These two implementations exist so the *same*
//! core code paths run with no mobile platform underneath:
//!
//! - [`InMemorySecretStore`] — tests, and anything that wants a throwaway store.
//! - [`FileSecretStore`] — the CLI, and any other headless Rust consumer.
//!
//! Both are plain Rust and neither crosses the FFI. Their reason to exist is
//! that a trait with no Rust implementation cannot be exercised without a
//! device, which is how the CLI ended up with its own parallel secret storage.
//!
//! ## What these protect, and what they don't
//!
//! The trait contract says values are opaque strings and the *caller* picks the
//! encoding. These backends honour that exactly: they store the bytes they are
//! handed and add no encryption of their own. Seed material handed to
//! [`FileSecretStore`] is expected to already be a
//! [`seed_envelope`](super::seed_envelope) ciphertext.
//!
//! On Unix the root directory is created `0700` and each secret file `0600`.
//! That is filesystem permissions and nothing more — a file store has no
//! hardware backing and no at-rest protection the way Keychain does. It is the
//! right backend for a developer CLI and the wrong one for a shipping app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;

use super::secret_store::{SecretClass, SecretStore, SecretStoreError};

// ── Key ↔ filename encoding ──────────────────────────────────────────────────
//
// Secret keys are caller-chosen strings ("<wallet-id>.seed")
// and each must name exactly one file. Anything outside [A-Za-z0-9_-] is
// percent-encoded, `.` included — that keeps `.`, `..` and dotfiles
// unrepresentable, so no key can escape its bucket. Nothing lists a store, so
// there is no decoder.

/// Longest encoded filename we will write. Real keys are wallet-id shaped and
/// land far below this; the limit exists so a pathological key fails loudly
/// instead of hitting a filesystem `ENAMETOOLONG` deep inside an io error.
const MAX_ENCODED_KEY_LEN: usize = 200;

fn encode_key(key: &str) -> Result<String, SecretStoreError> {
    if key.is_empty() {
        return Err(SecretStoreError::Backend {
            message: "secret key must not be empty".to_string(),
        });
    }
    let mut encoded = String::with_capacity(key.len());
    for byte in key.as_bytes() {
        if byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_' {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    if encoded.len() > MAX_ENCODED_KEY_LEN {
        return Err(SecretStoreError::Backend {
            message: format!(
                "secret key too long: {} bytes encoded, limit {MAX_ENCODED_KEY_LEN}",
                encoded.len()
            ),
        });
    }
    Ok(encoded)
}

/// Non-persistent [`SecretStore`], backed by a map. Contents vanish on drop.
///
/// Use it in tests: it lets a test drive any core path that needs secret I/O
/// without touching the filesystem or a Keychain.
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    entries: Mutex<BTreeMap<(&'static str, String), String>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored secrets across all buckets. Test affordance.
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }
}

impl SecretStore for InMemorySecretStore {
    fn load_secret(&self, kind: SecretClass, key: String) -> Result<String, SecretStoreError> {
        self.entries
            .lock()
            .get(&(kind.bucket(), key))
            .cloned()
            .ok_or(SecretStoreError::NotFound)
    }

    fn save_secret(
        &self,
        kind: SecretClass,
        key: String,
        value: String,
    ) -> Result<(), SecretStoreError> {
        self.entries.lock().insert((kind.bucket(), key), value);
        Ok(())
    }

    fn delete_secret(&self, kind: SecretClass, key: String) -> Result<(), SecretStoreError> {
        self.entries.lock().remove(&(kind.bucket(), key));
        Ok(())
    }
}

// ── Filesystem ───────────────────────────────────────────────────────────────

/// Filesystem-backed [`SecretStore`] rooted at a directory.
///
/// Layout is `<root>/<bucket>/<encoded-key>`, one file per secret, where
/// `bucket` is [`SecretClass::bucket`]. Writes are atomic per key: the value
/// goes to a temporary file in the same directory and is then renamed over the
/// destination, so a crash mid-write cannot leave a truncated secret behind.
///
/// See the module docs for the (limited) protection this offers.
#[derive(Debug, Clone)]
pub struct FileSecretStore {
    root: PathBuf,
}

impl FileSecretStore {
    /// Create the store, creating `root` if it does not exist.
    ///
    /// On Unix `root` is set to `0700`. That is applied whether or not this
    /// call created it, so pointing an existing world-readable directory at
    /// this constructor tightens it rather than silently inheriting it.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, SecretStoreError> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|e| backend(&root, "create root", &e))?;
        restrict_dir(&root)?;
        Ok(Self { root })
    }

    /// Root directory this store reads and writes.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn bucket_dir(&self, kind: SecretClass) -> PathBuf {
        self.root.join(kind.bucket())
    }

    fn secret_path(&self, kind: SecretClass, key: &str) -> Result<PathBuf, SecretStoreError> {
        Ok(self.bucket_dir(kind).join(encode_key(key)?))
    }
}

impl SecretStore for FileSecretStore {
    fn load_secret(&self, kind: SecretClass, key: String) -> Result<String, SecretStoreError> {
        let path = self.secret_path(kind, &key)?;
        match std::fs::read_to_string(&path) {
            Ok(value) => Ok(value),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SecretStoreError::NotFound),
            Err(e) => Err(backend(&path, "read", &e)),
        }
    }

    fn save_secret(
        &self,
        kind: SecretClass,
        key: String,
        value: String,
    ) -> Result<(), SecretStoreError> {
        let dir = self.bucket_dir(kind);
        std::fs::create_dir_all(&dir).map_err(|e| backend(&dir, "create bucket", &e))?;
        restrict_dir(&dir)?;

        let path = self.secret_path(kind, &key)?;
        // Same-directory temp file keeps the rename on one filesystem, which is
        // what makes it atomic.
        let temp = path.with_extension("tmp");
        write_private(&temp, &value)?;
        std::fs::rename(&temp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            backend(&path, "rename into place", &e)
        })
    }

    fn delete_secret(&self, kind: SecretClass, key: String) -> Result<(), SecretStoreError> {
        let path = self.secret_path(kind, &key)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Contract says delete is idempotent.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(backend(&path, "delete", &e)),
        }
    }
}

fn backend(path: &Path, action: &str, error: &std::io::Error) -> SecretStoreError {
    SecretStoreError::Backend {
        message: format!("secret store: {action} {}: {error}", path.display()),
    }
}

/// Write `value` to `path`, owner-only from the moment the file exists.
///
/// The mode is set in the open call rather than with a follow-up `chmod` so
/// there is no window where the secret is on disk and group/world readable.
fn write_private(path: &Path, value: &str) -> Result<(), SecretStoreError> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| backend(path, "open", &e))?;
    file.write_all(value.as_bytes())
        .map_err(|e| backend(path, "write", &e))?;
    // Durability matters here: a lost seed envelope is unrecoverable.
    file.sync_all().map_err(|e| backend(path, "sync", &e))
}

/// Set a directory to owner-only. No-op on non-Unix.
fn restrict_dir(path: &Path) -> Result<(), SecretStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(path, permissions)
            .map_err(|e| backend(path, "restrict permissions on", &e))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "spectra-secret-backends-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    /// Every assertion a `SecretStore` implementation must satisfy. Both
    /// backends run it, so "works in tests" and "works in the CLI" cannot
    /// drift apart.
    fn assert_conforms(store: &dyn SecretStore) {
        // Absent key.
        assert!(matches!(
            store.load_secret(SecretClass::Seed, "missing".into()),
            Err(SecretStoreError::NotFound)
        ));

        // Round trip.
        store
            .save_secret(SecretClass::Seed, "wallet-1.seed".into(), "envelope".into())
            .unwrap();
        assert_eq!(
            store
                .load_secret(SecretClass::Seed, "wallet-1.seed".into())
                .unwrap(),
            "envelope"
        );

        // Buckets are independent: same key, different class, different value.
        store
            .save_secret(
                SecretClass::PrivateKey,
                "wallet-1.seed".into(),
                "raw-key".into(),
            )
            .unwrap();
        assert_eq!(
            store
                .load_secret(SecretClass::Seed, "wallet-1.seed".into())
                .unwrap(),
            "envelope"
        );
        assert_eq!(
            store
                .load_secret(SecretClass::PrivateKey, "wallet-1.seed".into())
                .unwrap(),
            "raw-key"
        );

        // Overwrite replaces.
        store
            .save_secret(
                SecretClass::Seed,
                "wallet-1.seed".into(),
                "envelope-2".into(),
            )
            .unwrap();
        assert_eq!(
            store
                .load_secret(SecretClass::Seed, "wallet-1.seed".into())
                .unwrap(),
            "envelope-2"
        );

        // Delete is idempotent and scoped to one bucket.
        store
            .delete_secret(SecretClass::Seed, "wallet-1.seed".into())
            .unwrap();
        store
            .delete_secret(SecretClass::Seed, "wallet-1.seed".into())
            .unwrap();
        assert!(matches!(
            store.load_secret(SecretClass::Seed, "wallet-1.seed".into()),
            Err(SecretStoreError::NotFound)
        ));
        assert_eq!(
            store
                .load_secret(SecretClass::PrivateKey, "wallet-1.seed".into())
                .unwrap(),
            "raw-key"
        );

        // Keys that stress the filename encoding.
        for key in ["a/b", "..", ".", "空格 key", "%41", "a\\b"] {
            store
                .save_secret(SecretClass::Generic, key.into(), "v".into())
                .unwrap();
            assert_eq!(
                store.load_secret(SecretClass::Generic, key.into()).unwrap(),
                "v",
                "round trip failed for key {key:?}"
            );
        }
    }

    #[test]
    fn in_memory_store_conforms() {
        assert_conforms(&InMemorySecretStore::new());
    }

    #[test]
    fn file_store_conforms() {
        let root = temp_root("conforms");
        let store = FileSecretStore::new(&root).unwrap();
        assert_conforms(&store);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn key_encoding_stays_inside_the_bucket() {
        for key in ["simple", "a/b", "..", ".", "a b", "%41", "wallet-1.seed"] {
            let encoded = encode_key(key).unwrap();
            assert!(
                !encoded.contains('/') && !encoded.contains('\\'),
                "{key:?} encoded to a path separator: {encoded:?}"
            );
            assert!(
                encoded != "." && encoded != ".." && !encoded.starts_with('.'),
                "{key:?} encoded to a directory-traversal or hidden name: {encoded:?}"
            );
        }
    }

    #[test]
    fn empty_and_overlong_keys_are_rejected() {
        assert!(encode_key("").is_err());
        assert!(encode_key(&"x".repeat(MAX_ENCODED_KEY_LEN + 1)).is_err());
        // The limit is on encoded length, not input length: '空' is 3 UTF-8
        // bytes and so costs 9 encoded chars. 22 of them fit under 200; 23
        // don't, even though 23 chars is nowhere near the limit by itself.
        assert!(encode_key(&"空".repeat(MAX_ENCODED_KEY_LEN / 9)).is_ok());
        assert!(encode_key(&"空".repeat(MAX_ENCODED_KEY_LEN / 9 + 1)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("perms");
        let store = FileSecretStore::new(&root).unwrap();
        store
            .save_secret(SecretClass::Seed, "wallet-1".into(), "envelope".into())
            .unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(&root.join("seed")), 0o700);
        assert_eq!(mode(&root.join("seed").join("wallet-1")), 0o600);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn new_tightens_an_existing_loose_root() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("loose");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();

        let _store = FileSecretStore::new(&root).unwrap();
        let mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
