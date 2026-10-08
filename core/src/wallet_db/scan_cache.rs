//! A wallet's encrypted on-device scan cache — a Monero wallet's outputs, a
//! Litecoin wallet's MWEB outputs — one per wallet and network; the key that
//! decrypts it derives from the wallet's scan key, which lives in the
//! SecretStore.

use super::*;
use crate::wallet_db::error::DbError;
use rusqlite::OptionalExtension;
pub(crate) fn scan_cache_load(
    database: &WalletDatabase,
    wallet_id: &str,
    chain_id: crate::registry::Chain,
) -> Result<Option<(u64, String)>, DbError> {
    with_conn(database, |conn| {
        conn.query_row(
            "SELECT revision,payload FROM scan_caches WHERE wallet_id=?1 AND chain_id=?2",
            params![wallet_id, chain_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(DbError::from)
    })
}
pub(crate) fn scan_cache_save(
    database: &WalletDatabase,
    wallet_id: &str,
    chain_id: crate::registry::Chain,
    revision: Option<u64>,
    payload: &str,
) -> Result<(), DbError> {
    with_conn(database, |conn| {
        let changed=match revision {
            None=>conn.execute("INSERT INTO scan_caches(wallet_id,chain_id,revision,payload) VALUES(?1,?2,0,?3)",params![wallet_id,chain_id,payload]),
            Some(revision)=>conn.execute("UPDATE scan_caches SET revision=revision+1,payload=?4 WHERE wallet_id=?1 AND chain_id=?2 AND revision=?3",params![wallet_id,chain_id,revision,payload]),
        }.map_err(DbError::from)?;
        if changed != 1 {
            return Err(DbError::Invalid(
                "The scan cache changed concurrently; reload and retry".into(),
            ));
        }
        Ok(())
    })
}
