use super::*;
use crate::service::{HistoryPage, HistoryQuery, HistoryQueryFilter, TransactionSnapshot};
use crate::store::persistence_models::CorePersistedTransactionRecord;

fn page_on_conn(conn: &rusqlite::Connection, query: &HistoryQuery) -> Result<HistoryPage, String> {
    // Refuse records whose identity/state cannot even be classified. Otherwise
    // the WHERE/JOIN clauses could silently hide corruption as an empty result.
    let unclassified: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM history_records WHERE json_type(payload, '$.id') IS NOT 'text'
         OR coalesce(json_extract(payload, '$.kind'), '') NOT IN ('send', 'receive')
         OR coalesce(json_extract(payload, '$.status'), '') NOT IN ('pending', 'confirmed', 'failed'))",
        [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if unclassified {
        return Err("history contains an unreadable identity or status".into());
    }
    // Identity, deduplication and status precedence are domain rules. Only matching
    // page payloads are decoded; sorting/searching do not materialize the history in Rust.
    let order = if query.oldest_first { "ASC" } else { "DESC" };
    let sql = format!("WITH ranked AS (
        SELECT h.*, ROW_NUMBER() OVER (
            PARTITION BY h.wallet_id, h.chain_name,
                coalesce(json_extract(h.payload, '$.deploymentId'), 'record:' || h.id),
                coalesce(nullif(json_extract(h.payload, '$.transactionHash'), ''), h.id)
            ORDER BY CASE json_extract(h.payload, '$.status') WHEN 'confirmed' THEN 3 WHEN 'pending' THEN 2 ELSE 1 END DESC,
                h.created_at DESC, h.id ASC) AS rank
        FROM history_records h JOIN wallets w ON lower(w.id) = h.wallet_id
        WHERE (?1 IS NULL OR h.wallet_id = lower(?1))
    ) SELECT payload FROM ranked WHERE rank = 1
        AND (?2 = 'all' OR json_extract(payload, '$.kind') = ?2 OR json_extract(payload, '$.status') = ?2)
        AND (?3 = '' OR instr(spectra_lower(coalesce(json_extract(payload, '$.walletName'), '') || ' ' ||
          coalesce(json_extract(payload, '$.assetDisplayName'), '') || ' ' ||
          coalesce(json_extract(payload, '$.symbol'), '') || ' ' || chain_name || ' ' ||
          coalesce(json_extract(payload, '$.address'), '') || ' ' || coalesce(tx_hash, '') || ' ' ||
          coalesce(json_extract(payload, '$.transactionHistorySource'), '')), ?3) > 0)
        ORDER BY created_at {order}, id ASC LIMIT ?4 OFFSET ?5");
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
            if !records.iter().any(|r| r.id == record.id) {
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
