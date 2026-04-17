//! SQLite-backed relational store for per-wallet UTXO state.
//!
//! Replaces four UserDefaults JSON blobs that Swift's WalletStore currently owns:
//!   - `dogecoin.keypool.snapshot`          → `wallet_keypool` table
//!   - `chain.keypool.snapshot.v1`          → `wallet_keypool` table
//!   - `dogecoin.ownedAddressMap.snapshot`  → `wallet_owned_addresses` table
//!   - `chain.ownedAddressMap.snapshot.v1`  → `wallet_owned_addresses` table
//!
//! All functions are synchronous (call from `spawn_blocking` in service.rs).
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

// ── Connection pool ──────────────────────────────────────────────────────────
//
// Re-uses a single Connection per db_path instead of opening (and running DDL)
// on every call.  The Mutex is uncontended in practice because all wallet_db
// callers already run inside `spawn_blocking`.
//
// Uses `parking_lot::Mutex` — no poisoning, smaller footprint, and faster
// uncontended lock/unlock than `std::sync::Mutex`.

static POOL: std::sync::LazyLock<Mutex<HashMap<String, Connection>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn with_conn<T>(db_path: &str, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let mut pool = POOL.lock();
    if !pool.contains_key(db_path) {
        let conn = open_new(db_path)?;
        pool.insert(db_path.to_string(), conn);
    }
    f(pool.get(db_path).unwrap())
}

fn open_new(db_path: &str) -> Result<Connection, String> {
    let conn = Connection::open(db_path)
        .map_err(|e| format!("wallet_db open {db_path}: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
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
         CREATE INDEX IF NOT EXISTS idx_hr_created ON history_records(created_at DESC);",
    )
    .map_err(|e| format!("wallet_db create tables: {e}"))?;
    Ok(conn)
}

fn now_secs() -> i64 {
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
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, KeypoolState>>, String> {
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
        let mut outer: std::collections::HashMap<String, std::collections::HashMap<String, KeypoolState>> =
            std::collections::HashMap::new();
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
pub fn address_save(
    db_path: &str,
    record: &OwnedAddressRecord,
) -> Result<(), String> {
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
pub fn address_load_all_chains(
    db_path: &str,
) -> Result<Vec<OwnedAddressRecord>, String> {
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

/// Represents one persisted transaction record.
/// `payload` is a base64-encoded JSON blob (the full `PersistedCoreTransactionRecord` from Swift).
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub id: String,
    pub wallet_id: Option<String>,
    pub chain_name: String,
    pub tx_hash: Option<String>,
    pub created_at: f64,
    pub payload: String,
}

// ── History record CRUD ───────────────────────────────────────────────────────

/// Upsert a batch of history records. Existing rows (matched by `id`) are overwritten.
pub fn history_upsert_batch(db_path: &str, records: &[HistoryRecord]) -> Result<(), String> {
    if records.is_empty() { return Ok(()); }
    with_conn(db_path, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| format!("history_upsert_batch begin: {e}"))?;
        let result = (|| -> Result<(), String> {
            for rec in records {
                conn.execute(
                    "INSERT INTO history_records (id, wallet_id, chain_name, tx_hash, created_at, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                         wallet_id  = excluded.wallet_id,
                         chain_name = excluded.chain_name,
                         tx_hash    = excluded.tx_hash,
                         created_at = excluded.created_at,
                         payload    = excluded.payload",
                    params![rec.id, rec.wallet_id, rec.chain_name, rec.tx_hash, rec.created_at, rec.payload],
                ).map_err(|e| format!("history_upsert_batch row: {e}"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT").map_err(|e| format!("history_upsert_batch commit: {e}"))?;
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
                Ok(HistoryRecord {
                    id: row.get(0)?,
                    wallet_id: row.get(1)?,
                    chain_name: row.get(2)?,
                    tx_hash: row.get(3)?,
                    created_at: row.get(4)?,
                    payload: row.get(5)?,
                })
            })
            .map_err(|e| format!("history_fetch_all query: {e}"))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|e| format!("history_fetch_all row: {e}"))?);
        }
        Ok(records)
    })
}

/// Delete history records by ID list.
pub fn history_delete(db_path: &str, ids: &[String]) -> Result<(), String> {
    if ids.is_empty() { return Ok(()); }
    with_conn(db_path, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| format!("history_delete begin: {e}"))?;
        let result = (|| -> Result<(), String> {
            for id in ids {
                conn.execute("DELETE FROM history_records WHERE id = ?1", params![id])
                    .map_err(|e| format!("history_delete row: {e}"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT").map_err(|e| format!("history_delete commit: {e}"))?;
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
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| format!("history_replace_all begin: {e}"))?;
        let result = (|| -> Result<(), String> {
            conn.execute("DELETE FROM history_records", []).map_err(|e| format!("history_replace_all delete: {e}"))?;
            for rec in records {
                conn.execute(
                    "INSERT INTO history_records (id, wallet_id, chain_name, tx_hash, created_at, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![rec.id, rec.wallet_id, rec.chain_name, rec.tx_hash, rec.created_at, rec.payload],
                ).map_err(|e| format!("history_replace_all insert: {e}"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT").map_err(|e| format!("history_replace_all commit: {e}"))?;
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
        conn.execute("DELETE FROM history_records WHERE wallet_id = ?1", params![wallet_id])
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

// ── Combined wallet teardown ──────────────────────────────────────────────────

/// Remove all relational wallet state (keypool + addresses) for a deleted wallet.
pub fn delete_wallet_data(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_keypool WHERE wallet_id = ?1",
            params![wallet_id],
        )
        .map_err(|e| format!("delete_wallet_data keypool: {e}"))?;
        conn.execute(
            "DELETE FROM wallet_owned_addresses WHERE wallet_id = ?1",
            params![wallet_id],
        )
        .map_err(|e| format!("delete_wallet_data addresses: {e}"))?;
        Ok(())
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> String {
        let path = std::env::temp_dir().join(format!(
            "wallet_db_test_{}.sqlite",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
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
        keypool_save(&db, "w1", "Bitcoin", &KeypoolState { next_external_index: 1, next_change_index: 0, reserved_receive_index: None }).unwrap();
        keypool_save(&db, "w2", "Bitcoin", &KeypoolState { next_external_index: 2, next_change_index: 1, reserved_receive_index: None }).unwrap();
        keypool_save(&db, "w1", "Dogecoin", &KeypoolState { next_external_index: 5, next_change_index: 2, reserved_receive_index: Some(4) }).unwrap();
        let all = keypool_load_all(&db).unwrap();
        assert_eq!(all["Bitcoin"]["w1"].next_external_index, 1);
        assert_eq!(all["Bitcoin"]["w2"].next_external_index, 2);
        assert_eq!(all["Dogecoin"]["w1"].reserved_receive_index, Some(4));
    }

    #[test]
    fn keypool_delete_for_wallet() {
        let db = tmp_db();
        keypool_save(&db, "w1", "Bitcoin", &KeypoolState { next_external_index: 5, next_change_index: 1, reserved_receive_index: None }).unwrap();
        keypool_save(&db, "w2", "Bitcoin", &KeypoolState { next_external_index: 3, next_change_index: 0, reserved_receive_index: None }).unwrap();
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
        keypool_save(&db, "w1", "Dogecoin", &KeypoolState { next_external_index: 1, next_change_index: 0, reserved_receive_index: None }).unwrap();
        address_save(&db, &OwnedAddressRecord {
            wallet_id: "w1".to_string(),
            chain_name: "Dogecoin".to_string(),
            address: "D1test".to_string(),
            derivation_path: None,
            branch: None,
            branch_index: None,
        }).unwrap();
        delete_wallet_data(&db, "w1").unwrap();
        assert!(keypool_load(&db, "w1", "Dogecoin").unwrap().is_none());
        assert!(address_load_all(&db, "w1", "Dogecoin").unwrap().is_empty());
    }
}
