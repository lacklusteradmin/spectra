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
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::state::{AddressBookEntry, CoreAppState, WalletSummary};

// Re-uses a single Connection per db_path instead of opening (and running DDL)
// on every call.  The Mutex is uncontended in practice because all wallet_db
// callers already run inside `spawn_blocking`.
//
// Uses `parking_lot::Mutex` — no poisoning, smaller footprint, and faster
// uncontended lock/unlock than `std::sync::Mutex`.

static POOL: std::sync::LazyLock<Mutex<HashMap<String, Connection>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn with_conn<T>(
    db_path: &str,
    f: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    use std::collections::hash_map::Entry;
    let mut pool = POOL.lock();
    let conn = match pool.entry(db_path.to_string()) {
        Entry::Occupied(e) => e.into_mut(),
        Entry::Vacant(e) => e.insert(open_new(db_path)?),
    };
    f(conn)
}

fn open_new(db_path: &str) -> Result<Connection, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("wallet_db open {db_path}: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
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
         CREATE TABLE IF NOT EXISTS wallets (
             id                         TEXT    NOT NULL PRIMARY KEY,
             name                       TEXT    NOT NULL,
             chain_name                 TEXT    NOT NULL,
             is_watch_only              INTEGER NOT NULL DEFAULT 0,
             include_in_portfolio_total INTEGER NOT NULL DEFAULT 1,
             sort_index                 INTEGER NOT NULL,  -- preserves CoreAppState.wallets order
             payload                    TEXT    NOT NULL,  -- full WalletSummary JSON
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

// ── Keypool types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct KeypoolState {
    pub next_external_index: i64,
    pub next_change_index: i64,
    pub reserved_receive_index: Option<i64>,
}

/// Full keypool snapshot for one wallet across all chains.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletKeypoolSnapshot {
    pub wallet_id: String,
    /// chain_name → state
    pub chains: std::collections::HashMap<String, KeypoolState>,
}

// ── Keypool CRUD ──────────────────────────────────────────────────────────────

/// Upsert keypool state for one (wallet, chain) pair.
pub fn keypool_save(
    db_path: &str,
    wallet_id: &str,
    chain_name: &str,
    state: &KeypoolState,
) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "INSERT INTO wallet_keypool
                 (wallet_id, chain_name, next_external_index, next_change_index,
                  reserved_receive_index, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(wallet_id, chain_name) DO UPDATE SET
                 next_external_index    = excluded.next_external_index,
                 next_change_index      = excluded.next_change_index,
                 reserved_receive_index = excluded.reserved_receive_index,
                 updated_at             = excluded.updated_at",
            params![
                wallet_id,
                chain_name,
                state.next_external_index,
                state.next_change_index,
                state.reserved_receive_index,
                now_secs(),
            ],
        )
        .map_err(|e| format!("keypool_save: {e}"))?;
        Ok(())
    })
}

/// Load keypool state for one (wallet, chain) pair.
pub fn keypool_load(
    db_path: &str,
    wallet_id: &str,
    chain_name: &str,
) -> Result<Option<KeypoolState>, String> {
    with_conn(db_path, |conn| {
        let result = conn.query_row(
            "SELECT next_external_index, next_change_index, reserved_receive_index
             FROM wallet_keypool WHERE wallet_id = ?1 AND chain_name = ?2",
            params![wallet_id, chain_name],
            |row| {
                Ok(KeypoolState {
                    next_external_index: row.get(0)?,
                    next_change_index: row.get(1)?,
                    reserved_receive_index: row.get(2)?,
                })
            },
        );
        match result {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("keypool_load: {e}")),
        }
    })
}

/// Load all keypool state for a wallet across every chain it has used.
pub fn keypool_load_for_wallet(
    db_path: &str,
    wallet_id: &str,
) -> Result<std::collections::HashMap<String, KeypoolState>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT chain_name, next_external_index, next_change_index, reserved_receive_index
                 FROM wallet_keypool WHERE wallet_id = ?1",
            )
            .map_err(|e| format!("keypool_load_for_wallet prepare: {e}"))?;
        let rows = stmt
            .query_map(params![wallet_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    KeypoolState {
                        next_external_index: row.get(1)?,
                        next_change_index: row.get(2)?,
                        reserved_receive_index: row.get(3)?,
                    },
                ))
            })
            .map_err(|e| format!("keypool_load_for_wallet query: {e}"))?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (chain, state) = row.map_err(|e| format!("keypool_load_for_wallet row: {e}"))?;
            map.insert(chain, state);
        }
        Ok(map)
    })
}

/// Load all keypool state across every wallet for a given chain.
pub fn keypool_load_for_chain(
    db_path: &str,
    chain_name: &str,
) -> Result<std::collections::HashMap<String, KeypoolState>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT wallet_id, next_external_index, next_change_index, reserved_receive_index
                 FROM wallet_keypool WHERE chain_name = ?1",
            )
            .map_err(|e| format!("keypool_load_for_chain prepare: {e}"))?;
        let rows = stmt
            .query_map(params![chain_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    KeypoolState {
                        next_external_index: row.get(1)?,
                        next_change_index: row.get(2)?,
                        reserved_receive_index: row.get(3)?,
                    },
                ))
            })
            .map_err(|e| format!("keypool_load_for_chain query: {e}"))?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (wallet, state) = row.map_err(|e| format!("keypool_load_for_chain row: {e}"))?;
            map.insert(wallet, state);
        }
        Ok(map)
    })
}

/// Load the entire keypool table as a nested map: chain → wallet_id → state.
/// This is the startup bulk-load that replaces reading UserDefaults JSON.
pub fn keypool_load_all(
    db_path: &str,
) -> Result<
    std::collections::HashMap<String, std::collections::HashMap<String, KeypoolState>>,
    String,
> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT chain_name, wallet_id, next_external_index, next_change_index, reserved_receive_index
                 FROM wallet_keypool",
            )
            .map_err(|e| format!("keypool_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?, // chain_name
                    row.get::<_, String>(1)?, // wallet_id
                    KeypoolState {
                        next_external_index: row.get(2)?,
                        next_change_index: row.get(3)?,
                        reserved_receive_index: row.get(4)?,
                    },
                ))
            })
            .map_err(|e| format!("keypool_load_all query: {e}"))?;
        let mut outer: std::collections::HashMap<
            String,
            std::collections::HashMap<String, KeypoolState>,
        > = std::collections::HashMap::new();
        for row in rows {
            let (chain, wallet, state) = row.map_err(|e| format!("keypool_load_all row: {e}"))?;
            outer.entry(chain).or_default().insert(wallet, state);
        }
        Ok(outer)
    })
}

/// Remove all keypool entries for a deleted wallet.
pub fn keypool_delete_for_wallet(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_keypool WHERE wallet_id = ?1",
            params![wallet_id],
        )
        .map_err(|e| format!("keypool_delete_for_wallet: {e}"))?;
        Ok(())
    })
}

/// Remove all keypool entries for a chain (e.g. when the user switches network modes).
pub fn keypool_delete_for_chain(db_path: &str, chain_name: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_keypool WHERE chain_name = ?1",
            params![chain_name],
        )
        .map_err(|e| format!("keypool_delete_for_chain: {e}"))?;
        Ok(())
    })
}

/// Wipe the entire keypool table (full reset).
pub fn keypool_delete_all(db_path: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute("DELETE FROM wallet_keypool", [])
            .map_err(|e| format!("keypool_delete_all: {e}"))?;
        Ok(())
    })
}

// ── Owned address types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct OwnedAddressRecord {
    pub wallet_id: String,
    pub chain_name: String,
    pub address: String,
    pub derivation_path: Option<String>,
    pub branch: Option<String>,
    pub branch_index: Option<i64>,
}

// ── Owned address CRUD ────────────────────────────────────────────────────────

/// Upsert a single owned address record (identified by wallet + chain + address).
pub fn address_save(db_path: &str, record: &OwnedAddressRecord) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "INSERT INTO wallet_owned_addresses
                 (wallet_id, chain_name, address, derivation_path, branch, branch_index, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(wallet_id, chain_name, address) DO UPDATE SET
                 derivation_path = excluded.derivation_path,
                 branch          = excluded.branch,
                 branch_index    = excluded.branch_index,
                 updated_at      = excluded.updated_at",
            params![
                record.wallet_id,
                record.chain_name,
                record.address,
                record.derivation_path,
                record.branch,
                record.branch_index,
                now_secs(),
            ],
        )
        .map_err(|e| format!("address_save: {e}"))?;
        Ok(())
    })
}

/// Load all owned addresses for a (wallet, chain) pair.
pub fn address_load_all(
    db_path: &str,
    wallet_id: &str,
    chain_name: &str,
) -> Result<Vec<OwnedAddressRecord>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT address, derivation_path, branch, branch_index
                 FROM wallet_owned_addresses WHERE wallet_id = ?1 AND chain_name = ?2",
            )
            .map_err(|e| format!("address_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map(params![wallet_id, chain_name], |row| {
                Ok(OwnedAddressRecord {
                    wallet_id: wallet_id.to_string(),
                    chain_name: chain_name.to_string(),
                    address: row.get(0)?,
                    derivation_path: row.get(1)?,
                    branch: row.get(2)?,
                    branch_index: row.get(3)?,
                })
            })
            .map_err(|e| format!("address_load_all query: {e}"))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|e| format!("address_load_all row: {e}"))?);
        }
        Ok(records)
    })
}

/// Load ALL owned address records across all wallets and chains.
/// Used at startup to bulk-restore the in-memory map.
pub fn address_load_all_chains(db_path: &str) -> Result<Vec<OwnedAddressRecord>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT wallet_id, chain_name, address, derivation_path, branch, branch_index
                 FROM wallet_owned_addresses",
            )
            .map_err(|e| format!("address_load_all_chains prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(OwnedAddressRecord {
                    wallet_id: row.get(0)?,
                    chain_name: row.get(1)?,
                    address: row.get(2)?,
                    derivation_path: row.get(3)?,
                    branch: row.get(4)?,
                    branch_index: row.get(5)?,
                })
            })
            .map_err(|e| format!("address_load_all_chains query: {e}"))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|e| format!("address_load_all_chains row: {e}"))?);
        }
        Ok(records)
    })
}

/// Remove all owned address records for a deleted wallet.
pub fn address_delete_for_wallet(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_owned_addresses WHERE wallet_id = ?1",
            params![wallet_id],
        )
        .map_err(|e| format!("address_delete_for_wallet: {e}"))?;
        Ok(())
    })
}

/// Remove all owned address records for a chain (e.g. after a rescan).
pub fn address_delete_for_chain(db_path: &str, chain_name: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_owned_addresses WHERE chain_name = ?1",
            params![chain_name],
        )
        .map_err(|e| format!("address_delete_for_chain: {e}"))?;
        Ok(())
    })
}

/// Wipe the owned address table (full reset).
pub fn address_delete_all(db_path: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute("DELETE FROM wallet_owned_addresses", [])
            .map_err(|e| format!("address_delete_all: {e}"))?;
        Ok(())
    })
}

// ── History record types ──────────────────────────────────────────────────────

/// Represents one persisted transaction record. `payload` is the typed
/// `CorePersistedTransactionRecord` directly — Rust serializes it to JSON
/// for the SQLite TEXT column and deserializes on read, so the JSON shape
/// never crosses the FFI as a String.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub id: String,
    pub wallet_id: Option<String>,
    pub chain_name: String,
    pub tx_hash: Option<String>,
    pub created_at: f64,
    pub payload: crate::store::persistence_models::CorePersistedTransactionRecord,
}

// ── History record CRUD ───────────────────────────────────────────────────────

/// Seconds between the Unix epoch and Swift's reference date (2001-01-01 UTC).
///
/// `CorePersistedTransactionRecord::created_at` is in Swift reference time
/// because that is what the persisted shape has always used; the
/// `history_records.created_at` column is Unix, because that is what every
/// other table and every chain API uses. The conversion lives here so no
/// front end has to remember which side of the boundary it is on — getting it
/// wrong silently misorders history by 31 years.
use crate::store::persistence_models::SWIFT_REFERENCE_EPOCH_OFFSET_SECS;

/// Build the indexed row for a transaction from the record itself.
///
/// The id / wallet id / tx hash are lowercased so lookups are case-insensitive
/// without every query having to say so.
pub fn history_record_from_payload(
    payload: crate::store::persistence_models::CorePersistedTransactionRecord,
) -> HistoryRecord {
    HistoryRecord {
        id: payload.id.to_lowercase(),
        wallet_id: payload.wallet_id.as_deref().map(str::to_lowercase),
        chain_name: payload.chain_name.clone(),
        tx_hash: payload.transaction_hash.as_deref().map(str::to_lowercase),
        created_at: payload.created_at + SWIFT_REFERENCE_EPOCH_OFFSET_SECS,
        payload,
    }
}

/// Which of `ids` already exist, lowercased.
pub fn history_existing_ids(db_path: &str, ids: &[String]) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare("SELECT id FROM history_records")
            .map_err(|e| format!("history_existing_ids prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("history_existing_ids query: {e}"))?;
        let wanted: std::collections::HashSet<String> =
            ids.iter().map(|id| id.to_lowercase()).collect();
        let mut found = Vec::new();
        for row in rows {
            let id = row.map_err(|e| format!("history_existing_ids row: {e}"))?;
            if wanted.contains(&id) {
                found.push(id);
            }
        }
        Ok(found)
    })
}

/// Load every transaction for one wallet, newest first.
pub fn history_fetch_for_wallet(
    db_path: &str,
    wallet_id: &str,
) -> Result<Vec<HistoryRecord>, String> {
    let wallet_id = wallet_id.to_lowercase();
    Ok(history_fetch_all(db_path)?
        .into_iter()
        .filter(|record| record.wallet_id.as_deref() == Some(wallet_id.as_str()))
        .collect())
}

/// Upsert a batch of history records. Existing rows (matched by `id`) are overwritten.
pub fn history_upsert_batch(db_path: &str, records: &[HistoryRecord]) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    with_conn(db_path, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("history_upsert_batch begin: {e}"))?;
        let result = (|| -> Result<(), String> {
            for rec in records {
                let payload_json = serde_json::to_string(&rec.payload)
                    .map_err(|e| format!("history_upsert_batch encode payload: {e}"))?;
                conn.execute(
                    "INSERT INTO history_records (id, wallet_id, chain_name, tx_hash, created_at, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                         wallet_id  = excluded.wallet_id,
                         chain_name = excluded.chain_name,
                         tx_hash    = excluded.tx_hash,
                         created_at = excluded.created_at,
                         payload    = excluded.payload",
                    params![rec.id, rec.wallet_id, rec.chain_name, rec.tx_hash, rec.created_at, payload_json],
                ).map_err(|e| format!("history_upsert_batch row: {e}"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT")
                    .map_err(|e| format!("history_upsert_batch commit: {e}"))?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    })
}

/// Fetch all history records ordered by created_at DESC.
pub fn history_fetch_all(db_path: &str) -> Result<Vec<HistoryRecord>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, wallet_id, chain_name, tx_hash, created_at, payload
                 FROM history_records ORDER BY created_at DESC, id ASC",
            )
            .map_err(|e| format!("history_fetch_all prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                let payload_json: String = row.get(5)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, f64>(4)?,
                    payload_json,
                ))
            })
            .map_err(|e| format!("history_fetch_all query: {e}"))?;
        let mut records = Vec::new();
        for row in rows {
            let (id, wallet_id, chain_name, tx_hash, created_at, payload_json) =
                row.map_err(|e| format!("history_fetch_all row: {e}"))?;
            let payload = serde_json::from_str(&payload_json)
                .map_err(|e| format!("history_fetch_all decode payload: {e}"))?;
            records.push(HistoryRecord {
                id,
                wallet_id,
                chain_name,
                tx_hash,
                created_at,
                payload,
            });
        }
        Ok(records)
    })
}

/// Delete history records by ID list.
pub fn history_delete(db_path: &str, ids: &[String]) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    with_conn(db_path, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("history_delete begin: {e}"))?;
        let result = (|| -> Result<(), String> {
            for id in ids {
                conn.execute("DELETE FROM history_records WHERE id = ?1", params![id])
                    .map_err(|e| format!("history_delete row: {e}"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT")
                    .map_err(|e| format!("history_delete commit: {e}"))?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    })
}

/// Atomically delete all records then insert the provided batch (full replacement).
pub fn history_replace_all(db_path: &str, records: &[HistoryRecord]) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("history_replace_all begin: {e}"))?;
        let result = (|| -> Result<(), String> {
            conn.execute("DELETE FROM history_records", [])
                .map_err(|e| format!("history_replace_all delete: {e}"))?;
            for rec in records {
                let payload_json = serde_json::to_string(&rec.payload)
                    .map_err(|e| format!("history_replace_all encode payload: {e}"))?;
                conn.execute(
                    "INSERT INTO history_records (id, wallet_id, chain_name, tx_hash, created_at, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![rec.id, rec.wallet_id, rec.chain_name, rec.tx_hash, rec.created_at, payload_json],
                ).map_err(|e| format!("history_replace_all insert: {e}"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT")
                    .map_err(|e| format!("history_replace_all commit: {e}"))?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    })
}

/// Delete all history records for a given wallet_id.
pub fn history_delete_for_wallet(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM history_records WHERE wallet_id = ?1",
            params![wallet_id],
        )
        .map_err(|e| format!("history_delete_for_wallet: {e}"))?;
        Ok(())
    })
}

/// Delete all history records (hard reset).
pub fn history_clear(db_path: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute("DELETE FROM history_records", [])
            .map_err(|e| format!("history_clear: {e}"))?;
        Ok(())
    })
}

// ── App state (wallets + settings) ────────────────────────────────────────────
//
// This is the persistence layer for `store::state::CoreAppState` — the
// chain-agnostic wallet model. Before it existed, `CoreAppState` had no home in
// Rust at all: iOS persisted its own Swift-side model and the CLI wrote its own
// `wallets.json`, so "the wallet list" had two incompatible on-disk shapes and
// neither belonged to the core.
//
// Storage follows the `history_records` house style: identity and query columns
// are promoted, the rest of the record rides along as JSON in `payload`. Wallet
// order is explicit in `sort_index` because `CoreAppState.wallets` is a `Vec`
// and its order is user-visible.

const META_SCHEMA_VERSION: &str = "schema_version";
const META_SELECTED_WALLET_ID: &str = "selected_wallet_id";
const META_SETTINGS: &str = "settings";
/// Tracked tokens. A separate meta row rather than a field inside `settings`,
/// because `AppSettings` is the "every front end must agree" bag and this is a
/// list the user edits.
const META_TOKEN_PREFERENCES: &str = "token_preferences";
const META_PRICE_ALERTS: &str = "price_alerts";

/// Insert or replace one wallet, keeping its existing position when it is
/// already stored and appending it to the end when it is new.
///
/// Use this for a single-wallet edit. To write a whole state snapshot use
/// [`app_state_save`], which also prunes wallets that are no longer present.
pub fn wallet_upsert(db_path: &str, wallet: &WalletSummary) -> Result<(), String> {
    let payload =
        serde_json::to_string(wallet).map_err(|e| format!("wallet_upsert encode: {e}"))?;
    with_conn(db_path, |conn| {
        let next_index: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_index) + 1, 0) FROM wallets",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("wallet_upsert next index: {e}"))?;
        conn.execute(
            "INSERT INTO wallets
                 (id, name, chain_name, is_watch_only, include_in_portfolio_total,
                  sort_index, payload, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 name                       = excluded.name,
                 chain_name                 = excluded.chain_name,
                 is_watch_only              = excluded.is_watch_only,
                 include_in_portfolio_total = excluded.include_in_portfolio_total,
                 payload                    = excluded.payload,
                 updated_at                 = excluded.updated_at",
            params![
                wallet.id,
                wallet.name,
                wallet.chain_name,
                wallet.is_watch_only,
                wallet.include_in_portfolio_total,
                next_index,
                payload,
                now_secs(),
            ],
        )
        .map_err(|e| format!("wallet_upsert: {e}"))?;
        Ok(())
    })
}

/// Load one wallet by id.
pub fn wallet_load(db_path: &str, wallet_id: &str) -> Result<Option<WalletSummary>, String> {
    with_conn(db_path, |conn| {
        let result = conn.query_row(
            "SELECT payload FROM wallets WHERE id = ?1",
            params![wallet_id],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(payload) => serde_json::from_str(&payload)
                .map(Some)
                .map_err(|e| format!("wallet_load decode {wallet_id}: {e}")),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("wallet_load: {e}")),
        }
    })
}

/// Load every wallet, in the stored display order.
pub fn wallet_load_all(db_path: &str) -> Result<Vec<WalletSummary>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare("SELECT id, payload FROM wallets ORDER BY sort_index ASC")
            .map_err(|e| format!("wallet_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("wallet_load_all query: {e}"))?;
        let mut wallets = Vec::new();
        for row in rows {
            let (id, payload) = row.map_err(|e| format!("wallet_load_all row: {e}"))?;
            wallets.push(
                serde_json::from_str(&payload)
                    .map_err(|e| format!("wallet_load_all decode {id}: {e}"))?,
            );
        }
        Ok(wallets)
    })
}

/// Delete one wallet row. Does not touch that wallet's keypool, owned addresses
/// or history — use [`delete_wallet_data`] for the full teardown.
pub fn wallet_delete(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute("DELETE FROM wallets WHERE id = ?1", params![wallet_id])
            .map_err(|e| format!("wallet_delete: {e}"))?;
        Ok(())
    })
}

/// Persist a whole [`CoreAppState`] snapshot.
///
/// Replaces the stored wallet set: wallets absent from `state` are removed, and
/// `sort_index` is rewritten so the stored order matches `state.wallets`. Runs
/// in one transaction, so a failure part-way leaves the previous snapshot
/// intact rather than a half-written wallet list.
pub fn app_state_save(db_path: &str, state: &CoreAppState) -> Result<(), String> {
    let settings = serde_json::to_string(&state.settings)
        .map_err(|e| format!("app_state_save encode settings: {e}"))?;
    let token_preferences = serde_json::to_string(&state.token_preferences)
        .map_err(|e| format!("app_state_save encode token_preferences: {e}"))?;
    let price_alerts = serde_json::to_string(&state.price_alerts)
        .map_err(|e| format!("app_state_save encode price_alerts: {e}"))?;
    let payloads: Vec<(usize, &WalletSummary, String)> = state
        .wallets
        .iter()
        .enumerate()
        .map(|(index, wallet)| {
            serde_json::to_string(wallet)
                .map(|payload| (index, wallet, payload))
                .map_err(|e| format!("app_state_save encode wallet {}: {e}", wallet.id))
        })
        .collect::<Result<_, _>>()?;

    with_conn(db_path, |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("app_state_save begin: {e}"))?;
        let updated_at = now_secs();

        tx.execute("DELETE FROM wallets", [])
            .map_err(|e| format!("app_state_save clear wallets: {e}"))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO wallets
                         (id, name, chain_name, is_watch_only, include_in_portfolio_total,
                          sort_index, payload, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(|e| format!("app_state_save prepare: {e}"))?;
            for (index, wallet, payload) in &payloads {
                stmt.execute(params![
                    wallet.id,
                    wallet.name,
                    wallet.chain_name,
                    wallet.is_watch_only,
                    wallet.include_in_portfolio_total,
                    *index as i64,
                    payload,
                    updated_at,
                ])
                .map_err(|e| format!("app_state_save insert {}: {e}", wallet.id))?;
            }
        }

        tx.execute("DELETE FROM address_book", [])
            .map_err(|e| format!("app_state_save clear address book: {e}"))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO address_book
                         (id, chain_name, address, sort_index, payload, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(|e| format!("app_state_save prepare address book: {e}"))?;
            for (index, entry) in state.address_book.iter().enumerate() {
                let payload = serde_json::to_string(entry).map_err(|e| {
                    format!("app_state_save encode address entry {}: {e}", entry.id)
                })?;
                stmt.execute(params![
                    entry.id,
                    entry.chain_name,
                    entry.address,
                    index as i64,
                    payload,
                    updated_at,
                ])
                .map_err(|e| format!("app_state_save insert address entry {}: {e}", entry.id))?;
            }
        }

        let mut meta = tx
            .prepare(
                "INSERT INTO app_state_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .map_err(|e| format!("app_state_save prepare meta: {e}"))?;
        meta.execute(params![
            META_SCHEMA_VERSION,
            state.schema_version.to_string()
        ])
        .map_err(|e| format!("app_state_save schema_version: {e}"))?;
        meta.execute(params![META_PRICE_ALERTS, price_alerts])
            .map_err(|e| format!("app_state_save price_alerts: {e}"))?;
        meta.execute(params![META_TOKEN_PREFERENCES, token_preferences])
            .map_err(|e| format!("app_state_save token_preferences: {e}"))?;
        meta.execute(params![META_SETTINGS, settings])
            .map_err(|e| format!("app_state_save settings: {e}"))?;
        // Absent selection is stored as an absent row, not an empty string, so
        // "no wallet selected" and "a wallet whose id is empty" stay distinct.
        match &state.selected_wallet_id {
            Some(id) => meta
                .execute(params![META_SELECTED_WALLET_ID, id])
                .map(|_| ())
                .map_err(|e| format!("app_state_save selected_wallet_id: {e}"))?,
            None => tx
                .execute(
                    "DELETE FROM app_state_meta WHERE key = ?1",
                    params![META_SELECTED_WALLET_ID],
                )
                .map(|_| ())
                .map_err(|e| format!("app_state_save clear selected_wallet_id: {e}"))?,
        }
        drop(meta);

        tx.commit()
            .map_err(|e| format!("app_state_save commit: {e}"))
    })
}

/// Load every saved recipient, in the stored display order.
pub fn address_book_load_all(db_path: &str) -> Result<Vec<AddressBookEntry>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare("SELECT id, payload FROM address_book ORDER BY sort_index ASC")
            .map_err(|e| format!("address_book_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("address_book_load_all query: {e}"))?;
        let mut entries = Vec::new();
        for row in rows {
            let (id, payload) = row.map_err(|e| format!("address_book_load_all row: {e}"))?;
            entries.push(
                serde_json::from_str(&payload)
                    .map_err(|e| format!("address_book_load_all decode {id}: {e}"))?,
            );
        }
        Ok(entries)
    })
}

/// Load the persisted [`CoreAppState`].
///
/// An untouched database loads as `CoreAppState::default()`, so first run needs
/// no special-casing at the call site.
pub fn app_state_load(db_path: &str) -> Result<CoreAppState, String> {
    let wallets = wallet_load_all(db_path)?;
    let address_book = address_book_load_all(db_path)?;
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare("SELECT key, value FROM app_state_meta")
            .map_err(|e| format!("app_state_load prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("app_state_load query: {e}"))?;

        let mut state = CoreAppState {
            wallets,
            address_book,
            ..CoreAppState::default()
        };
        for row in rows {
            let (key, value) = row.map_err(|e| format!("app_state_load row: {e}"))?;
            match key.as_str() {
                META_SCHEMA_VERSION => {
                    state.schema_version = value
                        .parse()
                        .map_err(|e| format!("app_state_load schema_version {value:?}: {e}"))?;
                }
                META_SELECTED_WALLET_ID => state.selected_wallet_id = Some(value),
                META_SETTINGS => {
                    state.settings = serde_json::from_str(&value)
                        .map_err(|e| format!("app_state_load settings: {e}"))?;
                }
                META_TOKEN_PREFERENCES => {
                    state.token_preferences = serde_json::from_str(&value)
                        .map_err(|e| format!("app_state_load token_preferences: {e}"))?;
                }
                META_PRICE_ALERTS => {
                    state.price_alerts = serde_json::from_str(&value)
                        .map_err(|e| format!("app_state_load price_alerts: {e}"))?;
                }
                // Forward compatibility: a newer build's extra meta keys are
                // ignored rather than treated as corruption.
                _ => {}
            }
        }
        Ok(state)
    })
}

// ── Combined wallet teardown ──────────────────────────────────────────────────

/// Remove every trace of a deleted wallet: its row, keypool, owned addresses
/// and history records.
pub fn delete_wallet_data(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        for (table, column) in [
            ("wallets", "id"),
            ("wallet_keypool", "wallet_id"),
            ("wallet_owned_addresses", "wallet_id"),
            ("history_records", "wallet_id"),
        ] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE {column} = ?1"),
                params![wallet_id],
            )
            .map_err(|e| format!("delete_wallet_data {table}: {e}"))?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database no other test can be holding.
    ///
    /// This used to key on `subsec_nanos()` alone. Thirteen tests share the
    /// helper and the runner runs them in parallel, so two could land on the
    /// same nanosecond and read each other's rows — which is exactly how
    /// `app_state_round_trips` failed in a full run and passed on its own.
    /// Process, thread and a counter cannot collide.
    fn tmp_db() -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "wallet_db_test_{}_{:?}_{}.sqlite",
            std::process::id(),
            std::thread::current().id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn keypool_round_trip() {
        let db = tmp_db();
        let state = KeypoolState {
            next_external_index: 5,
            next_change_index: 2,
            reserved_receive_index: Some(4),
        };
        keypool_save(&db, "wallet-1", "Bitcoin", &state).unwrap();
        let loaded = keypool_load(&db, "wallet-1", "Bitcoin").unwrap().unwrap();
        assert_eq!(loaded.next_external_index, 5);
        assert_eq!(loaded.next_change_index, 2);
        assert_eq!(loaded.reserved_receive_index, Some(4));
    }

    #[test]
    fn keypool_upsert_updates_existing() {
        let db = tmp_db();
        let first = KeypoolState {
            next_external_index: 0,
            next_change_index: 0,
            reserved_receive_index: None,
        };
        keypool_save(&db, "wallet-1", "Dogecoin", &first).unwrap();
        let updated = KeypoolState {
            next_external_index: 10,
            next_change_index: 3,
            reserved_receive_index: Some(9),
        };
        keypool_save(&db, "wallet-1", "Dogecoin", &updated).unwrap();
        let loaded = keypool_load(&db, "wallet-1", "Dogecoin").unwrap().unwrap();
        assert_eq!(loaded.next_external_index, 10);
        assert_eq!(loaded.reserved_receive_index, Some(9));
    }

    #[test]
    fn keypool_load_all_groups_by_chain() {
        let db = tmp_db();
        keypool_save(
            &db,
            "w1",
            "Bitcoin",
            &KeypoolState {
                next_external_index: 1,
                next_change_index: 0,
                reserved_receive_index: None,
            },
        )
        .unwrap();
        keypool_save(
            &db,
            "w2",
            "Bitcoin",
            &KeypoolState {
                next_external_index: 2,
                next_change_index: 1,
                reserved_receive_index: None,
            },
        )
        .unwrap();
        keypool_save(
            &db,
            "w1",
            "Dogecoin",
            &KeypoolState {
                next_external_index: 5,
                next_change_index: 2,
                reserved_receive_index: Some(4),
            },
        )
        .unwrap();
        let all = keypool_load_all(&db).unwrap();
        assert_eq!(all["Bitcoin"]["w1"].next_external_index, 1);
        assert_eq!(all["Bitcoin"]["w2"].next_external_index, 2);
        assert_eq!(all["Dogecoin"]["w1"].reserved_receive_index, Some(4));
    }

    #[test]
    fn keypool_delete_for_wallet() {
        let db = tmp_db();
        keypool_save(
            &db,
            "w1",
            "Bitcoin",
            &KeypoolState {
                next_external_index: 5,
                next_change_index: 1,
                reserved_receive_index: None,
            },
        )
        .unwrap();
        keypool_save(
            &db,
            "w2",
            "Bitcoin",
            &KeypoolState {
                next_external_index: 3,
                next_change_index: 0,
                reserved_receive_index: None,
            },
        )
        .unwrap();
        super::keypool_delete_for_wallet(&db, "w1").unwrap();
        assert!(keypool_load(&db, "w1", "Bitcoin").unwrap().is_none());
        assert!(keypool_load(&db, "w2", "Bitcoin").unwrap().is_some());
    }

    #[test]
    fn address_round_trip() {
        let db = tmp_db();
        let rec = OwnedAddressRecord {
            wallet_id: "w1".to_string(),
            chain_name: "Bitcoin".to_string(),
            address: "bc1qtest".to_string(),
            derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
            branch: Some("external".to_string()),
            branch_index: Some(0),
        };
        address_save(&db, &rec).unwrap();
        let records = address_load_all(&db, "w1", "Bitcoin").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].address, "bc1qtest");
        assert_eq!(records[0].branch.as_deref(), Some("external"));
    }

    #[test]
    fn delete_wallet_data_removes_both_tables() {
        let db = tmp_db();
        keypool_save(
            &db,
            "w1",
            "Dogecoin",
            &KeypoolState {
                next_external_index: 1,
                next_change_index: 0,
                reserved_receive_index: None,
            },
        )
        .unwrap();
        address_save(
            &db,
            &OwnedAddressRecord {
                wallet_id: "w1".to_string(),
                chain_name: "Dogecoin".to_string(),
                address: "D1test".to_string(),
                derivation_path: None,
                branch: None,
                branch_index: None,
            },
        )
        .unwrap();
        delete_wallet_data(&db, "w1").unwrap();
        assert!(keypool_load(&db, "w1", "Dogecoin").unwrap().is_none());
        assert!(address_load_all(&db, "w1", "Dogecoin").unwrap().is_empty());
    }

    use super::super::state::{AppSettings, WalletAddress};

    /// Minimal history record. The payload is decoded from JSON rather than
    /// built field-by-field: `CorePersistedTransactionRecord` has ~30 fields of
    /// which only these are required, and going through serde keeps the helper
    /// honest about which ones those are.
    fn history_record(id: &str, wallet_id: &str) -> HistoryRecord {
        let payload = serde_json::from_value(serde_json::json!({
            "id": id,
            "walletId": wallet_id,
            "kind": "send",
            "walletName": "Wallet",
            "assetName": "Bitcoin",
            "symbol": "BTC",
            "chainName": "Bitcoin",
            "amount": 1.0,
            "address": "bc1qexample",
            "createdAt": 0.0,
        }))
        .expect("history payload fixture must match CorePersistedTransactionRecord");
        HistoryRecord {
            id: id.to_string(),
            wallet_id: Some(wallet_id.to_string()),
            chain_name: "Bitcoin".to_string(),
            tx_hash: Some(format!("hash-{id}")),
            created_at: 0.0,
            payload,
        }
    }

    fn wallet(id: &str, chain: &str) -> WalletSummary {
        WalletSummary {
            id: id.to_string(),
            name: format!("Wallet {id}"),
            is_watch_only: false,
            chain_name: chain.to_string(),
            include_in_portfolio_total: true,
            network_mode: None,
            xpub: None,
            derivation_preset: "default".to_string(),
            derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: vec![WalletAddress {
                chain_name: chain.to_string(),
                address: format!("addr-{id}"),
                kind: "receive".to_string(),
                derivation_path: None,
            }],
        }
    }

    #[test]
    fn app_state_load_on_empty_db_is_default() {
        let db = tmp_db();
        assert_eq!(app_state_load(&db).unwrap(), CoreAppState::default());
    }

    #[test]
    fn app_state_round_trips() {
        let db = tmp_db();
        let state = CoreAppState {
            schema_version: 2,
            wallets: vec![wallet("w1", "Bitcoin"), wallet("w2", "Ethereum")],
            selected_wallet_id: Some("w2".to_string()),
            settings: AppSettings {
                fiat_currency_code: "CNY".to_string(),
                pinned_dashboard_asset_symbols: vec!["BTC".to_string()],
                // Every other field is a settings field the blob used to hold;
                // `every_settings_field_round_trips` covers them together.
                ..AppSettings::default()
            },
            token_preferences: Vec::new(),
            price_alerts: Vec::new(),
            address_book: vec![AddressBookEntry {
                id: "ab1".to_string(),
                name: "Cold".to_string(),
                chain_name: "Bitcoin".to_string(),
                address: "bc1qexample".to_string(),
                note: "vault".to_string(),
            }],
        };
        app_state_save(&db, &state).unwrap();
        assert_eq!(app_state_load(&db).unwrap(), state);
    }

    #[test]
    fn app_state_save_preserves_wallet_order() {
        let db = tmp_db();
        // Ids deliberately out of lexicographic order, so a load that sorted by
        // id instead of position would fail here.
        let ordered = vec![
            wallet("zz", "Bitcoin"),
            wallet("aa", "Solana"),
            wallet("mm", "Sui"),
        ];
        let state = CoreAppState {
            wallets: ordered.clone(),
            ..CoreAppState::default()
        };
        app_state_save(&db, &state).unwrap();
        let ids: Vec<String> = app_state_load(&db)
            .unwrap()
            .wallets
            .iter()
            .map(|w| w.id.clone())
            .collect();
        assert_eq!(ids, vec!["zz", "aa", "mm"]);
    }

    #[test]
    fn app_state_save_prunes_removed_wallets() {
        let db = tmp_db();
        app_state_save(
            &db,
            &CoreAppState {
                wallets: vec![wallet("w1", "Bitcoin"), wallet("w2", "Ethereum")],
                selected_wallet_id: Some("w1".to_string()),
                ..CoreAppState::default()
            },
        )
        .unwrap();
        app_state_save(
            &db,
            &CoreAppState {
                wallets: vec![wallet("w2", "Ethereum")],
                ..CoreAppState::default()
            },
        )
        .unwrap();

        let loaded = app_state_load(&db).unwrap();
        assert_eq!(loaded.wallets.len(), 1);
        assert_eq!(loaded.wallets[0].id, "w2");
        assert!(wallet_load(&db, "w1").unwrap().is_none());
        // Clearing the selection must clear the stored row, not leave the stale
        // id behind.
        assert_eq!(loaded.selected_wallet_id, None);
    }

    #[test]
    fn wallet_upsert_appends_then_updates_in_place() {
        let db = tmp_db();
        wallet_upsert(&db, &wallet("w1", "Bitcoin")).unwrap();
        wallet_upsert(&db, &wallet("w2", "Ethereum")).unwrap();

        let mut renamed = wallet("w1", "Bitcoin");
        renamed.name = "Renamed".to_string();
        renamed.include_in_portfolio_total = false;
        wallet_upsert(&db, &renamed).unwrap();

        let all = wallet_load_all(&db).unwrap();
        assert_eq!(all.len(), 2, "upsert must not duplicate an existing wallet");
        // w1 keeps position 0 across the update.
        assert_eq!(all[0].id, "w1");
        assert_eq!(all[0].name, "Renamed");
        assert!(!all[0].include_in_portfolio_total);
        assert_eq!(all[1].id, "w2");
    }

    #[test]
    fn delete_wallet_data_removes_the_wallet_row_and_its_history() {
        let db = tmp_db();
        app_state_save(
            &db,
            &CoreAppState {
                wallets: vec![wallet("w1", "Bitcoin"), wallet("w2", "Bitcoin")],
                ..CoreAppState::default()
            },
        )
        .unwrap();
        history_upsert_batch(
            &db,
            &[history_record("tx1", "w1"), history_record("tx2", "w2")],
        )
        .unwrap();

        delete_wallet_data(&db, "w1").unwrap();

        assert!(wallet_load(&db, "w1").unwrap().is_none());
        assert!(wallet_load(&db, "w2").unwrap().is_some());
        let remaining: Vec<String> = history_fetch_all(&db)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(remaining, vec!["tx2"], "only w1's history should be gone");
    }
}
