//! Per-wallet history pagination state: cursor strings, page counters, and
//! exhaustion flags — one entry per (chain_id, wallet_id) pair.
//!
//! Designed to replace the 22 @Published cursor/exhaustion vars that Swift's
//! StoreHistoryRefresh.swift currently owns. Swift still drives the actual
//! HTTP fetches; this store only owns *where we are* in the pagination.
//!
//! ## Key conventions
//!
//! - UTXO chains (BTC / BCH / BSV / LTC / DOGE / Tron) use a `cursor`
//!   string (a txid or offset token returned by the last page).
//! - EVM and other page-numbered chains use a `page` counter (0 = start).
//! - `exhausted = true` means no more pages exist; Swift should not attempt
//!   another fetch until `reset` is called.

use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Default)]
struct PaginationEntry {
    /// Opaque cursor returned by the last successful fetch. `None` = not yet
    /// started (fetch from the beginning).
    cursor: Option<String>,
    /// Zero-based page index for page-numbered chains (EVM, etc.).
    page: u32,
    /// Set to `true` when the last fetch returned an empty or terminal page.
    exhausted: bool,
}

/// Thread-safe in-memory pagination store. The `WalletService` holds one of
/// these as an `Arc<HistoryPaginationStore>` for the app's lifetime.
pub struct HistoryPaginationStore {
    inner: RwLock<HashMap<String, PaginationEntry>>,
}

impl HistoryPaginationStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    fn key(chain_id: u32, wallet_id: &str) -> String {
        format!("{chain_id}:{wallet_id}")
    }

    // ----------------------------------------------------------------
    // Reads
    // ----------------------------------------------------------------

    /// Current cursor for the next fetch, or `None` if no fetch has been done.
    pub fn cursor(&self, chain_id: u32, wallet_id: &str) -> Option<String> {
        self.inner
            .read()
            .ok()?
            .get(&Self::key(chain_id, wallet_id))
            .and_then(|e| e.cursor.clone())
    }

    /// Current page index (0-based) for page-numbered chains.
    pub fn page(&self, chain_id: u32, wallet_id: &str) -> u32 {
        self.inner
            .read()
            .ok()
            .and_then(|m| m.get(&Self::key(chain_id, wallet_id)).map(|e| e.page))
            .unwrap_or(0)
    }

    /// Whether all history pages have been fetched.
    pub fn is_exhausted(&self, chain_id: u32, wallet_id: &str) -> bool {
        self.inner
            .read()
            .ok()
            .and_then(|m| m.get(&Self::key(chain_id, wallet_id)).map(|e| e.exhausted))
            .unwrap_or(false)
    }

    // ----------------------------------------------------------------
    // Writes
    // ----------------------------------------------------------------

    /// Record the cursor returned after a successful fetch. A `None` cursor
    /// means the chain confirmed there are no more pages — mark as exhausted.
    pub fn advance_cursor(
        &self,
        chain_id: u32,
        wallet_id: &str,
        next_cursor: Option<String>,
    ) {
        if let Ok(mut map) = self.inner.write() {
            let entry = map
                .entry(Self::key(chain_id, wallet_id))
                .or_default();
            if let Some(c) = next_cursor {
                entry.cursor = Some(c);
                entry.exhausted = false;
            } else {
                entry.exhausted = true;
            }
        }
    }

    /// Advance the page counter for page-numbered chains. Call after a
    /// successful non-empty fetch. Pass `is_last = true` when the page was
    /// the terminal page (empty result or chain said "no next").
    pub fn advance_page(
        &self,
        chain_id: u32,
        wallet_id: &str,
        is_last: bool,
    ) {
        if let Ok(mut map) = self.inner.write() {
            let entry = map
                .entry(Self::key(chain_id, wallet_id))
                .or_default();
            entry.page = entry.page.saturating_add(1);
            if is_last {
                entry.exhausted = true;
            }
        }
    }

    /// Directly set the page counter to `page`. Use this for page-based chains
    /// where Swift tracks the absolute page number (e.g. EVM chains start at
    /// page 1 for the first request and increment per load-more).
    pub fn set_page(&self, chain_id: u32, wallet_id: &str, page: u32) {
        if let Ok(mut map) = self.inner.write() {
            map.entry(Self::key(chain_id, wallet_id))
                .or_default()
                .page = page;
        }
    }

    /// Explicitly mark exhausted (e.g. when an empty page is returned).
    pub fn set_exhausted(&self, chain_id: u32, wallet_id: &str, exhausted: bool) {
        if let Ok(mut map) = self.inner.write() {
            map.entry(Self::key(chain_id, wallet_id))
                .or_default()
                .exhausted = exhausted;
        }
    }

    /// Reset a single (chain, wallet) pair — clears cursor, page, and
    /// exhaustion. Call when the user refreshes from the top or after a send.
    pub fn reset(&self, chain_id: u32, wallet_id: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(&Self::key(chain_id, wallet_id));
        }
    }

    /// Reset all chains for a wallet (called when a wallet is deleted or
    /// when the user triggers a global history refresh).
    pub fn reset_all_for_wallet(&self, wallet_id: &str) {
        if let Ok(mut map) = self.inner.write() {
            let prefix = format!("{}:{}", u32::MAX, wallet_id); // marker to avoid false matches
            let _ = prefix; // unused; pattern-match on suffix instead
            map.retain(|key, _| {
                // key format is "{chain_id}:{wallet_id}"
                // Remove entries whose suffix equals ":{wallet_id}"
                !key.ends_with(&format!(":{wallet_id}"))
            });
        }
    }

    /// Reset all pagination state for a specific chain across all wallets.
    pub fn reset_chain(&self, chain_id: u32) {
        let prefix = format!("{chain_id}:");
        if let Ok(mut map) = self.inner.write() {
            map.retain(|key, _| !key.starts_with(&prefix));
        }
    }

    /// Clear everything. Used on full reset / account wipe.
    pub fn reset_all(&self) {
        if let Ok(mut map) = self.inner.write() {
            map.clear();
        }
    }
}

// ----------------------------------------------------------------
// Default impl
// ----------------------------------------------------------------

impl Default for HistoryPaginationStore {
    fn default() -> Self {
        Self::new()
    }
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_chain_starts_empty() {
        let store = HistoryPaginationStore::new();
        assert!(store.cursor(0, "wallet-1").is_none());
        assert!(!store.is_exhausted(0, "wallet-1"));
        assert_eq!(store.page(0, "wallet-1"), 0);
    }

    #[test]
    fn advance_cursor_tracks_state() {
        let store = HistoryPaginationStore::new();
        store.advance_cursor(0, "wallet-1", Some("abc123".to_string()));
        assert_eq!(store.cursor(0, "wallet-1").as_deref(), Some("abc123"));
        assert!(!store.is_exhausted(0, "wallet-1"));

        // Terminal: no next cursor → exhausted.
        store.advance_cursor(0, "wallet-1", None);
        assert!(store.is_exhausted(0, "wallet-1"));
    }

    #[test]
    fn advance_page_increments_and_exhausts() {
        let store = HistoryPaginationStore::new();
        store.advance_page(1, "wallet-2", false);
        assert_eq!(store.page(1, "wallet-2"), 1);
        assert!(!store.is_exhausted(1, "wallet-2"));

        store.advance_page(1, "wallet-2", true);
        assert_eq!(store.page(1, "wallet-2"), 2);
        assert!(store.is_exhausted(1, "wallet-2"));
    }

    #[test]
    fn reset_clears_single_entry() {
        let store = HistoryPaginationStore::new();
        store.advance_cursor(0, "wallet-1", Some("tx1".to_string()));
        store.advance_cursor(0, "wallet-2", Some("tx2".to_string()));

        store.reset(0, "wallet-1");

        assert!(store.cursor(0, "wallet-1").is_none());
        assert_eq!(store.cursor(0, "wallet-2").as_deref(), Some("tx2"));
    }

    #[test]
    fn reset_all_for_wallet_removes_all_chains() {
        let store = HistoryPaginationStore::new();
        store.advance_cursor(0, "wallet-1", Some("tx-btc".to_string()));
        store.advance_page(1, "wallet-1", false);
        store.advance_cursor(4, "wallet-1", Some("tx-xrp".to_string()));
        store.advance_cursor(0, "wallet-2", Some("tx-btc2".to_string()));

        store.reset_all_for_wallet("wallet-1");

        assert!(store.cursor(0, "wallet-1").is_none());
        assert_eq!(store.page(1, "wallet-1"), 0);
        assert!(store.cursor(4, "wallet-1").is_none());
        // wallet-2 unaffected
        assert_eq!(store.cursor(0, "wallet-2").as_deref(), Some("tx-btc2"));
    }

    #[test]
    fn reset_chain_removes_all_wallets_on_chain() {
        let store = HistoryPaginationStore::new();
        store.advance_cursor(0, "wallet-1", Some("tx1".to_string()));
        store.advance_cursor(0, "wallet-2", Some("tx2".to_string()));
        store.advance_cursor(1, "wallet-1", Some("eth-tx".to_string()));

        store.reset_chain(0);

        assert!(store.cursor(0, "wallet-1").is_none());
        assert!(store.cursor(0, "wallet-2").is_none());
        assert_eq!(store.cursor(1, "wallet-1").as_deref(), Some("eth-tx"));
    }
}
