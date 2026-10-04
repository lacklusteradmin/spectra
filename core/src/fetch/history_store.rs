//! Per-wallet history pagination state: cursor strings, page counters, and
//! exhaustion flags — one entry per (chain_id, wallet_id) pair.
//!
//! Core's history services read and advance this state as they fetch pages.
//! Provider adapters determine whether pagination uses a cursor or page number.

use crate::registry::Chain;
use crate::wallet_db::{WalletDatabase, error::DbError};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default)]
struct PaginationEntry {
    cursor: Option<String>,
    page: u32,
    exhausted: bool,
}

#[derive(Default)]
struct PaginationState {
    entries: HashMap<(Chain, String), PaginationEntry>,
    database: Option<Arc<WalletDatabase>>,
}

/// Resident pagination is backed by the service's SQLite database after bind.
/// Disk writes succeed before a cursor becomes visible to subsequent readers.
pub struct HistoryPaginationStore {
    inner: RwLock<PaginationState>,
    pub(crate) operation_lock: tokio::sync::Mutex<()>,
}

impl HistoryPaginationStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(PaginationState::default()),
            operation_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) fn bind(&self, database: Arc<WalletDatabase>) -> Result<(), DbError> {
        let mut state = self
            .inner
            .write()
            .map_err(|_| DbError::Invalid("history pagination lock poisoned".into()))?;
        let rows = crate::wallet_db::history_pagination_load(&database)?;
        state.entries = rows
            .into_iter()
            .map(|(chain, wallet, cursor, page, exhausted)| {
                (
                    (chain, wallet),
                    PaginationEntry {
                        cursor,
                        page,
                        exhausted,
                    },
                )
            })
            .collect();
        state.database = Some(database);
        Ok(())
    }

    pub fn cursor(&self, chain: Chain, wallet: &str) -> Option<String> {
        self.inner
            .read()
            .ok()?
            .entries
            .get(&(chain, wallet.to_string()))
            .and_then(|entry| entry.cursor.clone())
    }
    pub fn page(&self, chain: Chain, wallet: &str) -> u32 {
        self.inner
            .read()
            .ok()
            .and_then(|state| {
                state
                    .entries
                    .get(&(chain, wallet.to_string()))
                    .map(|entry| entry.page)
            })
            .unwrap_or(0)
    }
    pub fn is_exhausted(&self, chain: Chain, wallet: &str) -> bool {
        self.inner
            .read()
            .ok()
            .and_then(|state| {
                state
                    .entries
                    .get(&(chain, wallet.to_string()))
                    .map(|entry| entry.exhausted)
            })
            .unwrap_or(false)
    }
    fn update(
        &self,
        chain: Chain,
        wallet: &str,
        modify: impl FnOnce(&mut PaginationEntry),
    ) -> Result<(), DbError> {
        let mut state = self
            .inner
            .write()
            .map_err(|_| DbError::Invalid("history pagination lock poisoned".into()))?;
        let key = (chain, wallet.to_string());
        let mut entry = state.entries.get(&key).cloned().unwrap_or_default();
        modify(&mut entry);
        if let Some(database) = &state.database {
            crate::wallet_db::history_pagination_save(
                database,
                &(
                    chain,
                    wallet.to_string(),
                    entry.cursor.clone(),
                    entry.page,
                    entry.exhausted,
                ),
            )?;
        }
        state.entries.insert(key, entry);
        Ok(())
    }
    pub(crate) fn advance_cursor_checked(
        &self,
        chain: Chain,
        wallet: &str,
        cursor: Option<String>,
    ) -> Result<(), DbError> {
        self.update(chain, wallet, |entry| {
            entry.exhausted = cursor.is_none();
            entry.cursor = cursor;
        })
    }
    pub(crate) fn set_page_checked(
        &self,
        chain: Chain,
        wallet: &str,
        page: u32,
        exhausted: bool,
    ) -> Result<(), DbError> {
        self.update(chain, wallet, |entry| {
            entry.page = page;
            entry.exhausted = exhausted;
        })
    }
    fn delete(&self, chain: Option<Chain>, wallet: Option<&str>) -> Result<(), DbError> {
        let mut state = self
            .inner
            .write()
            .map_err(|_| DbError::Invalid("history pagination lock poisoned".into()))?;
        if let Some(database) = &state.database {
            crate::wallet_db::history_pagination_delete(database, chain, wallet)?;
        }
        state.entries.retain(|(id, owner), _| {
            !((chain.is_none() || chain == Some(*id))
                && (wallet.is_none() || wallet == Some(owner.as_str())))
        });
        Ok(())
    }
    pub fn reset(&self, chain: Chain, wallet: &str) {
        report(self.delete(Some(chain), Some(wallet)));
    }
    pub fn reset_all_for_wallet(&self, wallet: &str) {
        report(self.delete(None, Some(wallet)));
    }
    pub fn reset_chain(&self, chain: Chain) {
        report(self.delete(Some(chain), None));
    }
    pub fn reset_all(&self) {
        report(self.delete(None, None));
    }
}
fn report(result: Result<(), DbError>) {
    if let Err(error) = result {
        tracing::error!(%error,"history pagination write failed; cursor remains unchanged");
    }
}

// ── Default impl

impl Default for HistoryPaginationStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_and_exhaustion_survive_reopen_and_reset() {
        let path = std::env::temp_dir().join(format!(
            "spectra-history-pagination-{}.sqlite",
            crate::store::new_event_id()
        ));
        let database = WalletDatabase::new(path.to_str().unwrap());
        let first = HistoryPaginationStore::new();
        first.bind(database.clone()).unwrap();
        first
            .advance_cursor_checked(Chain::Solana, "wallet", Some("older-signature".into()))
            .unwrap();
        first
            .set_page_checked(Chain::Ethereum, "wallet", 2, false)
            .unwrap();
        drop(first);
        drop(database);
        let second = HistoryPaginationStore::new();
        second
            .bind(WalletDatabase::new(path.to_str().unwrap()))
            .unwrap();
        assert_eq!(
            second.cursor(Chain::Solana, "wallet").as_deref(),
            Some("older-signature")
        );
        assert_eq!(second.page(Chain::Ethereum, "wallet"), 2);
        second
            .advance_cursor_checked(Chain::Solana, "wallet", None)
            .unwrap();
        second.reset_chain(Chain::Ethereum);
        drop(second);
        let third = HistoryPaginationStore::new();
        third
            .bind(WalletDatabase::new(path.to_str().unwrap()))
            .unwrap();
        assert!(third.is_exhausted(Chain::Solana, "wallet"));
        assert!(third.cursor(Chain::Solana, "wallet").is_none());
        assert_eq!(third.page(Chain::Ethereum, "wallet"), 0);
        drop(third);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cursor_chain_starts_empty() {
        let store = HistoryPaginationStore::new();
        assert!(
            store
                .cursor(crate::registry::Chain::Bitcoin, "wallet-1")
                .is_none()
        );
        assert!(!store.is_exhausted(crate::registry::Chain::Bitcoin, "wallet-1"));
        assert_eq!(store.page(crate::registry::Chain::Bitcoin, "wallet-1"), 0);
    }

    #[test]
    fn advance_cursor_tracks_state() {
        let store = HistoryPaginationStore::new();
        store
            .advance_cursor_checked(
                crate::registry::Chain::Bitcoin,
                "wallet-1",
                Some("abc123".to_string()),
            )
            .unwrap();
        assert_eq!(
            store
                .cursor(crate::registry::Chain::Bitcoin, "wallet-1")
                .as_deref(),
            Some("abc123")
        );
        assert!(!store.is_exhausted(crate::registry::Chain::Bitcoin, "wallet-1"));

        // Terminal: no next cursor → exhausted.
        store
            .advance_cursor_checked(crate::registry::Chain::Bitcoin, "wallet-1", None)
            .unwrap();
        assert!(store.is_exhausted(crate::registry::Chain::Bitcoin, "wallet-1"));
    }

    #[test]
    fn reset_clears_single_entry() {
        let store = HistoryPaginationStore::new();
        store
            .advance_cursor_checked(
                crate::registry::Chain::Bitcoin,
                "wallet-1",
                Some("tx1".to_string()),
            )
            .unwrap();
        store
            .advance_cursor_checked(
                crate::registry::Chain::Bitcoin,
                "wallet-2",
                Some("tx2".to_string()),
            )
            .unwrap();

        store.reset(crate::registry::Chain::Bitcoin, "wallet-1");

        assert!(
            store
                .cursor(crate::registry::Chain::Bitcoin, "wallet-1")
                .is_none()
        );
        assert_eq!(
            store
                .cursor(crate::registry::Chain::Bitcoin, "wallet-2")
                .as_deref(),
            Some("tx2")
        );
    }

    #[test]
    fn reset_chain_removes_all_wallets_on_chain() {
        let store = HistoryPaginationStore::new();
        store
            .advance_cursor_checked(
                crate::registry::Chain::Bitcoin,
                "wallet-1",
                Some("tx1".to_string()),
            )
            .unwrap();
        store
            .advance_cursor_checked(
                crate::registry::Chain::Bitcoin,
                "wallet-2",
                Some("tx2".to_string()),
            )
            .unwrap();
        store
            .advance_cursor_checked(
                crate::registry::Chain::Ethereum,
                "wallet-1",
                Some("eth-tx".to_string()),
            )
            .unwrap();

        store.reset_chain(crate::registry::Chain::Bitcoin);

        assert!(
            store
                .cursor(crate::registry::Chain::Bitcoin, "wallet-1")
                .is_none()
        );
        assert!(
            store
                .cursor(crate::registry::Chain::Bitcoin, "wallet-2")
                .is_none()
        );
        assert_eq!(
            store
                .cursor(crate::registry::Chain::Ethereum, "wallet-1")
                .as_deref(),
            Some("eth-tx")
        );
    }
}
