use super::*;

/// Insert or replace one wallet, keeping its existing position when it is
/// already stored and appending it to the end when it is new.
///
/// Use this for a single-wallet edit. To write a whole state snapshot use
/// [`app_state_save`], which also prunes wallets that are no longer present.
pub fn wallet_upsert(database: &WalletDatabase, wallet: &WalletState) -> Result<(), String> {
    let payload =
        serde_json::to_string(wallet).map_err(|e| format!("wallet_upsert encode: {e}"))?;
    with_conn(database, |conn| {
        let next_index: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_index) + 1, 0) FROM wallets",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("wallet_upsert next index: {e}"))?;
        conn.execute(
            "INSERT INTO wallets
                 (id, name, chain_name, is_watch_only, include_in_portfolio_total,
                  sort_index, payload, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 name                       = excluded.name,
                 chain_name                 = excluded.chain_name,
                 is_watch_only              = excluded.is_watch_only,
                 include_in_portfolio_total = excluded.include_in_portfolio_total,
                 payload                    = excluded.payload,
                 updated_at                 = excluded.updated_at",
            params![
                wallet.id,
                wallet.name,
                wallet.chain_name,
                wallet.is_watch_only,
                wallet.include_in_portfolio_total,
                next_index,
                payload,
                now_secs(),
            ],
        )
        .map_err(|e| format!("wallet_upsert: {e}"))?;
        Ok(())
    })
}

pub fn wallet_load(
    database: &WalletDatabase,
    wallet_id: &str,
) -> Result<Option<WalletState>, String> {
    with_conn(database, |conn| {
        let result = conn.query_row(
            "SELECT payload FROM wallets WHERE id = ?1",
            params![wallet_id],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(payload) => serde_json::from_str(&payload)
                .map(Some)
                .map_err(|e| format!("wallet_load decode {wallet_id}: {e}")),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("wallet_load: {e}")),
        }
    })
}

/// Load every wallet, in the stored display order. A row this build cannot
/// decode is skipped rather than fatal.
///
/// It used to be fatal, and that took the whole app down with it. Stored
/// shapes change here without migrations — Rule 0 says so outright — so every
/// such change orphans the rows the previous shape wrote. One of them failed
/// this load, which failed `app_state_load`, which failed `open_state`; every
/// call into core waits on `open_state`, so the app could no longer list
/// wallets, import one, or even reset itself. Shrinking
/// `CoreWalletDerivationOverrides` to two fields is what demonstrated it: a
/// row carrying the old `mnemonicWordlist` bricked the install, and the only
/// way out was deleting the app.
///
/// Refusing protected nothing. What the row holds — a name, a chain, cached
/// addresses and balances — is re-derivable from the secret the Keychain
/// still holds under the same wallet id, and the bytes are left untouched on
/// disk either way: no production write replaces the table wholesale, and a
/// skipped id is absent from both sides of every `AppStateChanges::between`,
/// so nothing prunes it. The strictness that does matter is on the way in —
/// `CoreWalletDerivationOverrides` keeps `deny_unknown_fields`, because
/// silently ignoring an override a caller *set* would derive a different
/// address than the caller asked for.
pub fn wallet_load_all(database: &WalletDatabase) -> Result<Vec<WalletState>, String> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare("SELECT id, payload FROM wallets ORDER BY sort_index ASC")
            .map_err(|e| format!("wallet_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("wallet_load_all query: {e}"))?;
        let mut wallets = Vec::new();
        for row in rows {
            let (id, payload) = row.map_err(|e| format!("wallet_load_all row: {e}"))?;
            match serde_json::from_str(&payload) {
                Ok(wallet) => wallets.push(wallet),
                Err(error) => {
                    tracing::warn!(wallet_id = %id, %error, "unreadable wallet row; skipping it")
                }
            }
        }
        Ok(wallets)
    })
}

/// Delete one wallet row. Does not touch that wallet's keypool, owned addresses
/// or history — use [`delete_wallet_data`] for the full teardown.
pub fn wallet_delete(database: &WalletDatabase, wallet_id: &str) -> Result<(), String> {
    with_conn(database, |conn| {
        conn.execute("DELETE FROM wallets WHERE id = ?1", params![wallet_id])
            .map_err(|e| format!("wallet_delete: {e}"))?;
        Ok(())
    })
}
