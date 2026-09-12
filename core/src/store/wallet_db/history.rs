use super::*;
use crate::store::persistence_models::CorePersistedTransactionRecord;

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

/// Project distinct indexed paths, not transaction payloads. Repeated transactions
/// on one address produce one path to parse, scoped to this wallet and chain.
pub(crate) fn history_keypool_indices(
    db_path: &str,
    wallet_id: &str,
    chain_name: &str,
) -> Result<(Option<i32>, Option<i32>), String> {
    with_conn(db_path, |conn| {
        let mut maxima = [None, None];
        for (branch, field) in ["sourceDerivationPath", "changeDerivationPath"]
            .into_iter()
            .enumerate()
        {
            let sql = format!(
                "SELECT DISTINCT json_extract(payload, '$.{field}')
                FROM history_records WHERE wallet_id = ?1 AND chain_name = ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![wallet_id.to_lowercase(), chain_name], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .map_err(|e| e.to_string())?;
            for path in rows {
                if let Some(index) = path
                    .map_err(|e| e.to_string())?
                    .as_deref()
                    .and_then(|path| {
                        crate::app_core::utxo_discovery_index(path, chain_name, branch as u32)
                    })
                {
                    let index = i32::try_from(index).map_err(|_| "keypool index out of range")?;
                    maxima[branch] = Some(maxima[branch].map_or(index, |old: i32| old.max(index)));
                }
            }
        }
        Ok((maxima[0], maxima[1]))
    })
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
    history_fetch_where(db_path, "wallet_id = ?1", params![wallet_id.to_lowercase()])
}

/// Every stored record for one chain, across every wallet — what a history
/// merge needs to compare an incoming page against.
///
/// A merge only ever matches within one chain (`matches_identity` checks
/// `chain_name` first, before anything else), so fetching every other chain's
/// history alongside it was pure waste: rows this call will reject, paid for
/// in a SQL round trip and a JSON decode each, on every refresh cycle.
pub fn history_fetch_for_chain(
    db_path: &str,
    chain_name: &str,
) -> Result<Vec<HistoryRecord>, String> {
    history_fetch_where(db_path, "chain_name = ?1", params![chain_name])
}

/// Shared body for `history_fetch_all` and the scoped fetches: same query,
/// same row decode, a different `WHERE` — pushed to SQL and `idx_hr_wallet` /
/// `idx_hr_chain` rather than fetched whole and filtered in Rust, which is
/// what every caller here did until each was found reading the whole table
/// for one wallet's or one chain's worth of rows.
fn history_fetch_where(
    db_path: &str,
    predicate: &str,
    query_params: impl rusqlite::Params,
) -> Result<Vec<HistoryRecord>, String> {
    with_conn(db_path, |conn| {
        let sql = format!(
            "SELECT id, wallet_id, chain_name, tx_hash, created_at, payload
             FROM history_records WHERE {predicate} ORDER BY created_at DESC, id ASC"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("history_fetch_where prepare: {e}"))?;
        decode_history_rows(&mut stmt, query_params, "history_fetch_where")
    })
}

/// Upsert a batch of history records. Existing rows (matched by `id`) are overwritten.
pub fn history_upsert_batch(db_path: &str, records: &[HistoryRecord]) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    with_conn(db_path, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("history_upsert_batch begin: {e}"))?;
        let result = history_upsert_on_conn(conn, records);
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

fn history_upsert_on_conn(
    conn: &rusqlite::Connection,
    records: &[HistoryRecord],
) -> Result<(), String> {
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
            params![
                rec.id,
                rec.wallet_id,
                rec.chain_name,
                rec.tx_hash,
                rec.created_at,
                payload_json
            ],
        )
        .map_err(|e| format!("history_upsert_batch row: {e}"))?;
    }
    Ok(())
}

/// Hold the SQLite write transaction across the read, domain merge and write.
/// A second refresh (including another connection) sees the first one's result.
pub(crate) fn history_update_chain<T>(
    db_path: &str,
    chain_name: &str,
    update: impl FnOnce(Vec<HistoryRecord>) -> Result<(Vec<HistoryRecord>, T), String>,
) -> Result<T, String> {
    with_conn(db_path, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
        let existing = {
            let mut stmt = tx.prepare("SELECT id, wallet_id, chain_name, tx_hash, created_at, payload FROM history_records WHERE chain_name = ?1 ORDER BY created_at DESC, id ASC").map_err(|e| e.to_string())?;
            decode_history_rows(&mut stmt, params![chain_name], "history_update_chain")?
        };
        let (rows, result) = update(existing)?;
        history_upsert_on_conn(&tx, &rows)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
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
        decode_history_rows(&mut stmt, [], "history_fetch_all")
    })
}

/// Run a prepared history-row query and decode every row. Shared by
/// `history_fetch_all` and `history_fetch_where` so the six-column decode and
/// the JSON payload parse exist once.
fn decode_history_rows(
    stmt: &mut rusqlite::Statement,
    query_params: impl rusqlite::Params,
    context: &str,
) -> Result<Vec<HistoryRecord>, String> {
    let rows = stmt
        .query_map(query_params, |row| {
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
        .map_err(|e| format!("{context} query: {e}"))?;
    let mut records = Vec::new();
    for row in rows {
        let (id, wallet_id, chain_name, tx_hash, created_at, payload_json) =
            row.map_err(|e| format!("{context} row: {e}"))?;
        let payload = serde_json::from_str(&payload_json)
            .map_err(|e| format!("{context} decode payload: {e}"))?;
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

/// Submission completion updates only its own fields and cannot undo a receipt
/// that arrived while the network request was in flight.
pub(crate) fn history_save_send_progress(
    db_path: &str,
    incoming: &CorePersistedTransactionRecord,
    reserve_nonce: bool,
) -> Result<(), String> {
    use rusqlite::OptionalExtension;
    with_conn(db_path, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
        let owner = incoming.wallet_id.as_deref().ok_or("send has no wallet")?;
        let present: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM wallets WHERE id = ?1)",
                params![owner],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !present {
            return Err("wallet removed during submission".into());
        }
        // The in-process sender lock orders normal sends. This reservation also
        // refuses a stale nonce selected by another process before it broadcasts.
        // Explicit replacement requests intentionally bypass this check.
        if reserve_nonce {
            let nonce = incoming
                .ethereum_nonce
                .ok_or("missing EVM nonce reservation")?;
            let source = incoming
                .source_address
                .as_deref()
                .ok_or("missing EVM sender")?;
            let conflict: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_records WHERE chain_name = ?1 AND id != lower(?2)
                 AND lower(json_extract(payload, '$.sourceAddress')) = lower(?3)
                 AND json_extract(payload, '$.ethereumNonce') = ?4
                 AND json_extract(payload, '$.kind') = 'send'
                 AND json_extract(payload, '$.status') = 'pending')",
                params![incoming.chain_name, incoming.id, source, nonce], |row| row.get(0)
            ).map_err(|e| e.to_string())?;
            if conflict {
                return Err(
                    "EVM nonce was reserved by another send; retry with a fresh nonce".into(),
                );
            }
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT payload FROM history_records WHERE id = lower(?1)",
                params![incoming.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let payload = if let Some(json) = previous {
            let mut stored: CorePersistedTransactionRecord =
                serde_json::from_str(&json).map_err(|e| e.to_string())?;
            if stored.wallet_id != incoming.wallet_id || stored.chain_name != incoming.chain_name {
                return Err("send record identity changed".into());
            }
            stored.signed_transaction_payload = incoming.signed_transaction_payload.clone();
            stored.signed_transaction_payload_format =
                incoming.signed_transaction_payload_format.clone();
            if incoming.ethereum_nonce.is_some() {
                stored.ethereum_nonce = incoming.ethereum_nonce;
            }
            if incoming.transaction_hash.is_some() {
                stored.transaction_hash = incoming.transaction_hash.clone();
            }
            if stored.status != Some(crate::store::wallet_domain::CoreTransactionStatus::Confirmed)
            {
                stored.status = incoming.status;
                stored.failure_reason = incoming.failure_reason.clone();
            }
            stored
        } else {
            incoming.clone()
        };
        let record = history_record_from_payload(payload);
        let json = serde_json::to_string(&record.payload).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO history_records(id,wallet_id,chain_name,tx_hash,created_at,payload) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET tx_hash=excluded.tx_hash,payload=excluded.payload", params![record.id, record.wallet_id, record.chain_name, record.tx_hash, record.created_at, json]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    })
}
