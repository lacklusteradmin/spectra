//! History pagination cursor surface for `WalletService`.
//!
//! These methods are thin, synchronous delegations to `self.history_pagination`
//! (cursor/page/exhaustion bookkeeping for the per-(chain, wallet) history
//! feed) — bookkeeping, not a chain read, which is why they are not in
//! [`super::network`].

use super::*;

/// Where the next history page for one (chain, wallet) starts.
///
/// Three getters used to answer this one field at a time, so a caller that
/// wanted the whole position made three calls and could see it change between
/// them.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HistoryCursor {
    /// Cursor chains (the UTXO family). `None` before the first fetch.
    pub next_cursor: Option<String>,
    /// Page-numbered chains (EVM). Zero before the first fetch.
    pub next_page: u32,
    /// Every page has been fetched. No further request until a reset.
    pub is_exhausted: bool,
}

/// How much history pagination to forget.
///
/// Four methods stood for these four cases — `reset_history`,
/// `reset_history_for_wallet`, `reset_history_for_chain`, `reset_all_history`
/// — which is one question with the answer in the method name.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum HistoryScope {
    /// One wallet's feed on one chain: pull-to-refresh, or a send confirming.
    ChainAndWallet { chain_id: String, wallet_id: String },
    /// Every chain for one wallet: the wallet was deleted, or fully refreshed.
    Wallet { wallet_id: String },
    /// Every wallet on one chain: a re-org, or an endpoint switch.
    Chain { chain_id: String },
    /// Everything: account wipe.
    All,
}

#[uniffi::export]
impl WalletService {
    /// Where the next history fetch for this (chain, wallet) starts.
    pub fn history_cursor(&self, chain_id: String, wallet_id: String) -> HistoryCursor {
        HistoryCursor {
            next_cursor: self.history_pagination.cursor(&chain_id, &wallet_id),
            next_page: self.history_pagination.page(&chain_id, &wallet_id),
            is_exhausted: self.history_pagination.is_exhausted(&chain_id, &wallet_id),
        }
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

    /// Forget history pagination, for as much of it as `scope` names.
    pub fn reset_history(&self, scope: HistoryScope) {
        match scope {
            HistoryScope::ChainAndWallet {
                chain_id,
                wallet_id,
            } => self.history_pagination.reset(&chain_id, &wallet_id),
            HistoryScope::Wallet { wallet_id } => {
                self.history_pagination.reset_all_for_wallet(&wallet_id)
            }
            HistoryScope::Chain { chain_id } => self.history_pagination.reset_chain(&chain_id),
            HistoryScope::All => self.history_pagination.reset_all(),
        }
    }
}
