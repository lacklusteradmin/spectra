//! History pagination cursor surface for `WalletService`.
//!
//! These methods are thin, synchronous delegations to `self.history_pagination`
//! (cursor/page/exhaustion bookkeeping for the per-(chain, wallet) history
//! feed). They were split out of the main `service.rs` impl blocks to keep that
//! file navigable. UniFFI 0.31 merges multiple `#[uniffi::export]` impl blocks
//! for the same type, so the FFI surface is unchanged by this move.

use super::*;

#[uniffi::export]
impl WalletService {
    /// Current cursor for the next history fetch, or `None` if no fetch has
    /// been done yet. Pass the returned value as the starting point for the
    /// next page request.
    pub fn history_next_cursor(&self, chain_id: String, wallet_id: String) -> Option<String> {
        self.history_pagination.cursor(&chain_id, &wallet_id)
    }

    /// Current zero-based page index for page-numbered chains (EVM, etc.).
    pub fn history_next_page(&self, chain_id: String, wallet_id: String) -> u32 {
        self.history_pagination.page(&chain_id, &wallet_id)
    }

    /// Returns `true` when all history pages have been fetched and no more
    /// pages are available. Swift should not attempt another fetch until
    /// `reset_history` is called.
    pub fn is_history_exhausted(&self, chain_id: String, wallet_id: String) -> bool {
        self.history_pagination.is_exhausted(&chain_id, &wallet_id)
    }

    /// Record the cursor returned after a successful cursor-based fetch (UTXO
    /// chains). Pass `None` when the chain confirms there are no more pages —
    /// this marks the chain as exhausted.
    pub fn advance_history_cursor(
        &self,
        chain_id: String,
        wallet_id: String,
        next_cursor: Option<String>,
    ) {
        self.history_pagination
            .advance_cursor(&chain_id, &wallet_id, next_cursor);
    }

    /// Increment the page counter after a successful page-based fetch (EVM,
    /// etc.). Pass `is_last = true` when the returned page was empty or the
    /// chain indicated no next page.
    pub fn advance_history_page(&self, chain_id: String, wallet_id: String, is_last: bool) {
        self.history_pagination
            .advance_page(&chain_id, &wallet_id, is_last);
    }

    /// Directly set the page counter to `page`. For page-based chains (EVM)
    /// where Swift tracks absolute page numbers (1-indexed). Swift sets the
    /// page to 1 on reset and stores the page that was just fetched after each
    /// successful request.
    pub fn set_history_page(&self, chain_id: String, wallet_id: String, page: u32) {
        self.history_pagination
            .set_page(&chain_id, &wallet_id, page);
    }

    /// Explicitly mark a (chain, wallet) pair as exhausted or not. Used when
    /// Swift detects an empty page without going through `advance_history_*`.
    pub fn set_history_exhausted(&self, chain_id: String, wallet_id: String, exhausted: bool) {
        self.history_pagination
            .set_exhausted(&chain_id, &wallet_id, exhausted);
    }

    /// Reset pagination state for one (chain, wallet) pair — clears cursor,
    /// page, and exhaustion flag. Call after the user pulls-to-refresh or
    /// after a send confirmation.
    pub fn reset_history(&self, chain_id: String, wallet_id: String) {
        self.history_pagination.reset(&chain_id, &wallet_id);
    }

    /// Reset pagination for all chains of one wallet (e.g. wallet deleted or
    /// user triggers a full history refresh for that wallet).
    pub fn reset_history_for_wallet(&self, wallet_id: String) {
        self.history_pagination.reset_all_for_wallet(&wallet_id);
    }

    /// Reset pagination for all wallets on one chain (e.g. chain re-org or
    /// endpoint switch).
    pub fn reset_history_for_chain(&self, chain_id: String) {
        self.history_pagination.reset_chain(&chain_id);
    }

    /// Clear all history pagination state. Used on full account wipe / logout.
    pub fn reset_all_history(&self) {
        self.history_pagination.reset_all();
    }
}
