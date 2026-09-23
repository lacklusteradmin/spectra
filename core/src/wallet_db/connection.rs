//! SQLite-backed relational store for per-wallet UTXO state.
//!
//! Replaces four UserDefaults JSON blobs that Swift's WalletStore currently owns:
//!   - `dogecoin.keypool.snapshot`          → `wallet_keypool` table
//!   - `chain.keypool.snapshot.v1`          → `wallet_keypool` table
//!   - `dogecoin.ownedAddressMap.snapshot`  → `wallet_owned_addresses` table
//!   - `chain.ownedAddressMap.snapshot.v1`  → `wallet_owned_addresses` table
//!
//! All functions are synchronous (call from `spawn_blocking` in `service::state`).
//!
//! ## Schema
//!
//! ```sql
//! wallet_keypool (wallet_id, chain_name) → (next_external_index, next_change_index, reserved_receive_index)
//! wallet_owned_addresses (wallet_id, chain_name, address) → (derivation_path, branch, branch_index)
//! ```

use parking_lot::Mutex;
use rusqlite::Connection;

/// Explicitly owned SQLite connection, shared by cloning its Arc.
pub struct WalletDatabase {
    path: String,
    connection: Mutex<Option<Connection>>,
}

impl WalletDatabase {
    /// Create a handle; the first storage operation opens the connection.
    pub fn new(database_path: &str) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            path: database_path.to_string(),
            connection: Mutex::new(None),
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn with_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self.connection.lock();
        if guard.is_none() {
            *guard = Some(open_new(&self.path)?);
        }
        f(guard.as_ref().expect("connection initialized"))
    }
}

pub(super) fn with_conn<T>(
    database: &WalletDatabase,
    f: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    database.with_connection(f)
}

fn open_new(database_path: &str) -> Result<Connection, String> {
    let conn = Connection::open(database_path)
        .map_err(|e| format!("wallet_db open {database_path}: {e}"))?;
    conn.create_scalar_function(
        "spectra_lower",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |context| Ok(context.get::<String>(0)?.to_lowercase()),
    )
    .map_err(|e| format!("wallet_db search function: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("wallet_db busy timeout: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         CREATE TABLE IF NOT EXISTS monero_wallets (wallet_id TEXT NOT NULL, chain_id TEXT NOT NULL, revision INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(wallet_id, chain_id));
         CREATE TABLE IF NOT EXISTS send_artifacts (
             id TEXT PRIMARY KEY NOT NULL,
             revision INTEGER NOT NULL,
             payload TEXT NOT NULL CHECK(json_valid(payload))
         );
         CREATE INDEX IF NOT EXISTS idx_send_sender ON send_artifacts
             (json_extract(payload, '$.view.chain_id'), lower(json_extract(payload, '$.view.sender')), json_extract(payload, '$.view.stage'));
         CREATE INDEX IF NOT EXISTS idx_send_wallet ON send_artifacts
             (json_extract(payload, '$.view.wallet_id'), json_extract(payload, '$.view.chain_id'), json_extract(payload, '$.view.stage'));
         CREATE TABLE IF NOT EXISTS send_reservations (
             resource TEXT PRIMARY KEY NOT NULL,
             artifact_id TEXT NOT NULL REFERENCES send_artifacts(id)
         );
         CREATE TABLE IF NOT EXISTS wallet_keypool (
             wallet_id              TEXT    NOT NULL,
             chain_name             TEXT    NOT NULL,
             next_external_index    INTEGER NOT NULL DEFAULT 0,
             next_change_index      INTEGER NOT NULL DEFAULT 0,
             reserved_receive_index INTEGER,           -- NULL = not reserved
             updated_at             INTEGER NOT NULL,
             PRIMARY KEY (wallet_id, chain_name)
         );
         CREATE TABLE IF NOT EXISTS wallet_owned_addresses (
             wallet_id       TEXT    NOT NULL,
             chain_name      TEXT    NOT NULL,
             address         TEXT    NOT NULL,
             derivation_path TEXT,
             branch          TEXT,                    -- 'external' | 'change'
             branch_index    INTEGER,
             updated_at      INTEGER NOT NULL,
             PRIMARY KEY (wallet_id, chain_name, address)
         );
         CREATE TABLE IF NOT EXISTS history_records (
             id         TEXT NOT NULL PRIMARY KEY,
             wallet_id  TEXT,
             chain_name TEXT NOT NULL,
             tx_hash    TEXT,
             created_at REAL NOT NULL,
             payload    TEXT NOT NULL CHECK (
                 json_valid(payload)
                 AND json_type(payload, '$.id') IS 'text'
                 AND coalesce(json_extract(payload, '$.kind'), '') IN ('send', 'receive')
                 AND coalesce(json_extract(payload, '$.status'), '') IN ('pending', 'confirmed', 'failed')),
             asset_key TEXT GENERATED ALWAYS AS
                 (coalesce(json_extract(payload, '$.deploymentId'), 'record:' || id)) STORED,
             hash_key TEXT GENERATED ALWAYS AS
                 (coalesce(nullif(json_extract(payload, '$.transactionHash'), ''), id)) STORED,
             status_rank INTEGER GENERATED ALWAYS AS
                 (CASE json_extract(payload, '$.status') WHEN 'confirmed' THEN 3 WHEN 'pending' THEN 2 ELSE 1 END) STORED
         );
         CREATE INDEX IF NOT EXISTS idx_hr_wallet  ON history_records(wallet_id);
         CREATE INDEX IF NOT EXISTS idx_hr_chain   ON history_records(chain_name);
         CREATE INDEX IF NOT EXISTS idx_hr_created ON history_records(created_at DESC, id ASC);
         CREATE INDEX IF NOT EXISTS idx_hr_oldest ON history_records(created_at ASC, id ASC);
         CREATE INDEX IF NOT EXISTS idx_hr_wallet_date ON history_records(wallet_id, created_at DESC, id ASC);
         CREATE INDEX IF NOT EXISTS idx_hr_wallet_oldest ON history_records(wallet_id, created_at ASC, id ASC);
         CREATE INDEX IF NOT EXISTS idx_hr_identity ON history_records
             (wallet_id, chain_name, asset_key, hash_key, status_rank DESC, created_at DESC, id ASC);
         CREATE INDEX IF NOT EXISTS idx_hr_status_date ON history_records
             (json_extract(payload, '$.status'), created_at DESC, id);
         CREATE INDEX IF NOT EXISTS idx_hr_pending_sender ON history_records
             (chain_name, lower(json_extract(payload, '$.sourceAddress')))
             WHERE json_extract(payload, '$.kind') = 'send' AND json_extract(payload, '$.status') = 'pending';
         CREATE INDEX IF NOT EXISTS idx_hr_source_path ON history_records
             (wallet_id, chain_name, json_extract(payload, '$.sourceDerivationPath'));
         CREATE INDEX IF NOT EXISTS idx_hr_change_path ON history_records
             (wallet_id, chain_name, json_extract(payload, '$.changeDerivationPath'));
         CREATE TABLE IF NOT EXISTS wallets (
             id                         TEXT    NOT NULL PRIMARY KEY,
             name                       TEXT    NOT NULL,
             chain_name                 TEXT    NOT NULL,
             is_watch_only              INTEGER NOT NULL DEFAULT 0,
             include_in_portfolio_total INTEGER NOT NULL DEFAULT 1,
             sort_index                 INTEGER NOT NULL,  -- preserves CoreAppState.wallets order
             payload                    TEXT    NOT NULL,  -- full WalletState JSON
             updated_at                 INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_wallets_lower_id ON wallets(lower(id));
         CREATE INDEX IF NOT EXISTS idx_wallets_chain ON wallets(chain_name);
         CREATE INDEX IF NOT EXISTS idx_wallets_order ON wallets(sort_index);
         CREATE TABLE IF NOT EXISTS app_state_meta (
             key   TEXT NOT NULL PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS address_book (
             id         TEXT    NOT NULL PRIMARY KEY,
             chain_name TEXT    NOT NULL,
             address    TEXT    NOT NULL,
             sort_index INTEGER NOT NULL,  -- preserves CoreAppState.address_book order
             payload    TEXT    NOT NULL,  -- full AddressBookEntry JSON
             updated_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_ab_chain ON address_book(chain_name);
         CREATE INDEX IF NOT EXISTS idx_ab_order ON address_book(sort_index);",
    )
    .map_err(|e| format!("wallet_db create tables: {e}"))?;
    Ok(conn)
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
