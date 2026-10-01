use super::*;
use crate::wallet_db::error::DbError;

/// Insert or replace one wallet, keeping its existing position when it is
/// already stored and appending it to the end when it is new.
///
/// Use this for a single-wallet edit. To write a whole state snapshot use
/// [`app_state_save`], which also prunes wallets that are no longer present.
pub fn wallet_upsert(database: &WalletDatabase, wallet: &WalletState) -> Result<(), DbError> {
    let payload = serde_json::to_string(wallet)
        .map_err(|e| DbError::Corrupt(format!("wallet_upsert encode: {e}")))?;
    with_conn(database, |conn| {
        let next_index: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_index) + 1, 0) FROM wallets",
                [],
                |row| row.get(0),
            )
            .map_err(DbError::from)?;
        conn.execute(
            "INSERT INTO wallets
                 (id, name, chain_id, is_watch_only, include_in_portfolio_total,
                  sort_index, payload, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 name                       = excluded.name,
                 chain_id                 = excluded.chain_id,
                 is_watch_only              = excluded.is_watch_only,
                 include_in_portfolio_total = excluded.include_in_portfolio_total,
                 payload                    = excluded.payload,
                 updated_at                 = excluded.updated_at",
            params![
                wallet.id,
                wallet.name,
                wallet.chain_id,
                wallet.is_watch_only(),
                wallet.include_in_portfolio_total,
                next_index,
                payload,
                now_secs(),
            ],
        )
        .map_err(DbError::from)?;
        Ok(())
    })
}

/// Load wallets in stored display order; refuse undecodable records without modifying them.
pub fn wallet_load_all(database: &WalletDatabase) -> Result<Vec<WalletState>, DbError> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare("SELECT id, payload FROM wallets ORDER BY sort_index ASC")
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(DbError::from)?;
        let mut wallets = Vec::new();
        for row in rows {
            let (id, payload) = row.map_err(DbError::from)?;
            wallets.push(
                serde_json::from_str(&payload)
                    .map_err(|e| DbError::Corrupt(format!("wallet_load_all decode {id}: {e}")))?,
            );
        }
        Ok(wallets)
    })
}
