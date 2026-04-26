//! WalletService — state persistence (SQLite-backed load/save/delete).
//!
//! Sliced out of `service/mod.rs`. The `WalletService` type itself stays in
//! `mod.rs`; methods here live in a separate `impl` block (Rust permits
//! multiple impl blocks per type, and UniFFI exports them as if they were one).

#![allow(unused_imports)]

use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Load the JSON state blob stored under `key` in the SQLite database at
    /// `db_path`. Returns an empty JSON object `"{}"` when no value has been
    /// saved yet. Thread-safe: rusqlite is called in `spawn_blocking`.
    pub async fn load_state(
        &self,
        db_path: String,
        key: String,
    ) -> Result<String, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            sqlite_load(&db_path, &key)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Persist the JSON state blob under `key` in the SQLite database at
    /// `db_path`. Creates the file (and the `state` table) on first use.
    pub async fn save_state(
        &self,
        db_path: String,
        key: String,
        state_json: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            sqlite_save(&db_path, &key, &state_json)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn save_app_settings_typed(
        &self,
        db_path: String,
        settings: PersistedAppSettings,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let json = serde_json::to_string(&settings)
                .map_err(|e| format!("save_app_settings_typed serialize: {e}"))?;
            sqlite_save(&db_path, "app.settings.v1", &json)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn load_app_settings_typed(
        &self,
        db_path: String,
    ) -> Result<Option<PersistedAppSettings>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            let json = sqlite_load(&db_path, "app.settings.v1")?;
            if json == "{}" {
                return Ok(None);
            }
            serde_json::from_str::<PersistedAppSettings>(&json)
                .map(Some)
                .map_err(|e| format!("load_app_settings_typed deserialize: {e}"))
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Persist keypool state using typed record (no JSON intermediate).
    pub async fn save_keypool_state_typed(
        &self,
        db_path: String,
        wallet_id: String,
        chain_name: String,
        state: crate::wallet_db::KeypoolState,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::keypool_save(&db_path, &wallet_id, &chain_name, &state)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn load_all_keypool_state_typed(
        &self,
        db_path: String,
    ) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, crate::wallet_db::KeypoolState>>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || crate::wallet_db::keypool_load_all(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Remove all keypool state for a wallet (called when a wallet is deleted).
    pub async fn delete_keypool_for_wallet(
        &self,
        db_path: String,
        wallet_id: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::keypool_delete_for_wallet(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Remove all keypool state for a chain (called when the user switches network modes,
    /// triggering a rescan).
    pub async fn delete_keypool_for_chain(
        &self,
        db_path: String,
        chain_name: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::keypool_delete_for_chain(&db_path, &chain_name)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Upsert a single owned address record.
    pub async fn save_owned_address_typed(
        &self,
        db_path: String,
        record: crate::wallet_db::OwnedAddressRecord,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::address_save(&db_path, &record)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn load_all_owned_addresses_typed(
        &self,
        db_path: String,
    ) -> Result<Vec<crate::wallet_db::OwnedAddressRecord>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || crate::wallet_db::address_load_all_chains(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Remove all owned address records for a deleted wallet.
    pub async fn delete_owned_addresses_for_wallet(
        &self,
        db_path: String,
        wallet_id: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::address_delete_for_wallet(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Remove all owned address records for a chain (called after a full rescan).
    pub async fn delete_owned_addresses_for_chain(
        &self,
        db_path: String,
        chain_name: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::address_delete_for_chain(&db_path, &chain_name)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Remove all relational wallet state (keypool + addresses) for a deleted wallet.
    /// This is the single call to make when a wallet is removed.
    pub async fn delete_wallet_relational_data(
        &self,
        db_path: String,
        wallet_id: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::delete_wallet_data(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Upsert a batch of transaction history records. `records[*].payload`
    /// is the typed `CorePersistedTransactionRecord`; Rust serializes to JSON
    /// for the SQLite TEXT column internally — no JSON crosses the FFI.
    pub async fn upsert_history_records(
        &self,
        db_path: String,
        records: Vec<crate::wallet_db::HistoryRecord>,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_upsert_batch(&db_path, &records)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    pub async fn fetch_all_history_records_typed(
        &self,
        db_path: String,
    ) -> Result<Vec<crate::wallet_db::HistoryRecord>, SpectraBridgeError> {
        tokio::task::spawn_blocking(move || crate::wallet_db::history_fetch_all(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Delete history records by ID.
    pub async fn delete_history_records(
        &self,
        db_path: String,
        ids: Vec<String>,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || crate::wallet_db::history_delete(&db_path, &ids))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Atomically replace ALL history records with the provided batch.
    pub async fn replace_all_history_records(
        &self,
        db_path: String,
        records: Vec<crate::wallet_db::HistoryRecord>,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_replace_all(&db_path, &records)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Delete all history records (hard reset).
    pub async fn clear_all_history_records(
        &self,
        db_path: String,
    ) -> Result<(), SpectraBridgeError> {
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_clear(&db_path)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(SpectraBridgeError::from)
    }

    /// Seed the in-memory wallet list from typed `WalletSummary` records.
    pub async fn init_wallet_state_direct(
        &self,
        wallets: Vec<WalletSummary>,
    ) -> Result<(), SpectraBridgeError> {
        let mut state = self.wallet_state.write().await;
        state.wallets = wallets;
        Ok(())
    }

    /// Add or replace a wallet from a typed `WalletSummary` record.
    pub async fn upsert_wallet_direct(
        &self,
        wallet: WalletSummary,
    ) -> Result<(), SpectraBridgeError> {
        let mut state = self.wallet_state.write().await;
        reduce_state_in_place(&mut state, StateCommand::UpsertWallet { wallet });
        Ok(())
    }

    /// Current cursor for the next history fetch, or `None` if no fetch has
    /// been done yet. Pass the returned value as the starting point for the
    /// next page request.
    pub fn history_next_cursor(&self, chain_id: u32, wallet_id: String) -> Option<String> {
        self.history_pagination.cursor(chain_id, &wallet_id)
    }

    /// Current zero-based page index for page-numbered chains (EVM, etc.).
    pub fn history_next_page(&self, chain_id: u32, wallet_id: String) -> u32 {
        self.history_pagination.page(chain_id, &wallet_id)
    }

    /// Returns `true` when all history pages have been fetched and no more
    /// pages are available. Swift should not attempt another fetch until
    /// `reset_history` is called.
    pub fn is_history_exhausted(&self, chain_id: u32, wallet_id: String) -> bool {
        self.history_pagination.is_exhausted(chain_id, &wallet_id)
    }

    /// Record the cursor returned after a successful cursor-based fetch (UTXO
    /// chains). Pass `None` when the chain confirms there are no more pages —
    /// this marks the chain as exhausted.
    pub fn advance_history_cursor(
        &self,
        chain_id: u32,
        wallet_id: String,
        next_cursor: Option<String>,
    ) {
        self.history_pagination
            .advance_cursor(chain_id, &wallet_id, next_cursor);
    }

    /// Increment the page counter after a successful page-based fetch (EVM,
    /// etc.). Pass `is_last = true` when the returned page was empty or the
    /// chain indicated no next page.
    pub fn advance_history_page(&self, chain_id: u32, wallet_id: String, is_last: bool) {
        self.history_pagination
            .advance_page(chain_id, &wallet_id, is_last);
    }

    /// Directly set the page counter to `page`. For page-based chains (EVM)
    /// where Swift tracks absolute page numbers (1-indexed). Swift sets the
    /// page to 1 on reset and stores the page that was just fetched after each
    /// successful request.
    pub fn set_history_page(&self, chain_id: u32, wallet_id: String, page: u32) {
        self.history_pagination.set_page(chain_id, &wallet_id, page);
    }

    /// Explicitly mark a (chain, wallet) pair as exhausted or not. Used when
    /// Swift detects an empty page without going through `advance_history_*`.
    pub fn set_history_exhausted(&self, chain_id: u32, wallet_id: String, exhausted: bool) {
        self.history_pagination
            .set_exhausted(chain_id, &wallet_id, exhausted);
    }

    /// Reset pagination state for one (chain, wallet) pair — clears cursor,
    /// page, and exhaustion flag. Call after the user pulls-to-refresh or
    /// after a send confirmation.
    pub fn reset_history(&self, chain_id: u32, wallet_id: String) {
        self.history_pagination.reset(chain_id, &wallet_id);
    }

    /// Reset pagination for all chains of one wallet (e.g. wallet deleted or
    /// user triggers a full history refresh for that wallet).
    pub fn reset_history_for_wallet(&self, wallet_id: String) {
        self.history_pagination.reset_all_for_wallet(&wallet_id);
    }

    /// Reset pagination for all wallets on one chain (e.g. chain re-org or
    /// endpoint switch).
    pub fn reset_history_for_chain(&self, chain_id: u32) {
        self.history_pagination.reset_chain(chain_id);
    }

    /// Clear all history pagination state. Used on full account wipe / logout.
    pub fn reset_all_history(&self) {
        self.history_pagination.reset_all();
    }

    // ── Typed persistence: 3 stores that previously did Swift→Rust JSON-shuttle ────
    //
    // The `*_state` JSON shuttle (Swift loads JSON → calls decode FFI → gets
    // typed struct) is replaced with single typed methods that do load+decode
    // and encode+save inside Rust. Halves FFI traffic on every persist op and
    // removes a full JSON parse cost per load.

    /// Load the persisted price-alert store. Returns `None` if no value has
    /// been saved yet or if the on-disk shape can't be decoded.
    pub async fn load_price_alert_store(
        &self,
        db_path: String,
        key: String,
    ) -> Result<Option<crate::store::persistence::models::CorePersistedPriceAlertStore>, SpectraBridgeError> {
        let json = tokio::task::spawn_blocking(move || sqlite_load(&db_path, &key))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)?;
        if json == "{}" {
            return Ok(None);
        }
        Ok(serde_json::from_str(&json).ok())
    }

    /// Persist the price-alert store typed.
    pub async fn save_price_alert_store(
        &self,
        db_path: String,
        key: String,
        value: crate::store::persistence::models::CorePersistedPriceAlertStore,
    ) -> Result<(), SpectraBridgeError> {
        let json = serde_json::to_string(&value).map_err(SpectraBridgeError::from)?;
        tokio::task::spawn_blocking(move || sqlite_save(&db_path, &key, &json))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }

    /// Load the persisted address-book store.
    pub async fn load_address_book_store(
        &self,
        db_path: String,
        key: String,
    ) -> Result<Option<crate::store::persistence::models::CorePersistedAddressBookStore>, SpectraBridgeError> {
        let json = tokio::task::spawn_blocking(move || sqlite_load(&db_path, &key))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)?;
        if json == "{}" {
            return Ok(None);
        }
        Ok(serde_json::from_str(&json).ok())
    }

    /// Persist the address-book store typed.
    pub async fn save_address_book_store(
        &self,
        db_path: String,
        key: String,
        value: crate::store::persistence::models::CorePersistedAddressBookStore,
    ) -> Result<(), SpectraBridgeError> {
        let json = serde_json::to_string(&value).map_err(SpectraBridgeError::from)?;
        tokio::task::spawn_blocking(move || sqlite_save(&db_path, &key, &json))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)
    }
}
