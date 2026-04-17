// Phase 3 — Rust-driven balance refresh
//
// Swift implements `BalanceObserver`; Rust calls it whenever a balance fetch
// completes. The `RefreshEntry` type is the unit of registration.

/// Callback interface implemented by Swift. Rust calls these from the tokio
/// task that owns the refresh timer loop. Implementations must be
/// `Send + Sync` (UniFFI enforces this for foreign trait objects).
#[uniffi::export(with_foreign)]
pub trait BalanceObserver: Send + Sync {
    /// Called after each successful balance fetch within a cycle.
    /// `balance_json` is the raw JSON from `WalletService::fetch_balance_auto`.
    fn on_balance_updated(&self, chain_id: u32, wallet_id: String, balance_json: String);

    /// Called once the full sweep of all registered entries completes.
    fn on_refresh_cycle_complete(&self, refreshed: u32, errors: u32);
}

/// One (chain, wallet, address) triple registered for periodic refresh.
///
/// For Bitcoin HD wallets: set `address` to the xpub/ypub/zpub.
/// `WalletService::fetch_balance_auto` detects extended keys automatically.
#[derive(Debug, Clone, serde::Deserialize, uniffi::Record)]
pub struct RefreshEntry {
    pub chain_id: u32,
    pub wallet_id: String,
    /// The canonical fetch key: a wallet address for most chains, or an
    /// xpub/ypub/zpub for Bitcoin HD wallets.
    pub address: String,
}
