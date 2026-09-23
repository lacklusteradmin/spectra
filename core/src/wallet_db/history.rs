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

/// Index the same Unix timestamp carried by the stored payload.
pub fn history_record_from_payload(
    payload: crate::store::persistence_models::CorePersistedTransactionRecord,
) -> HistoryRecord {
    HistoryRecord {
        id: payload.id.to_lowercase(),
        wallet_id: payload.wallet_id.as_deref().map(str::to_lowercase),
        chain_name: payload.chain_name.clone(),
        tx_hash: payload.transaction_hash.as_deref().map(str::to_lowercase),
        created_at: payload.created_at_unix,
        payload,
    }
}

/// Project distinct indexed paths, not transaction payloads. Repeated transactions
/// on one address produce one path to parse, scoped to this wallet and chain.
pub(crate) fn history_keypool_indices(
    database: &WalletDatabase,
    wallet_id: &str,
    chain_name: &str,
) -> Result<(Option<i32>, Option<i32>), String> {
    with_conn(database, |conn| {
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
pub fn history_existing_ids(
    database: &WalletDatabase,
    ids: &[String],
) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    with_conn(database, |conn| {
        use rusqlite::OptionalExtension;
        let mut stmt = conn
            .prepare_cached("SELECT id FROM history_records WHERE id = ?1")
            .map_err(|e| format!("history_existing_ids prepare: {e}"))?;
        let wanted: std::collections::BTreeSet<String> =
            ids.iter().map(|id| id.to_lowercase()).collect();
        let mut found = Vec::new();
        for id in wanted {
            if let Some(id) = stmt
                .query_row(params![id], |row| row.get::<_, String>(0))
                .optional()
                .map_err(|e| format!("history_existing_ids query: {e}"))?
            {
                found.push(id);
            }
        }
        Ok(found)
    })
}

/// Load every transaction for one wallet, newest first.
pub fn history_fetch_for_wallet(
    database: &WalletDatabase,
    wallet_id: &str,
) -> Result<Vec<HistoryRecord>, String> {
    history_fetch_where(
        database,
        "wallet_id = ?1",
        params![wallet_id.to_lowercase()],
    )
}

/// Shared body for `history_fetch_all` and the scoped fetches: same query,
/// same row decode, a different `WHERE` — pushed to SQL and `idx_hr_wallet` /
/// `idx_hr_chain` rather than fetched whole and filtered in Rust, which is
/// what every caller here did until each was found reading the whole table
/// for one wallet's or one chain's worth of rows.
fn history_fetch_where(
    database: &WalletDatabase,
    predicate: &str,
    query_params: impl rusqlite::Params,
) -> Result<Vec<HistoryRecord>, String> {
    with_conn(database, |conn| {
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
pub fn history_upsert_batch(
    database: &WalletDatabase,
    records: &[HistoryRecord],
) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    with_conn(database, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| format!("history_upsert_batch begin: {e}"))?;
        history_upsert_on_conn(&tx, records)?;
        tx.commit()
            .map_err(|e| format!("history_upsert_batch commit: {e}"))
    })
}

fn history_upsert_on_conn(
    conn: &rusqlite::Connection,
    records: &[HistoryRecord],
) -> Result<(), String> {
    let mut statement = conn
        .prepare_cached(
            "INSERT INTO history_records (id, wallet_id, chain_name, tx_hash, created_at, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                         wallet_id  = excluded.wallet_id,
                         chain_name = excluded.chain_name,
                         tx_hash    = excluded.tx_hash,
                         created_at = excluded.created_at,
                         payload    = excluded.payload",
        )
        .map_err(|e| format!("history_upsert_batch prepare: {e}"))?;
    for rec in records {
        let payload_json = serde_json::to_string(&rec.payload)
            .map_err(|e| format!("history_upsert_batch encode payload: {e}"))?;
        statement
            .execute(params![
                rec.id,
                rec.wallet_id,
                rec.chain_name,
                rec.tx_hash,
                rec.created_at,
                payload_json
            ])
            .map_err(|e| format!("history_upsert_batch row: {e}"))?;
    }
    Ok(())
}

/// Hold the SQLite write transaction across the read, domain merge and write.
/// A second refresh (including another connection) sees the first one's result.
pub(crate) fn history_update_chain<T>(
    database: &WalletDatabase,
    chain_name: &str,
    update: impl FnOnce(Vec<HistoryRecord>) -> Result<(Vec<HistoryRecord>, T), String>,
) -> Result<T, String> {
    history_update_chain_checked(database, chain_name, |_, rows| update(rows))
}

pub(crate) fn history_update_chain_checked<T>(
    database: &WalletDatabase,
    chain_name: &str,
    update: impl FnOnce(
        &rusqlite::Connection,
        Vec<HistoryRecord>,
    ) -> Result<(Vec<HistoryRecord>, T), String>,
) -> Result<T, String> {
    with_conn(database, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
        let existing = {
            let mut stmt = tx.prepare("SELECT id, wallet_id, chain_name, tx_hash, created_at, payload FROM history_records WHERE chain_name = ?1 ORDER BY created_at DESC, id ASC").map_err(|e| e.to_string())?;
            decode_history_rows(&mut stmt, params![chain_name], "history_update_chain")?
        };
        let (rows, result) = update(&tx, existing)?;
        history_upsert_on_conn(&tx, &rows)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    })
}

/// Fetch all history records ordered by created_at DESC.
pub fn history_fetch_all(database: &WalletDatabase) -> Result<Vec<HistoryRecord>, String> {
    with_conn(database, |conn| {
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
pub fn history_delete(database: &WalletDatabase, ids: &[String]) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    with_conn(database, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| format!("history_delete begin: {e}"))?;
        {
            let mut statement = tx
                .prepare_cached("DELETE FROM history_records WHERE id = ?1")
                .map_err(|e| format!("history_delete prepare: {e}"))?;
            for id in ids {
                statement
                    .execute(params![id])
                    .map_err(|e| format!("history_delete row: {e}"))?;
            }
        }
        tx.commit()
            .map_err(|e| format!("history_delete commit: {e}"))
    })
}

/// Atomically delete all records then insert the provided batch (full replacement).
pub fn history_replace_all(
    database: &WalletDatabase,
    records: &[HistoryRecord],
) -> Result<(), String> {
    with_conn(database, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| format!("history_replace_all begin: {e}"))?;
        tx.execute("DELETE FROM history_records", [])
            .map_err(|e| format!("history_replace_all delete: {e}"))?;
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO history_records (id, wallet_id, chain_name, tx_hash, created_at, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
            ).map_err(|e| format!("history_replace_all prepare: {e}"))?;
            for rec in records {
                let payload_json = serde_json::to_string(&rec.payload)
                    .map_err(|e| format!("history_replace_all encode payload: {e}"))?;
                statement
                    .execute(params![
                        rec.id,
                        rec.wallet_id,
                        rec.chain_name,
                        rec.tx_hash,
                        rec.created_at,
                        payload_json
                    ])
                    .map_err(|e| format!("history_replace_all insert: {e}"))?;
            }
        }
        tx.commit()
            .map_err(|e| format!("history_replace_all commit: {e}"))
    })
}

/// Delete all history records for a given wallet_id.
pub fn history_delete_for_wallet(database: &WalletDatabase, wallet_id: &str) -> Result<(), String> {
    with_conn(database, |conn| {
        conn.execute(
            "DELETE FROM history_records WHERE wallet_id = ?1",
            params![wallet_id.to_lowercase()],
        )
        .map_err(|e| format!("history_delete_for_wallet: {e}"))?;
        Ok(())
    })
}

/// Delete all history records (hard reset).
pub fn history_clear(database: &WalletDatabase) -> Result<(), String> {
    with_conn(database, |conn| {
        conn.execute("DELETE FROM history_records", [])
            .map_err(|e| format!("history_clear: {e}"))?;
        Ok(())
    })
}

/// Submission completion updates only its own fields and cannot undo a receipt
/// that arrived while the network request was in flight.
pub(crate) fn history_save_send_progress(
    database: &WalletDatabase,
    incoming: &CorePersistedTransactionRecord,
    reserve_nonce: bool,
) -> Result<(), String> {
    use rusqlite::OptionalExtension;
    with_conn(database, |conn| {
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
            let nonce = incoming.nonce.ok_or("missing EVM nonce reservation")?;
            let source = incoming
                .source_address
                .as_deref()
                .ok_or("missing EVM sender")?;
            let conflict: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_records WHERE chain_name = ?1 AND id != lower(?2)
                 AND lower(json_extract(payload, '$.sourceAddress')) = lower(?3)
                 AND json_extract(payload, '$.nonce') = ?4
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
            if incoming.nonce.is_some() {
                stored.nonce = incoming.nonce;
            }
            if incoming.transaction_hash.is_some() {
                stored.transaction_hash = incoming.transaction_hash.clone();
            }
            if stored.status != crate::store::wallet_domain::CoreTransactionStatus::Confirmed {
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

/// Indexed pending sends for nonce reservation; validate nonces in the owning service.
pub(crate) fn history_pending_for_sender(
    database: &WalletDatabase,
    chain: &str,
    sender: &str,
) -> Result<Vec<HistoryRecord>, String> {
    history_fetch_where(database,
        "chain_name = ?1 AND lower(json_extract(payload, '$.sourceAddress')) = lower(?2) AND json_extract(payload, '$.kind') = 'send' AND json_extract(payload, '$.status') = 'pending'",
        params![chain, sender])
}
