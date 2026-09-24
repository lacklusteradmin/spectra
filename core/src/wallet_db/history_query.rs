use super::*;
use crate::service::{HistoryPage, HistoryQuery, HistoryQueryFilter, TransactionSnapshot};
use crate::store::persistence_models::CorePersistedTransactionRecord;

// All user-facing projections select the same canonical transaction and owner.
const VISIBLE: &str = "EXISTS (SELECT 1 FROM wallets w WHERE lower(w.id) = h.wallet_id)
    AND h.id = (SELECT candidate.id FROM history_records candidate
        WHERE candidate.wallet_id = h.wallet_id AND candidate.chain_id = h.chain_id
          AND candidate.asset_key = h.asset_key AND candidate.hash_key = h.hash_key
        ORDER BY candidate.status_rank DESC, candidate.created_at DESC, candidate.id ASC LIMIT 1)";

#[derive(serde::Serialize, serde::Deserialize)]
struct Cursor {
    created_at: f64,
    id: String,
    wallet_id: Option<String>,
    filter: HistoryQueryFilter,
    search: String,
    oldest_first: bool,
}

fn decode_cursor(query: &HistoryQuery) -> Result<Option<Cursor>, String> {
    query
        .cursor
        .as_ref()
        .map(|encoded| {
            let cursor: Cursor =
                serde_json::from_str(encoded).map_err(|_| "Invalid history cursor")?;
            if !cursor.created_at.is_finite()
                || cursor.id.is_empty()
                || cursor.wallet_id != query.wallet_id
                || cursor.filter != query.filter
                || cursor.search != query.search
                || cursor.oldest_first != query.oldest_first
            {
                return Err("History cursor does not match this query; restart pagination".into());
            }
            Ok(cursor)
        })
        .transpose()
}

fn page_sql(query: &HistoryQuery) -> String {
    let order = if query.oldest_first { "ASC" } else { "DESC" };
    let wallet = if query.wallet_id.is_some() {
        "h.wallet_id = lower(?1) AND"
    } else {
        ""
    };
    let seek = if query.cursor.is_some() {
        if query.oldest_first {
            "AND h.created_at >= ?5 AND (h.created_at > ?5 OR h.id > ?6)"
        } else {
            "AND h.created_at <= ?5 AND (h.created_at < ?5 OR h.id > ?6)"
        }
    } else {
        // Keep parameter numbering identical for both query shapes.
        "AND ?5 IS NULL AND ?6 IS NULL"
    };
    format!("SELECT h.payload, h.created_at, h.id FROM history_records h
        WHERE {wallet} {VISIBLE} {seek}
        AND (?2 = 'all' OR json_extract(h.payload, '$.kind') = ?2 OR json_extract(h.payload, '$.status') = ?2)
        AND (?3 = '' OR instr(spectra_lower(coalesce(json_extract(h.payload, '$.walletName'), '') || ' ' ||
          coalesce(json_extract(h.payload, '$.assetDisplayName'), '') || ' ' ||
          coalesce(json_extract(h.payload, '$.symbol'), '') || ' ' || h.chain_id || ' ' ||
          coalesce(json_extract(h.payload, '$.address'), '') || ' ' || coalesce(h.tx_hash, '') || ' ' ||
          coalesce(json_extract(h.payload, '$.transactionHistorySource'), '')), ?3) > 0)
        ORDER BY h.created_at {order}, h.id ASC LIMIT ?4")
}

fn page_on_conn(conn: &rusqlite::Connection, query: &HistoryQuery) -> Result<HistoryPage, String> {
    let cursor = decode_cursor(query)?;
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
                cursor.as_ref().map(|c| c.created_at),
                cursor.as_ref().map(|c| &c.id)
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (json, created_at, id) = row.map_err(|e| e.to_string())?;
        let record = serde_json::from_str(&json).map_err(|e| format!("history decode: {e}"))?;
        entries.push((record, created_at, id));
    }
    let has_more = entries.len() > query.limit as usize;
    entries.truncate(query.limit as usize);
    let next_cursor = if has_more {
        entries
            .last()
            .map(|(_, created_at, id)| {
                serde_json::to_string(&Cursor {
                    created_at: *created_at,
                    id: id.clone(),
                    wallet_id: query.wallet_id.clone(),
                    filter: query.filter,
                    search: query.search.clone(),
                    oldest_first: query.oldest_first,
                })
                .map_err(|e| e.to_string())
            })
            .transpose()?
    } else {
        None
    };
    Ok(HistoryPage {
        next_cursor,
        records: entries.into_iter().map(|(record, _, _)| record).collect(),
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
            .query_row(
                &format!("SELECT count(*) FROM history_records h WHERE {VISIBLE}"),
                [],
                |r| r.get::<_, u64>(0),
            )
            .map_err(|e| e.to_string())?;
        let mut records = page_on_conn(
            &tx,
            &HistoryQuery {
                limit: 50,
                ..Default::default()
            },
        )?
        .records;
        let mut pending = tx.prepare(&format!("SELECT h.payload FROM history_records h WHERE {VISIBLE} AND json_extract(h.payload, '$.status') = 'pending' ORDER BY h.created_at DESC, h.id ASC")).map_err(|e| e.to_string())?;
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
        let mut first = tx.prepare(&format!("SELECT h.wallet_id, min(h.created_at) FROM history_records h WHERE {VISIBLE} GROUP BY h.wallet_id")).map_err(|e| e.to_string())?;
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
                    for continued in [false, true] {
                        let query = HistoryQuery {
                            oldest_first,
                            wallet_id: wallet_id.clone(),
                            cursor: continued.then(|| "cursor".into()),
                            ..Default::default()
                        };
                        let plan = conn
                            .prepare(&format!("EXPLAIN QUERY PLAN {}", page_sql(&query)))
                            .unwrap()
                            .query_map(
                                params![
                                    query.wallet_id,
                                    "all",
                                    "",
                                    21,
                                    continued.then_some(10.0),
                                    continued.then_some("anchor")
                                ],
                                |row| row.get::<_, String>(3),
                            )
                            .unwrap()
                            .collect::<Result<Vec<_>, _>>()
                            .unwrap();
                        assert!(
                            plan.iter().any(|line| line.contains("idx_hr_identity")),
                            "{plan:?}"
                        );
                        if continued {
                            assert!(
                                plan.iter().any(|line| line.contains("SEARCH h USING INDEX")
                                    && line.contains("created_at")),
                                "{plan:?}"
                            );
                        }
                        assert!(
                            !plan.iter().any(|line| line.contains("TEMP B-TREE")),
                            "{plan:?}"
                        );
                    }
                }
            }
            Ok(())
        })
        .unwrap();
    }
}
