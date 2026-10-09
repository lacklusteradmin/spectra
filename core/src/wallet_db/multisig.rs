//! A multisig wallet's sessions, each its JSON payload: a spend its
//! cosigners sign, kept until it is submitted or discarded.
use rusqlite::{OptionalExtension, params};

use super::WalletDatabase;
use super::connection::with_conn;
use super::error::DbError;

/// Store `payload` as session `id` of `wallet_id`, replacing what it was.
pub(crate) fn multisig_session_save(
    database: &WalletDatabase,
    id: &str,
    wallet_id: &str,
    payload: &str,
) -> Result<(), DbError> {
    with_conn(database, |conn| {
        conn.execute(
            "INSERT INTO multisig_sessions (id, wallet_id, payload) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
            params![id, wallet_id, payload],
        )
        .map_err(DbError::from)?;
        Ok(())
    })
}

pub(crate) fn multisig_session_load(
    database: &WalletDatabase,
    id: &str,
) -> Result<Option<String>, DbError> {
    with_conn(database, |conn| {
        conn.query_row(
            "SELECT payload FROM multisig_sessions WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(DbError::from)
    })
}

/// A wallet's sessions, oldest first.
pub(crate) fn multisig_sessions_for_wallet(
    database: &WalletDatabase,
    wallet_id: &str,
) -> Result<Vec<String>, DbError> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload FROM multisig_sessions WHERE wallet_id = ?1
                 ORDER BY json_extract(payload, '$.created_at'), id",
            )
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map(params![wallet_id], |row| row.get(0))
            .map_err(DbError::from)?;
        rows.collect::<Result<_, _>>().map_err(DbError::from)
    })
}

pub(crate) fn multisig_session_delete(database: &WalletDatabase, id: &str) -> Result<(), DbError> {
    with_conn(database, |conn| {
        conn.execute("DELETE FROM multisig_sessions WHERE id = ?1", params![id])
            .map_err(DbError::from)?;
        Ok(())
    })
}
