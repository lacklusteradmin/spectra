use super::*;
use crate::service::{HistoryPage, HistoryQuery, HistoryQueryFilter, TransactionSnapshot};
use crate::store::persistence_models::CorePersistedTransactionRecord;

// The stored identity/rank index selects a winner with a single seek. Unlike
// ROW_NUMBER over every candidate, this permits the outer date index to stop
// once it has found the requested page. Classification is constrained on writes.
fn page_sql(query: &HistoryQuery) -> String {
    let order = if query.oldest_first { "ASC" } else { "DESC" };
    let wallet = if query.wallet_id.is_some() {
        "h.wallet_id = lower(?1) AND"
    } else {
        ""
    };
    format!("SELECT h.payload FROM history_records h
        WHERE {wallet} EXISTS (SELECT 1 FROM wallets w WHERE lower(w.id) = h.wallet_id)
        AND h.id = (SELECT candidate.id FROM history_records candidate
            WHERE candidate.wallet_id = h.wallet_id AND candidate.chain_name = h.chain_name
                AND candidate.asset_key = h.asset_key AND candidate.hash_key = h.hash_key
            ORDER BY candidate.status_rank DESC, candidate.created_at DESC, candidate.id ASC LIMIT 1)
        AND (?2 = 'all' OR json_extract(h.payload, '$.kind') = ?2 OR json_extract(h.payload, '$.status') = ?2)
        AND (?3 = '' OR instr(spectra_lower(coalesce(json_extract(h.payload, '$.walletName'), '') || ' ' ||
          coalesce(json_extract(h.payload, '$.assetDisplayName'), '') || ' ' ||
          coalesce(json_extract(h.payload, '$.symbol'), '') || ' ' || h.chain_name || ' ' ||
          coalesce(json_extract(h.payload, '$.address'), '') || ' ' || coalesce(h.tx_hash, '') || ' ' ||
          coalesce(json_extract(h.payload, '$.transactionHistorySource'), '')), ?3) > 0)
        ORDER BY h.created_at {order}, h.id ASC LIMIT ?4 OFFSET ?5")
}

fn page_on_conn(conn: &rusqlite::Connection, query: &HistoryQuery) -> Result<HistoryPage, String> {
    let sql = page_sql(query);
    let filter = match query.filter {
        HistoryQueryFilter::All => "all",
        HistoryQueryFilter::Send => "send",
        HistoryQueryFilter::Receive => "receive",
        HistoryQueryFilter::Pending => "pending",
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![
                query.wallet_id,
                filter,
                query.search.trim().to_lowercase(),
                i64::from(query.limit) + 1,
                query.offset as i64
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    for row in rows {
        records.push(
            serde_json::from_str(&row.map_err(|e| e.to_string())?)
                .map_err(|e| format!("history decode: {e}"))?,
        );
    }
    let has_more = records.len() > query.limit as usize;
    records.truncate(query.limit as usize);
    Ok(HistoryPage {
        next_offset: query.offset + records.len() as u64,
        records,
        has_more,
    })
}

pub(crate) fn history_page(
    database: &WalletDatabase,
    query: &HistoryQuery,
) -> Result<HistoryPage, String> {
    with_conn(database, |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        page_on_conn(&tx, query)
    })
}

pub(crate) fn history_find(
    database: &WalletDatabase,
    id: &str,
) -> Result<Option<CorePersistedTransactionRecord>, String> {
    use rusqlite::OptionalExtension;
    with_conn(database, |conn| {
        let json: Option<String> = conn
            .query_row(
                "SELECT payload FROM history_records WHERE id = lower(?1)",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        json.map(|json| serde_json::from_str(&json).map_err(|e| format!("history decode: {e}")))
            .transpose()
    })
}

pub(crate) fn history_snapshot(
    database: &WalletDatabase,
    sequence: &std::sync::atomic::AtomicU64,
) -> Result<TransactionSnapshot, String> {
    with_conn(database, |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let total_count = tx
            .query_row("SELECT count(*) FROM history_records", [], |r| {
                r.get::<_, u64>(0)
            })
            .map_err(|e| e.to_string())?;
        let mut records = page_on_conn(
            &tx,
            &HistoryQuery {
                limit: 50,
                ..Default::default()
            },
        )?
        .records;
        let mut pending = tx.prepare("SELECT payload FROM history_records WHERE json_extract(payload, '$.status') = 'pending' ORDER BY created_at DESC, id ASC").map_err(|e| e.to_string())?;
        let mut seen: std::collections::HashSet<String> =
            records.iter().map(|r| r.id.clone()).collect();
        let mut replaceable = Vec::new();
        let rows = pending
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let record: CorePersistedTransactionRecord =
                serde_json::from_str(&row.map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if let Some(send) = crate::service::history_derived::replaceable_send(&record) {
                replaceable.push(send);
            }
            if seen.insert(record.id.clone()) {
                records.push(record);
            }
        }
        let mut first = tx.prepare("SELECT wallet_id, min(created_at) FROM history_records WHERE wallet_id IS NOT NULL GROUP BY wallet_id").map_err(|e| e.to_string())?;
        let earliest = first
            .query_map([], |r| {
                Ok(crate::store::WalletEarliestTransactionDate {
                    wallet_id: r.get(0)?,
                    earliest_created_at_unix: r.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let revision = sequence.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        Ok(TransactionSnapshot {
            revision,
            recent_and_pending: records,
            replaceable,
            earliest,
            total_count,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_pages_seek_identity_winners_without_temporary_sorts() {
        let database = WalletDatabase::new(":memory:");
        with_conn(&database, |conn| {
            for oldest_first in [false, true] {
                for wallet_id in [None, Some("wallet".into())] {
                    let query = HistoryQuery {
                        oldest_first,
                        wallet_id,
                        ..Default::default()
                    };
                    let plan = conn
                        .prepare(&format!("EXPLAIN QUERY PLAN {}", page_sql(&query)))
                        .unwrap()
                        .query_map(params![query.wallet_id, "all", "", 21, 0], |row| {
                            row.get::<_, String>(3)
                        })
                        .unwrap()
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    assert!(
                        plan.iter().any(|line| line.contains("idx_hr_identity")),
                        "{plan:?}"
                    );
                    assert!(
                        !plan.iter().any(|line| line.contains("TEMP B-TREE")),
                        "{plan:?}"
                    );
                }
            }
            Ok(())
        })
        .unwrap();
    }
}
