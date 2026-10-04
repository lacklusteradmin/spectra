//! A completed fetch's provider continuation survives process restarts.
use super::{WalletDatabase, error::DbError, with_conn};
use crate::registry::Chain;
use rusqlite::params;

pub(crate) type StoredHistoryPage = (Chain, String, Option<String>, u32, bool);

pub(crate) fn history_pagination_load(
    database: &WalletDatabase,
) -> Result<Vec<StoredHistoryPage>, DbError> {
    with_conn(database, |connection| {
        let mut statement = connection.prepare(
            "SELECT chain_id, wallet_id, cursor, page, exhausted FROM history_pagination",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from)
    })
}

pub(crate) fn history_pagination_save(
    database: &WalletDatabase,
    row: &StoredHistoryPage,
) -> Result<(), DbError> {
    with_conn(database, |connection| {
        connection.execute("INSERT INTO history_pagination (chain_id, wallet_id, cursor, page, exhausted) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(chain_id,wallet_id) DO UPDATE SET cursor=excluded.cursor,page=excluded.page,exhausted=excluded.exhausted", params![row.0,row.1,row.2,row.3,row.4])?;
        Ok(())
    })
}

pub(crate) fn history_pagination_delete(
    database: &WalletDatabase,
    chain: Option<Chain>,
    wallet: Option<&str>,
) -> Result<(), DbError> {
    with_conn(database, |connection| {
        connection.execute("DELETE FROM history_pagination WHERE (?1 IS NULL OR chain_id=?1) AND (?2 IS NULL OR wallet_id=?2)", params![chain,wallet])?;
        Ok(())
    })
}
