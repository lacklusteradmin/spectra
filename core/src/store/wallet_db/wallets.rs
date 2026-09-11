use super::*;

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
