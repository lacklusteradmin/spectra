use super::*;

// ── Combined wallet teardown ──────────────────────────────────────────────────

/// Remove every trace of a deleted wallet: its row, keypool, owned addresses
/// and history records.
pub fn delete_wallet_data(database: &WalletDatabase, wallet_id: &str) -> Result<(), String> {
    with_conn(database, |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("delete_wallet_data begin: {e}"))?;

        tx.execute(
            "DELETE FROM monero_wallets WHERE wallet_id=?1",
            params![wallet_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM send_reservations WHERE artifact_id IN (SELECT id FROM send_artifacts WHERE json_extract(payload,'$.view.wallet_id')=?1)", params![wallet_id]).map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM send_artifacts WHERE json_extract(payload,'$.view.wallet_id')=?1",
            params![wallet_id],
        )
        .map_err(|e| e.to_string())?;
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
