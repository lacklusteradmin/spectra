use super::*;
use crate::store::persistence_models::TransactionRecord;
use crate::wallet_db::error::DbError;

// ── History record CRUD ───────────────────────────────────────────────────────

/// Where an undated pending transaction sorts: 9999-12-31T23:59:59Z.
const UNDATED_PENDING_SORT_KEY: f64 = 253_402_300_799.0;

/// The instant a record sorts by, which is its own time when it has one.
///
/// A pending transaction the chain has not dated yet is the newest thing in
/// the history, so it sorts after every dated one: first when newest-first,
/// last when oldest-first. Its payload keeps the unknown time, so it still
/// reads as undated; once it confirms, the dated record replaces the key.
fn history_sort_key(payload: &TransactionRecord) -> f64 {
    let undated = payload.created_at_unix <= 0.0;
    if undated && payload.status == crate::store::wallet_domain::TransactionStatus::Pending {
        UNDATED_PENDING_SORT_KEY
    } else {
        payload.created_at_unix
    }
}

/// Project distinct indexed paths, not transaction payloads. Repeated transactions
/// on one address produce one path to parse, scoped to this wallet and chain.
pub(crate) fn history_keypool_indices(
    database: &WalletDatabase,
    wallet_id: &str,
    chain_id: crate::registry::Chain,
) -> Result<(Option<i32>, Option<i32>), DbError> {
    with_conn(database, |conn| {
        let mut maxima = [None, None];
        for (branch, field) in ["sourceDerivationPath", "changeDerivationPath"]
            .into_iter()
            .enumerate()
        {
            let sql = format!(
                "SELECT DISTINCT json_extract(payload, '$.{field}')
                FROM history_records WHERE wallet_id = ?1 AND chain_id = ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(DbError::from)?;
            let rows = stmt
                .query_map(params![wallet_id.to_lowercase(), chain_id], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .map_err(DbError::from)?;
            for path in rows {
                if let Some(index) = path.map_err(DbError::from)?.as_deref().and_then(|path| {
                    crate::derivation::path::utxo_discovery_index(path, chain_id, branch as u32)
                }) {
                    let index = i32::try_from(index)
                        .map_err(|_| DbError::Corrupt("keypool index out of range".into()))?;
                    maxima[branch] = Some(maxima[branch].map_or(index, |old: i32| old.max(index)));
                }
            }
        }
        Ok((maxima[0], maxima[1]))
    })
}

/// Load every transaction for one wallet, newest first.
pub fn history_fetch_for_wallet(
    database: &WalletDatabase,
    wallet_id: &str,
) -> Result<Vec<TransactionRecord>, DbError> {
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
) -> Result<Vec<TransactionRecord>, DbError> {
    with_conn(database, |conn| {
        let sql = format!(
            "SELECT payload FROM history_records WHERE {predicate}
             ORDER BY created_at DESC, id ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(DbError::from)?;
        decode_history_rows(&mut stmt, query_params, "history_fetch_where")
    })
}

/// Upsert a batch of records. Existing rows (matched by `id`) are overwritten.
pub fn history_upsert_batch(
    database: &WalletDatabase,
    records: &[TransactionRecord],
) -> Result<(), DbError> {
    if records.is_empty() {
        return Ok(());
    }
    with_conn(database, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(DbError::from)?;
        history_upsert_on_conn(&tx, records)?;
        tx.commit().map_err(DbError::from)
    })
}

/// Each record under the columns it is indexed by, all derived from it:
/// its lowercased id, wallet and hash, its network and its sort key.
fn history_upsert_on_conn(
    conn: &rusqlite::Connection,
    records: &[TransactionRecord],
) -> Result<(), DbError> {
    let mut statement = conn
        .prepare_cached(
            "INSERT INTO history_records (id, wallet_id, chain_id, tx_hash, created_at, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                         wallet_id  = excluded.wallet_id,
                         chain_id = excluded.chain_id,
                         tx_hash    = excluded.tx_hash,
                         created_at = excluded.created_at,
                         payload    = excluded.payload",
        )
        .map_err(DbError::from)?;
    for record in records {
        let payload_json = serde_json::to_string(record)
            .map_err(|e| DbError::Corrupt(format!("history_upsert_batch encode payload: {e}")))?;
        statement
            .execute(params![
                record.id.to_lowercase(),
                record.wallet_id.as_deref().map(str::to_lowercase),
                record.chain_id,
                record.transaction_hash.as_deref().map(str::to_lowercase),
                history_sort_key(record),
                payload_json
            ])
            .map_err(DbError::from)?;
    }
    Ok(())
}

/// Hold the SQLite write transaction across the read, domain merge and write.
/// A second refresh (including another connection) sees the first one's result.
pub(crate) fn history_update_chain<T, E: From<DbError>>(
    database: &WalletDatabase,
    chain_id: crate::registry::Chain,
    update: impl FnOnce(Vec<TransactionRecord>) -> Result<(Vec<TransactionRecord>, T), E>,
) -> Result<T, E> {
    history_update_chain_checked(database, chain_id, |_, rows| update(rows))
}

pub(crate) fn history_update_chain_checked<T, E: From<DbError>>(
    database: &WalletDatabase,
    chain_id: crate::registry::Chain,
    update: impl FnOnce(
        &rusqlite::Connection,
        Vec<TransactionRecord>,
    ) -> Result<(Vec<TransactionRecord>, T), E>,
) -> Result<T, E> {
    with_conn(database, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(DbError::from)?;
        let existing = {
            let mut stmt = tx.prepare("SELECT payload FROM history_records WHERE chain_id = ?1 ORDER BY created_at DESC, id ASC").map_err(DbError::from)?;
            decode_history_rows(&mut stmt, params![chain_id], "history_update_chain")?
        };
        let (rows, result) = update(&tx, existing)?;
        history_upsert_on_conn(&tx, &rows)?;
        tx.commit().map_err(DbError::from)?;
        Ok::<_, E>(result)
    })
}

/// Fetch all history records ordered by created_at DESC.
pub fn history_fetch_all(database: &WalletDatabase) -> Result<Vec<TransactionRecord>, DbError> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare("SELECT payload FROM history_records ORDER BY created_at DESC, id ASC")
            .map_err(DbError::from)?;
        decode_history_rows(&mut stmt, [], "history_fetch_all")
    })
}

/// Run a prepared query selecting `payload` and decode every row.
fn decode_history_rows(
    stmt: &mut rusqlite::Statement,
    query_params: impl rusqlite::Params,
    context: &str,
) -> Result<Vec<TransactionRecord>, DbError> {
    stmt.query_map(query_params, |row| row.get::<_, String>(0))
        .map_err(DbError::from)?
        .map(|json| {
            serde_json::from_str(&json.map_err(DbError::from)?)
                .map_err(|e| DbError::Corrupt(format!("{context} decode payload: {e}")))
        })
        .collect()
}

/// Delete history records by ID list: a test's stand-in for a record
/// removed while another operation holds it.
#[cfg(test)]
pub(crate) fn history_delete(database: &WalletDatabase, ids: &[String]) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }
    with_conn(database, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(DbError::from)?;
        {
            let mut statement = tx
                .prepare_cached("DELETE FROM history_records WHERE id = ?1")
                .map_err(DbError::from)?;
            for id in ids {
                statement.execute(params![id]).map_err(DbError::from)?;
            }
        }
        tx.commit().map_err(DbError::from)
    })
}

/// Delete every record. The rows are never decoded: a reset clears records
/// this build cannot read, rather than failing on them after its wallets
/// are gone.
pub fn history_clear(database: &WalletDatabase) -> Result<(), DbError> {
    with_conn(database, |conn| {
        conn.execute("DELETE FROM history_records", [])
            .map_err(DbError::from)?;
        Ok(())
    })
}

/// Submission completion updates only its own fields and cannot undo a receipt
/// that arrived while the network request was in flight.
pub(crate) fn history_save_send_progress(
    database: &WalletDatabase,
    incoming: &TransactionRecord,
    reserve_nonce: bool,
) -> Result<(), DbError> {
    use rusqlite::OptionalExtension;
    with_conn(database, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(DbError::from)?;
        let owner = incoming
            .wallet_id
            .as_deref()
            .ok_or_else(|| DbError::Invalid("send has no wallet".into()))?;
        let present: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM wallets WHERE id = ?1)",
                params![owner],
                |row| row.get(0),
            )
            .map_err(DbError::from)?;
        if !present {
            return Err(DbError::Invalid("wallet removed during submission".into()));
        }
        // The in-process sender lock orders normal sends. This reservation also
        // refuses a stale nonce selected by another process before it broadcasts.
        // Explicit replacement requests intentionally bypass this check.
        if reserve_nonce {
            let nonce = incoming
                .nonce
                .ok_or_else(|| DbError::Invalid("missing EVM nonce reservation".into()))?;
            let source = incoming
                .source_address
                .as_deref()
                .ok_or_else(|| DbError::Invalid("missing EVM sender".into()))?;
            let conflict: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_records WHERE chain_id = ?1 AND id != lower(?2)
                 AND lower(json_extract(payload, '$.sourceAddress')) = lower(?3)
                 AND json_extract(payload, '$.nonce') = ?4
                 AND json_extract(payload, '$.kind') IN ('send', 'stake', 'unstake', 'withdraw', 'claimRewards', 'revokeApproval', 'deleteAccessKey', 'mergeCoins', 'closeTokenAccounts', 'refundTokenStorage', 'trustAsset', 'removeTrustLine', 'shield')
                 AND json_extract(payload, '$.status') = 'pending')",
                params![incoming.chain_id, incoming.id, source, nonce], |row| row.get(0)
            ).map_err(DbError::from)?;
            if conflict {
                return Err(DbError::Invalid(
                    "EVM nonce was reserved by another send; retry with a fresh nonce".into(),
                ));
            }
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT payload FROM history_records WHERE id = lower(?1)",
                params![incoming.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(DbError::from)?;
        let payload = if let Some(json) = previous {
            let mut stored: TransactionRecord =
                serde_json::from_str(&json).map_err(DbError::from)?;
            if stored.wallet_id != incoming.wallet_id || stored.chain_id != incoming.chain_id {
                return Err(DbError::Invalid("send record identity changed".into()));
            }
            if incoming.nonce.is_some() {
                stored.nonce = incoming.nonce;
            }
            if incoming.transaction_hash.is_some() {
                stored.transaction_hash = incoming.transaction_hash.clone();
            }
            if stored.status != crate::store::wallet_domain::TransactionStatus::Confirmed {
                stored.status = incoming.status;
                stored.failure_reason = incoming.failure_reason.clone();
            }
            stored
        } else {
            incoming.clone()
        };
        history_upsert_on_conn(&tx, std::slice::from_ref(&payload))?;
        tx.commit().map_err(DbError::from)
    })
}

/// Indexed pending sends for nonce reservation; validate nonces in the owning service.
pub(crate) fn history_pending_for_sender(
    database: &WalletDatabase,
    chain: crate::registry::Chain,
    sender: &str,
) -> Result<Vec<TransactionRecord>, DbError> {
    history_fetch_where(
        database,
        "chain_id = ?1 AND lower(json_extract(payload, '$.sourceAddress')) = lower(?2) AND json_extract(payload, '$.kind') IN ('send', 'stake', 'unstake', 'withdraw', 'claimRewards', 'revokeApproval', 'deleteAccessKey', 'mergeCoins', 'closeTokenAccounts', 'refundTokenStorage', 'trustAsset', 'removeTrustLine', 'shield') AND json_extract(payload, '$.status') = 'pending'",
        params![chain, sender],
    )
}
