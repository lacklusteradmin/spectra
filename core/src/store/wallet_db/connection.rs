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
    let conn = Connection::open(database_path).map_err(|e| format!("wallet_db open {database_path}: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("wallet_db busy timeout: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         CREATE TABLE IF NOT EXISTS state (key TEXT PRIMARY KEY, value TEXT NOT NULL, saved_at INTEGER NOT NULL);
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
             payload    TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_hr_wallet  ON history_records(wallet_id);
         CREATE INDEX IF NOT EXISTS idx_hr_chain   ON history_records(chain_name);
         CREATE INDEX IF NOT EXISTS idx_hr_created ON history_records(created_at DESC);
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
