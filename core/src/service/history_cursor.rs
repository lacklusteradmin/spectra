//! History pagination cursor surface for `WalletService`.
//!
//! These methods are thin, synchronous delegations to `self.history_pagination`
//! (cursor/page/exhaustion bookkeeping for the per-(chain, wallet) history
//! feed) — bookkeeping, not a chain read, which is why they are not in
//! [`super::network`].

use super::*;

/// Where the next history page for one (chain, wallet) starts.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryCursor {
    /// Cursor chains (the UTXO family). `None` before the first fetch.
    pub next_cursor: Option<String>,
    /// Page-numbered chains (EVM). Zero before the first fetch.
    pub next_page: u32,
    /// Every page has been fetched. No further request until a reset.
    pub is_exhausted: bool,
}

/// How much history pagination to forget.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryScope {
    /// One wallet's feed on one chain: pull-to-refresh, or a send confirming.
    ChainAndWallet {
        chain_id: crate::registry::Chain,
        wallet_id: String,
    },
    /// Everything: account wipe.
    All,
}

impl WalletService {
    /// Where the next history fetch for this (chain, wallet) starts.
    pub fn history_cursor(
        &self,
        chain_id: crate::registry::Chain,
        wallet_id: String,
    ) -> HistoryCursor {
        HistoryCursor {
            next_cursor: self.history_pagination.cursor(chain_id, &wallet_id),
            next_page: self.history_pagination.page(chain_id, &wallet_id),
            is_exhausted: self.history_pagination.is_exhausted(chain_id, &wallet_id),
        }
    }

    /// Wallets whose family's history feed still has pages to fetch, for the
    /// transaction snapshot. A front end asks nothing per wallet.
    pub(crate) async fn wallets_with_more_history_now(&self) -> Vec<String> {
        self.wallet_state
            .read()
            .await
            .wallets
            .iter()
            .filter(|wallet| {
                !self
                    .history_pagination
                    .is_exhausted(wallet.family(), &wallet.id)
            })
            .map(|wallet| wallet.id.clone())
            .collect()
    }
}

impl WalletService {
    /// Forget history pagination, for as much of it as `scope` names.
    ///
    /// Internal: the commands that invalidate a feed — removing a wallet,
    /// changing Bitcoin's Esplora source — reset it where they commit.
    pub fn reset_history(&self, scope: HistoryScope) {
        match scope {
            HistoryScope::ChainAndWallet {
                chain_id,
                wallet_id,
            } => self.history_pagination.reset(chain_id, &wallet_id),
            HistoryScope::All => self.history_pagination.reset_all(),
        }
    }
}

// Core-internal cursor writes. Not exported: `history_bitcoin` and
// `history_refresh` call these, no front end does.
impl WalletService {
    /// Record the cursor returned after a successful cursor-based fetch (UTXO
    /// chains). Pass `None` when the chain confirms there are no more pages —
    /// this marks the chain as exhausted.
    pub fn advance_history_cursor(
        &self,
        chain_id: crate::registry::Chain,
        wallet_id: String,
        next_cursor: Option<String>,
    ) {
        self.history_pagination
            .advance_cursor(chain_id, &wallet_id, next_cursor);
    }

    /// Record the page just fetched, and whether it was the last one.
    ///
    /// For the page-based family (EVM), where a page number is absolute rather
    /// than a cursor. One write, so a reader never sees half an update.
    pub fn set_history_page(
        &self,
        chain_id: crate::registry::Chain,
        wallet_id: String,
        page: u32,
        is_exhausted: bool,
    ) {
        self.history_pagination.set_page(chain_id, &wallet_id, page);
        self.history_pagination
            .set_exhausted(chain_id, &wallet_id, is_exhausted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pagination_updates_and_resets_do_not_cross_wallet_or_chain_boundaries() {
        let service = WalletService::new(vec![]).unwrap();
        service.advance_history_cursor(
            crate::registry::Chain::Bitcoin,
            "a".into(),
            Some("next".into()),
        );
        service.set_history_page(crate::registry::Chain::Ethereum, "a".into(), 4, true);
        service.set_history_page(crate::registry::Chain::Ethereum, "b".into(), 2, false);
        assert_eq!(
            service
                .history_cursor(crate::registry::Chain::Bitcoin, "a".into())
                .next_cursor
                .as_deref(),
            Some("next")
        );
        assert!(
            service
                .history_cursor(crate::registry::Chain::Ethereum, "a".into())
                .is_exhausted
        );
        service.history_pagination.reset_all_for_wallet("a");
        assert_eq!(
            service
                .history_cursor(crate::registry::Chain::Ethereum, "a".into())
                .next_page,
            0
        );
        assert_eq!(
            service
                .history_cursor(crate::registry::Chain::Ethereum, "b".into())
                .next_page,
            2
        );
        assert!(
            service
                .history_cursor(crate::registry::Chain::Bitcoin, "a".into())
                .next_cursor
                .is_none()
        );
    }
}
