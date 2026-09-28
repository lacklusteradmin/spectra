use super::*;

// ── Owned address types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct OwnedAddressRecord {
    pub wallet_id: String,
    pub chain_id: String,
    pub address: String,
    pub derivation_path: Option<String>,
    pub branch: Option<String>,
    pub branch_index: Option<i64>,
}

// ── Owned address CRUD ────────────────────────────────────────────────────────

/// Upsert a single owned address record (identified by wallet + chain + address).
pub fn address_save(database: &WalletDatabase, record: &OwnedAddressRecord) -> Result<(), String> {
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
        .map_err(|e| format!("address_save: {e}"))?;
        Ok(())
    })
}

/// Used at startup to bulk-restore the in-memory map.
pub fn address_load_all_chains(
    database: &WalletDatabase,
) -> Result<Vec<OwnedAddressRecord>, String> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT wallet_id, chain_id, address, derivation_path, branch, branch_index
                 FROM wallet_owned_addresses",
            )
            .map_err(|e| format!("address_load_all_chains prepare: {e}"))?;
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
            .map_err(|e| format!("address_load_all_chains query: {e}"))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|e| format!("address_load_all_chains row: {e}"))?);
        }
        Ok(records)
    })
}
