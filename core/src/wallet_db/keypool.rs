use super::*;

// ── Keypool types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct KeypoolState {
    pub next_external_index: i64,
    pub next_change_index: i64,
    pub reserved_receive_index: Option<i64>,
}

// ── Keypool CRUD ──────────────────────────────────────────────────────────────

/// Upsert keypool state for one (wallet, chain) pair.
pub fn keypool_save(
    database: &WalletDatabase,
    wallet_id: &str,
    chain_id: &str,
    state: &KeypoolState,
) -> Result<(), String> {
    with_conn(database, |conn| {
        conn.execute(
            "INSERT INTO wallet_keypool
                 (wallet_id, chain_id, next_external_index, next_change_index,
                  reserved_receive_index, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(wallet_id, chain_id) DO UPDATE SET
                 next_external_index    = excluded.next_external_index,
                 next_change_index      = excluded.next_change_index,
                 reserved_receive_index = excluded.reserved_receive_index,
                 updated_at             = excluded.updated_at",
            params![
                wallet_id,
                chain_id,
                state.next_external_index,
                state.next_change_index,
                state.reserved_receive_index,
                now_secs(),
            ],
        )
        .map_err(|e| format!("keypool_save: {e}"))?;
        Ok(())
    })
}

/// Load the entire keypool table as a nested map: chain → wallet_id → state.
/// This is the startup bulk-load that replaces reading UserDefaults JSON.
pub fn keypool_load_all(
    database: &WalletDatabase,
) -> Result<
    std::collections::HashMap<String, std::collections::HashMap<String, KeypoolState>>,
    String,
> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT chain_id, wallet_id, next_external_index, next_change_index, reserved_receive_index
                 FROM wallet_keypool",
            )
            .map_err(|e| format!("keypool_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?, // chain_id
                    row.get::<_, String>(1)?, // wallet_id
                    KeypoolState {
                        next_external_index: row.get(2)?,
                        next_change_index: row.get(3)?,
                        reserved_receive_index: row.get(4)?,
                    },
                ))
            })
            .map_err(|e| format!("keypool_load_all query: {e}"))?;
        let mut outer: std::collections::HashMap<
            String,
            std::collections::HashMap<String, KeypoolState>,
        > = std::collections::HashMap::new();
        for row in rows {
            let (chain, wallet, state) = row.map_err(|e| format!("keypool_load_all row: {e}"))?;
            outer.entry(chain).or_default().insert(wallet, state);
        }
        Ok(outer)
    })
}
