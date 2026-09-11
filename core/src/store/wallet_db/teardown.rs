use super::*;

// ── Combined wallet teardown ──────────────────────────────────────────────────

/// Remove every trace of a deleted wallet: its row, keypool, owned addresses
/// and history records.
pub fn delete_wallet_data(db_path: &str, wallet_id: &str) -> Result<(), String> {
    with_conn(db_path, |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("delete_wallet_data begin: {e}"))?;
        for (table, column) in [
            ("wallets", "id"),
            ("wallet_keypool", "wallet_id"),
            ("wallet_owned_addresses", "wallet_id"),
            ("history_records", "wallet_id"),
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE {column} = ?1"),
                params![wallet_id],
            )
            .map_err(|e| format!("delete_wallet_data {table}: {e}"))?;
        }
        tx.commit()
            .map_err(|e| format!("delete_wallet_data commit: {e}"))
    })
}
