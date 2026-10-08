//! Each Zcash wallet's shielded state: a librustzcash wallet database of its
//! own (zcash_client_sqlite), in `zcash/` beside Spectra's database. It holds
//! the account's viewing keys, scanned notes and note commitment trees —
//! never a spending key — and is deleted with the wallet.

use std::path::{Path, PathBuf};

use rand::rngs::OsRng;
use zcash_client_sqlite::{WalletDb, util::SystemClock};
use zcash_protocol::consensus::Network;

use crate::wallet_db::WalletDatabase;
use crate::wallet_db::error::DbError;

pub(crate) type ZcashDb = WalletDb<rusqlite::Connection, Network, SystemClock, OsRng>;

/// Where `wallet_id`'s shielded database lives.
pub(crate) fn zcash_db_path(
    database: &WalletDatabase,
    wallet_id: &str,
) -> Result<PathBuf, DbError> {
    if wallet_id.is_empty()
        || !wallet_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(DbError::Invalid("invalid wallet id".into()));
    }
    let directory = Path::new(database.path())
        .parent()
        .filter(|_| database.path() != ":memory:")
        .ok_or_else(|| DbError::Invalid("Shielded funds need a database on disk".into()))?;
    Ok(directory.join("zcash").join(format!("{wallet_id}.sqlite")))
}

/// Open (creating or migrating) the shielded database at `path`.
pub(crate) fn open_zcash_db(path: &Path, network: Network) -> Result<ZcashDb, DbError> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)
            .map_err(|error| DbError::Invalid(format!("zcash database directory: {error}")))?;
    }
    let mut db =
        WalletDb::for_path(path, network, SystemClock, OsRng).map_err(|source| DbError::Open {
            path: path.display().to_string(),
            source,
        })?;
    zcash_client_sqlite::wallet::init::init_wallet_db(&mut db, None)
        .map_err(|error| DbError::Corrupt(format!("zcash database migration: {error}")))?;
    Ok(db)
}

/// Remove the shielded database at `path` and SQLite's files beside it.
pub(crate) fn delete_zcash_db(path: &Path) -> Result<(), DbError> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(DbError::Invalid(format!(
                    "remove {}: {error}",
                    file.display()
                )));
            }
        }
    }
    Ok(())
}
