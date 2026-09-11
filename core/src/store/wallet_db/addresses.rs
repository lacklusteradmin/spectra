use super::*;

// ── Owned address types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct OwnedAddressRecord {
    pub wallet_id: String,
    pub chain_name: String,
    pub address: String,
    pub derivation_path: Option<String>,
    pub branch: Option<String>,
    pub branch_index: Option<i64>,
}

// ── Owned address CRUD ────────────────────────────────────────────────────────

/// Upsert a single owned address record (identified by wallet + chain + address).
pub fn address_save(db_path: &str, record: &OwnedAddressRecord) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "INSERT INTO wallet_owned_addresses
                 (wallet_id, chain_name, address, derivation_path, branch, branch_index, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(wallet_id, chain_name, address) DO UPDATE SET
                 derivation_path = excluded.derivation_path,
                 branch          = excluded.branch,
                 branch_index    = excluded.branch_index,
                 updated_at      = excluded.updated_at",
            params![
                record.wallet_id,
                record.chain_name,
                record.address,
                record.derivation_path,
                record.branch,
                record.branch_index,
                now_secs(),
            ],
        )
        .map_err(|e| format!("address_save: {e}"))?;
        Ok(())
    })
}

/// Load all owned addresses for a (wallet, chain) pair.
pub fn address_load_all(
    db_path: &str,
    wallet_id: &str,
    chain_name: &str,
) -> Result<Vec<OwnedAddressRecord>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT address, derivation_path, branch, branch_index
                 FROM wallet_owned_addresses WHERE wallet_id = ?1 AND chain_name = ?2",
            )
            .map_err(|e| format!("address_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map(params![wallet_id, chain_name], |row| {
                Ok(OwnedAddressRecord {
                    wallet_id: wallet_id.to_string(),
                    chain_name: chain_name.to_string(),
                    address: row.get(0)?,
                    derivation_path: row.get(1)?,
                    branch: row.get(2)?,
                    branch_index: row.get(3)?,
                })
            })
            .map_err(|e| format!("address_load_all query: {e}"))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|e| format!("address_load_all row: {e}"))?);
        }
        Ok(records)
    })
}

/// Used at startup to bulk-restore the in-memory map.
pub fn address_load_all_chains(db_path: &str) -> Result<Vec<OwnedAddressRecord>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT wallet_id, chain_name, address, derivation_path, branch, branch_index
                 FROM wallet_owned_addresses",
            )
            .map_err(|e| format!("address_load_all_chains prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(OwnedAddressRecord {
                    wallet_id: row.get(0)?,
                    chain_name: row.get(1)?,
                    address: row.get(2)?,
                    derivation_path: row.get(3)?,
                    branch: row.get(4)?,
                    branch_index: row.get(5)?,
                })
            })
            .map_err(|e| format!("address_load_all_chains query: {e}"))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|e| format!("address_load_all_chains row: {e}"))?);
        }
        Ok(records)
    })
}

/// Remove all owned address records for a deleted wallet.
pub fn address_delete_for_wallet(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_owned_addresses WHERE wallet_id = ?1",
            params![wallet_id],
        )
        .map_err(|e| format!("address_delete_for_wallet: {e}"))?;
        Ok(())
    })
}

/// Remove all owned address records for a chain (e.g. after a rescan).
pub fn address_delete_for_chain(db_path: &str, chain_name: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_owned_addresses WHERE chain_name = ?1",
            params![chain_name],
        )
        .map_err(|e| format!("address_delete_for_chain: {e}"))?;
        Ok(())
    })
}

/// Wipe the owned address table (full reset).
pub fn address_delete_all(db_path: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute("DELETE FROM wallet_owned_addresses", [])
            .map_err(|e| format!("address_delete_all: {e}"))?;
        Ok(())
    })
}
