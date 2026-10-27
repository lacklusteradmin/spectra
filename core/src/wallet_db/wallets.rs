use super::*;
use crate::wallet_db::error::DbError;

/// Insert or replace one wallet, keeping its existing position when it is
/// already stored and appending it to the end when it is new.
///
/// Use this for a single-wallet edit. To write a whole state snapshot use
/// [`app_state_save`], which also prunes wallets that are no longer present.
pub fn wallet_upsert(database: &WalletDatabase, wallet: &WalletState) -> Result<(), DbError> {
    with_conn(database, |conn| write_wallet_row(conn, wallet, None))
}

/// The one statement that writes a wallet's row: at `sort_index`, or where
/// it already stands (the end, for a new one) when that is `None`.
pub(super) fn write_wallet_row(
    conn: &rusqlite::Connection,
    wallet: &WalletState,
    sort_index: Option<i64>,
) -> Result<(), DbError> {
    let payload = serde_json::to_string(wallet)
        .map_err(|e| DbError::Corrupt(format!("wallet {} encode: {e}", wallet.id)))?;
    conn.execute(
        "INSERT INTO wallets (id, name, sort_index, payload, updated_at)
         VALUES (?1, ?2, coalesce(?3, (SELECT coalesce(max(sort_index) + 1, 0) FROM wallets)), ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
             name       = excluded.name,
             sort_index = coalesce(?3, sort_index),
             payload    = excluded.payload,
             updated_at = excluded.updated_at",
        params![wallet.id, wallet.name, sort_index, payload, now_secs()],
    )
    .map_err(DbError::from)?;
    Ok(())
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
