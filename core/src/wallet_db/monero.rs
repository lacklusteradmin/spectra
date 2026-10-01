//! Encrypted local Monero scan cache; the decryption key lives in SecretStore.

use super::*;
use crate::wallet_db::error::DbError;
use rusqlite::OptionalExtension;
pub(crate) fn monero_load(
    database: &WalletDatabase,
    wallet_id: &str,
    chain_id: crate::registry::Chain,
) -> Result<Option<(u64, String)>, DbError> {
    with_conn(database, |conn| {
        conn.query_row(
            "SELECT revision,payload FROM monero_wallets WHERE wallet_id=?1 AND chain_id=?2",
            params![wallet_id, chain_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(DbError::from)
    })
}
pub(crate) fn monero_save(
    database: &WalletDatabase,
    wallet_id: &str,
    chain_id: crate::registry::Chain,
    revision: Option<u64>,
    payload: &str,
) -> Result<(), DbError> {
    with_conn(database, |conn| {
        let changed=match revision {
            None=>conn.execute("INSERT INTO monero_wallets(wallet_id,chain_id,revision,payload) VALUES(?1,?2,0,?3)",params![wallet_id,chain_id,payload]),
            Some(revision)=>conn.execute("UPDATE monero_wallets SET revision=revision+1,payload=?4 WHERE wallet_id=?1 AND chain_id=?2 AND revision=?3",params![wallet_id,chain_id,revision,payload]),
        }.map_err(DbError::from)?;
        if changed != 1 {
            return Err(DbError::Invalid(
                "Monero wallet changed concurrently; reload and retry".into(),
            ));
        }
        Ok(())
    })
}
