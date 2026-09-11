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
    db_path: &str,
    wallet_id: &str,
    chain_name: &str,
    state: &KeypoolState,
) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "INSERT INTO wallet_keypool
                 (wallet_id, chain_name, next_external_index, next_change_index,
                  reserved_receive_index, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(wallet_id, chain_name) DO UPDATE SET
                 next_external_index    = excluded.next_external_index,
                 next_change_index      = excluded.next_change_index,
                 reserved_receive_index = excluded.reserved_receive_index,
                 updated_at             = excluded.updated_at",
            params![
                wallet_id,
                chain_name,
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

/// Load keypool state for one (wallet, chain) pair.
pub fn keypool_load(
    db_path: &str,
    wallet_id: &str,
    chain_name: &str,
) -> Result<Option<KeypoolState>, String> {
    with_conn(db_path, |conn| {
        let result = conn.query_row(
            "SELECT next_external_index, next_change_index, reserved_receive_index
             FROM wallet_keypool WHERE wallet_id = ?1 AND chain_name = ?2",
            params![wallet_id, chain_name],
            |row| {
                Ok(KeypoolState {
                    next_external_index: row.get(0)?,
                    next_change_index: row.get(1)?,
                    reserved_receive_index: row.get(2)?,
                })
            },
        );
        match result {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("keypool_load: {e}")),
        }
    })
}

/// Load all keypool state for a wallet across every chain it has used.
pub fn keypool_load_for_wallet(
    db_path: &str,
    wallet_id: &str,
) -> Result<std::collections::HashMap<String, KeypoolState>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT chain_name, next_external_index, next_change_index, reserved_receive_index
                 FROM wallet_keypool WHERE wallet_id = ?1",
            )
            .map_err(|e| format!("keypool_load_for_wallet prepare: {e}"))?;
        let rows = stmt
            .query_map(params![wallet_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    KeypoolState {
                        next_external_index: row.get(1)?,
                        next_change_index: row.get(2)?,
                        reserved_receive_index: row.get(3)?,
                    },
                ))
            })
            .map_err(|e| format!("keypool_load_for_wallet query: {e}"))?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (chain, state) = row.map_err(|e| format!("keypool_load_for_wallet row: {e}"))?;
            map.insert(chain, state);
        }
        Ok(map)
    })
}

/// Load all keypool state across every wallet for a given chain.
pub fn keypool_load_for_chain(
    db_path: &str,
    chain_name: &str,
) -> Result<std::collections::HashMap<String, KeypoolState>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT wallet_id, next_external_index, next_change_index, reserved_receive_index
                 FROM wallet_keypool WHERE chain_name = ?1",
            )
            .map_err(|e| format!("keypool_load_for_chain prepare: {e}"))?;
        let rows = stmt
            .query_map(params![chain_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    KeypoolState {
                        next_external_index: row.get(1)?,
                        next_change_index: row.get(2)?,
                        reserved_receive_index: row.get(3)?,
                    },
                ))
            })
            .map_err(|e| format!("keypool_load_for_chain query: {e}"))?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (wallet, state) = row.map_err(|e| format!("keypool_load_for_chain row: {e}"))?;
            map.insert(wallet, state);
        }
        Ok(map)
    })
}

/// Load the entire keypool table as a nested map: chain → wallet_id → state.
/// This is the startup bulk-load that replaces reading UserDefaults JSON.
pub fn keypool_load_all(
    db_path: &str,
) -> Result<
    std::collections::HashMap<String, std::collections::HashMap<String, KeypoolState>>,
    String,
> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT chain_name, wallet_id, next_external_index, next_change_index, reserved_receive_index
                 FROM wallet_keypool",
            )
            .map_err(|e| format!("keypool_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?, // chain_name
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

/// Remove all keypool entries for a deleted wallet.
pub fn keypool_delete_for_wallet(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_keypool WHERE wallet_id = ?1",
            params![wallet_id],
        )
        .map_err(|e| format!("keypool_delete_for_wallet: {e}"))?;
        Ok(())
    })
}

/// Remove all keypool entries for a chain (e.g. when the user switches network modes).
pub fn keypool_delete_for_chain(db_path: &str, chain_name: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute(
            "DELETE FROM wallet_keypool WHERE chain_name = ?1",
            params![chain_name],
        )
        .map_err(|e| format!("keypool_delete_for_chain: {e}"))?;
        Ok(())
    })
}

/// Drop every derivation artefact a chain accumulated: its reserved keypool
/// indices and its discovered owned addresses.
///
/// One transaction because a network switch needs both gone or neither. A
/// keypool that outlives its network hands out indices for addresses the new
/// one never derived, and owned addresses that outlive it are attributed to a
/// network they were not derived on — so a half-applied delete is worse than
/// a failed one, which at least leaves a state the next attempt can repeat.
pub fn chain_derivation_delete_for_chain(db_path: &str, chain_name: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("chain_derivation_delete_for_chain begin: {e}"))?;
        for table in ["wallet_keypool", "wallet_owned_addresses"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE chain_name = ?1"),
                params![chain_name],
            )
            .map_err(|e| format!("chain_derivation_delete_for_chain {table}: {e}"))?;
        }
        tx.commit()
            .map_err(|e| format!("chain_derivation_delete_for_chain commit: {e}"))
    })
}

/// Wipe the entire keypool table (full reset).
pub fn keypool_delete_all(db_path: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        conn.execute("DELETE FROM wallet_keypool", [])
            .map_err(|e| format!("keypool_delete_all: {e}"))?;
        Ok(())
    })
}
