use super::*;
use crate::wallet_db::error::DbError;

// ── Owned address types ───────────────────────────────────────────────────────

/// Which branch of the account an address is on: BIP-44's external chain,
/// which hands out receive addresses, or its internal one, for change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AddressBranch {
    Receive,
    Change,
}

impl AddressBranch {
    /// The branch's index in a derivation path.
    pub fn index(self) -> u32 {
        match self {
            Self::Receive => 0,
            Self::Change => 1,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Receive => "receive",
            Self::Change => "change",
        }
    }
}

impl rusqlite::ToSql for AddressBranch {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl rusqlite::types::FromSql for AddressBranch {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        match value.as_str()? {
            "receive" => Ok(Self::Receive),
            "change" => Ok(Self::Change),
            other => Err(rusqlite::types::FromSqlError::Other(
                format!("unknown address branch {other:?}").into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct OwnedAddressRecord {
    pub wallet_id: String,
    pub chain_id: crate::registry::Chain,
    pub address: String,
    pub derivation_path: Option<String>,
    pub branch: Option<AddressBranch>,
    pub branch_index: Option<i64>,
}

// ── Owned address CRUD ────────────────────────────────────────────────────────

/// Upsert a single owned address record (identified by wallet + chain + address).
pub fn address_save(database: &WalletDatabase, record: &OwnedAddressRecord) -> Result<(), DbError> {
    with_conn(database, |conn| {
        conn.execute(
            "INSERT INTO wallet_owned_addresses
                 (wallet_id, chain_id, address, derivation_path, branch, branch_index, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(wallet_id, chain_id, address) DO UPDATE SET
                 derivation_path = excluded.derivation_path,
                 branch          = excluded.branch,
                 branch_index    = excluded.branch_index,
                 updated_at      = excluded.updated_at",
            params![
                record.wallet_id,
                record.chain_id,
                record.address,
                record.derivation_path,
                record.branch,
                record.branch_index,
                now_secs(),
            ],
        )
        .map_err(DbError::from)?;
        Ok(())
    })
}

/// Used at startup to bulk-restore the in-memory map.
pub fn address_load_all_chains(
    database: &WalletDatabase,
) -> Result<Vec<OwnedAddressRecord>, DbError> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT wallet_id, chain_id, address, derivation_path, branch, branch_index
                 FROM wallet_owned_addresses",
            )
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(OwnedAddressRecord {
                    wallet_id: row.get(0)?,
                    chain_id: row.get(1)?,
                    address: row.get(2)?,
                    derivation_path: row.get(3)?,
                    branch: row.get(4)?,
                    branch_index: row.get(5)?,
                })
            })
            .map_err(DbError::from)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(DbError::from)?);
        }
        Ok(records)
    })
}
