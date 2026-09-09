// Rust-owned balance refresh loop. Rust drives the timer, fetches, applies
// each result to its own wallet state, and hands the observer a typed
// `WalletSummary` — the front end only mirrors it.

use crate::service::WalletService;
use futures::stream::{self, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

// ── Internal state

struct Inner {
    wallet_service: Arc<WalletService>,
    observer: RwLock<Option<Arc<dyn BalanceObserver>>>,
    entries: RwLock<Vec<RefreshEntry>>,
    stop_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// True while a refresh cycle is in flight. The timer tick path skips
    /// missed ticks via `MissedTickBehavior::Skip`, but `trigger_immediate`
    /// spawns its own task and can stack concurrent cycles when multiple
    /// Swift callers (fiat refresh, pull-to-refresh, wallet change,
    /// app-resume) fire in close succession. This flag de-dupes across
    /// both paths.
    is_cycle_running: AtomicBool,
    /// Set by `trigger_immediate` when a cycle is already in flight. The
    /// running cycle checks this flag before exiting and re-runs if set,
    /// ensuring that an entries update arriving mid-cycle is never dropped.
    pending_trigger: AtomicBool,
}

// ── BalanceRefreshEngine (UniFFI-exported object)

/// Order matters: observer and entries before `start`, and `set_entries_typed`
/// again whenever the wallet list changes. A short-lived caller wants
/// `refresh_now` instead of `start`.
#[derive(uniffi::Object)]
pub struct BalanceRefreshEngine {
    inner: Arc<Inner>,
}

#[uniffi::export(async_runtime = "tokio")]
impl BalanceRefreshEngine {
    #[uniffi::constructor]
    pub fn new(wallet_service: Arc<WalletService>) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                wallet_service,
                observer: RwLock::new(None),
                entries: RwLock::new(vec![]),
                stop_tx: Mutex::new(None),
                is_cycle_running: AtomicBool::new(false),
                pending_trigger: AtomicBool::new(false),
            }),
        })
    }

    pub fn set_observer(&self, observer: Arc<dyn BalanceObserver>) {
        *self.inner.observer.write().unwrap() = Some(observer);
    }

    pub fn clear_observer(&self) {
        *self.inner.observer.write().unwrap() = None;
    }

    /// Rebuild the entry list from the wallets core holds, and answer how many
    /// there are. `wallet_id` scopes it to one wallet.
    ///
    /// A front end used to build this list: it walked its own wallet
    /// projection, resolved each address — by reading the seed out of the
    /// Keychain and deriving it — and handed the triples back. So what the
    /// engine refreshed was one platform's copy of core's own state, and a
    /// wallet that copy could not resolve an address for was dropped from the
    /// refresh with a `print`. Core reads its wallets and its selected
    /// networks directly.
    pub async fn sync_entries(&self, wallet_id: Option<String>) -> u32 {
        let state = self.inner.wallet_service.app_state().await;
        let mut entries = refresh_entries_for(&state);
        if let Some(wallet_id) = wallet_id {
            entries.retain(|entry| entry.wallet_id.eq_ignore_ascii_case(&wallet_id));
        }
        let count = entries.len() as u32;
        *self.inner.entries.write().unwrap() = entries;
        count
    }

    /// Start the periodic refresh loop.
    ///
    /// This method is `async` to ensure it runs inside the UniFFI tokio runtime,
    /// which is required for `tokio::spawn` to work. No-op if already running.
    pub async fn start(&self, interval_secs: u64) {
        let mut stop_lock = self.inner.stop_tx.lock().unwrap();
        if stop_lock.is_some() {
            return; // already running
        }
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        *stop_lock = Some(tx);
        drop(stop_lock);

        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        Self::run_cycle(&inner).await;
                    }
                    _ = &mut rx => break,
                }
            }
        });
    }

    /// Stop the periodic refresh loop. Safe to call even if not started.
    pub fn stop(&self) {
        if let Some(tx) = self.inner.stop_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }

    /// Run one refresh cycle immediately without waiting for the next tick.
    /// Also `async` to guarantee Tokio context for `tokio::spawn`.
    ///
    /// If a cycle is already in flight, sets `pending_trigger` so the running
    /// cycle will re-run once it finishes (picks up any entry changes that
    /// arrived while the cycle was running).
    pub async fn trigger_immediate(&self) {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            inner.pending_trigger.store(true, Ordering::Release);
            Self::run_cycle(&inner).await;
        });
    }

    /// Run one sweep and wait for it to finish.
    ///
    /// `trigger_immediate` spawns and returns, which is right for a long-lived
    /// app that will receive the observer callbacks later. A process that is
    /// about to exit has nowhere to receive them: the CLI got "0 refreshed"
    /// while the fetches were still in flight. This is the same cycle, awaited.
    pub async fn refresh_now(&self) {
        self.inner.pending_trigger.store(true, Ordering::Release);
        Self::run_cycle(&self.inner).await;
    }
}

// ── Refresh cycle (private, not exported)

impl BalanceRefreshEngine {
    async fn run_cycle(inner: &Inner) {
        // Acquire the in-flight flag atomically; bail if another cycle is
        // already running. Protects against overlapping cycles from tick +
        // trigger_immediate or two back-to-back trigger_immediate calls.
        // When we bail, the `pending_trigger` flag set by `trigger_immediate`
        // ensures the running cycle will re-run after it finishes.
        if inner
            .is_cycle_running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            tracing::debug!("refresh cycle deferred: already in flight");
            return;
        }
        // Drop guard clears the flag even on panic / cancel.
        struct InFlightGuard<'a>(&'a AtomicBool);
        impl Drop for InFlightGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _in_flight = InFlightGuard(&inner.is_cycle_running);

        // Loop to consume any pending triggers that arrived while we were
        // running. This ensures that entry updates (new wallets imported
        // mid-cycle) are picked up without waiting for the periodic timer.
        loop {
            // Clear the flag before snapshotting entries so that a trigger
            // arriving after the snapshot but before this clear is not lost
            // — it will set the flag again and we loop.
            inner.pending_trigger.store(false, Ordering::Release);

            // Snapshot entries under a short lock hold, then release before I/O.
            let entries = inner.entries.read().unwrap().clone();
            let entry_count = entries.len();
            if entries.is_empty() {
                break;
            }

            let cycle_start = Instant::now();
            tracing::debug!(entries = entry_count, "refresh cycle start");

            // Snapshot the observer Arc once before the loop instead of once per
            // entry — avoids N RwLock acquisitions during the hot path.
            let obs = inner.observer.read().unwrap().clone();

            // Fan out balance fetches with bounded concurrency (up to 8 in flight).
            // Fetch the native balance, then build a minimal WalletSummary (one
            // holding) from the coin template and the fetched amount. The observer
            // carries it to the front end, which merges it into the holding rather
            // than replacing the wallet: this summary knows only the native asset.
            let ws = Arc::clone(&inner.wallet_service);
            let results: Vec<Result<(String, String, WalletSummary), ()>> = stream::iter(entries)
                .map(|entry| {
                    let ws = Arc::clone(&ws);
                    async move {
                        let fetched = ws
                            .fetch_native_balance_summary_auto(
                                &entry.chain_id,
                                entry.address.clone(),
                            )
                            .await
                            .map_err(|_| ())?;
                        let template =
                            crate::service::native_coin_template(&entry.chain_id).ok_or(())?;
                        let amount = fetched.amount_display.parse::<f64>().unwrap_or(0.0);
                        let holding = AssetHolding { amount, ..template };
                        let wallet_summary = WalletSummary {
                            id: entry.wallet_id.clone(),
                            name: String::new(),
                            is_watch_only: false,
                            chain_name: holding.chain_name.clone(),
                            include_in_portfolio_total: true,
                            network_mode: None,
                            derivation_overrides: Default::default(),
                            xpub: None,
                            derivation_preset: String::new(),
                            derivation_path: None,
                            holdings: vec![holding],
                            addresses: vec![],
                        };
                        Ok((entry.chain_id.clone(), entry.wallet_id, wallet_summary))
                    }
                })
                .buffer_unordered(8)
                .collect()
                .await;

            let mut refreshed: u32 = 0;
            let mut errors: u32 = 0;

            for result in results {
                match result {
                    Ok((chain_id, wallet_id, summary)) => {
                        if let Some(ref o) = obs {
                            o.on_balance_updated(chain_id, wallet_id, Some(summary));
                        }
                        refreshed += 1;
                    }
                    Err(()) => {
                        errors += 1;
                    }
                }
            }

            if let Some(o) = obs.as_ref() {
                o.on_refresh_cycle_complete(refreshed, errors);
            }

            let elapsed_ms = cycle_start.elapsed().as_millis();
            tracing::debug!(refreshed, errors, elapsed_ms, "refresh cycle end");

            // Re-run if a trigger arrived while this cycle was executing.
            if !inner.pending_trigger.load(Ordering::Acquire) {
                break;
            }
        }
    }
}

// ── Tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the in-flight gate pattern used in `run_cycle`.
    ///
    /// Spins up two concurrent workers that replicate the exact
    /// `compare_exchange` + drop-guard pattern, each sleeping 50ms while it
    /// "owns" the gate. If the pattern is broken (guard removed, swapped to
    /// `store` instead of `compare_exchange`, etc.) both workers will enter
    /// the critical section and the invocation count will be 2. With the
    /// pattern intact only one worker enters; the other sees the gate held
    /// and bails.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn in_flight_gate_serialises_concurrent_workers() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let gate = Arc::new(AtomicBool::new(false));
        let work_count = Arc::new(AtomicU32::new(0));
        let skip_count = Arc::new(AtomicU32::new(0));

        async fn guarded_work(
            gate: Arc<AtomicBool>,
            work_count: Arc<AtomicU32>,
            skip_count: Arc<AtomicU32>,
        ) {
            if gate
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                skip_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
            struct Guard<'a>(&'a AtomicBool);
            impl Drop for Guard<'_> {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _g = Guard(&gate);
            work_count.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let a = tokio::spawn(guarded_work(
            Arc::clone(&gate),
            Arc::clone(&work_count),
            Arc::clone(&skip_count),
        ));
        let b = tokio::spawn(guarded_work(
            Arc::clone(&gate),
            Arc::clone(&work_count),
            Arc::clone(&skip_count),
        ));
        let _ = tokio::join!(a, b);

        assert_eq!(
            work_count.load(Ordering::Relaxed),
            1,
            "exactly one worker should enter"
        );
        assert_eq!(
            skip_count.load(Ordering::Relaxed),
            1,
            "the other worker should skip"
        );
        assert!(
            !gate.load(Ordering::Relaxed),
            "gate should be released after work finishes"
        );
    }

    /// After a worker finishes, a subsequent worker should see the gate
    /// clear and run normally. Catches a regression where the drop guard
    /// fails to release the flag.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn in_flight_gate_releases_after_completion() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let gate = Arc::new(AtomicBool::new(false));
        let work_count = Arc::new(AtomicU32::new(0));

        for _ in 0..3 {
            if gate
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                panic!("gate should be clear between sequential runs");
            }
            struct Guard<'a>(&'a AtomicBool);
            impl Drop for Guard<'_> {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _g = Guard(&gate);
            work_count.fetch_add(1, Ordering::Relaxed);
        }

        assert_eq!(work_count.load(Ordering::Relaxed), 3);
        assert!(!gate.load(Ordering::Relaxed));
    }
}

use crate::store::state::WalletSummary;
use crate::store::wallet_domain::AssetHolding;

/// Callback interface implemented by Swift. Rust calls these from the tokio
/// task that owns the refresh timer loop. Implementations must be
/// `Send + Sync` (UniFFI enforces this for foreign trait objects).
///
/// The refresh engine applies the balance update to the Rust-owned wallet
/// state before invoking the callback, so Swift receives a typed
/// `WalletSummary` record directly — no JSON shuttle.
#[uniffi::export(with_foreign)]
pub trait BalanceObserver: Send + Sync {
    /// Called after each successful balance fetch within a cycle. `summary`
    /// is the updated `WalletSummary` (already applied to the Rust store), or
    /// `None` if the native amount could not be parsed or the wallet is not
    /// in the in-memory state.
    fn on_balance_updated(
        &self,
        chain_id: String,
        wallet_id: String,
        summary: Option<WalletSummary>,
    );

    /// Called once the full sweep of all registered entries completes.
    fn on_refresh_cycle_complete(&self, refreshed: u32, errors: u32);
}

/// What to refresh for the wallets in `state`, one entry per wallet that has an
/// address to fetch.
///
/// A wallet with no address is not an error and not a log line — it is a
/// watch-only import that stored nothing, or a wallet on a chain the registry
/// does not know, and either way there is nothing to fetch.
pub(crate) fn refresh_entries_for(state: &crate::store::state::CoreAppState) -> Vec<RefreshEntry> {
    use crate::registry::Chain;
    state
        .wallets
        .iter()
        .filter_map(|wallet| {
            let chain = Chain::from_display_name(&wallet.chain_name)?;
            // A Bitcoin account xpub covers every address the wallet derives,
            // so it is the fetch key when the wallet has one. Otherwise it is
            // the address for the network the wallet is on.
            let address = wallet
                .xpub
                .as_deref()
                .map(str::trim)
                .filter(|xpub| chain == Chain::Bitcoin && !xpub.is_empty())
                .or_else(|| wallet.active_address(&state.settings))?;
            Some(RefreshEntry {
                // The chain the balance is fetched and filed under. Not the
                // selected network: the holding it produces is merged by chain
                // name, so filing a testnet balance under the testnet's name
                // would create a second holding rather than update the one the
                // wallet shows. See "Known open items".
                chain_id: chain.str_id().to_string(),
                wallet_id: wallet.id.clone(),
                address: address.to_string(),
            })
        })
        .collect()
}

/// One (chain, wallet, address) triple registered for periodic refresh.
///
/// For Bitcoin HD wallets: set `address` to the xpub/ypub/zpub.
/// `WalletService::fetch_native_balance_summary_auto` detects extended keys
/// automatically.
#[derive(Debug, Clone, serde::Deserialize, uniffi::Record)]
pub struct RefreshEntry {
    pub chain_id: String,
    pub wallet_id: String,
    /// The canonical fetch key: a wallet address for most chains, or an
    /// xpub/ypub/zpub for Bitcoin HD wallets.
    pub address: String,
}


#[cfg(test)]
mod refresh_entry_tests {
    use super::refresh_entries_for;
    use crate::registry::Chain;
    use crate::store::state::{CoreAppState, WalletAddress, WalletSummary};

    fn wallet(id: &str, chain: Chain, addresses: &[(Chain, &str)]) -> WalletSummary {
        WalletSummary {
            id: id.to_string(),
            name: id.to_string(),
            is_watch_only: false,
            chain_name: chain.chain_display_name().to_string(),
            include_in_portfolio_total: true,
            network_mode: None,
            xpub: None,
            derivation_preset: "standard".to_string(),
            derivation_path: None,
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: addresses
                .iter()
                .map(|(chain, address)| WalletAddress {
                    chain_name: chain.chain_display_name().to_string(),
                    address: (*address).to_string(),
                    kind: "receive".to_string(),
                    derivation_path: None,
                })
                .collect(),
        }
    }

    /// One entry per wallet that has an address, and the address is the one for
    /// the network that wallet is on.
    #[test]
    fn an_entry_carries_the_address_for_the_network_the_wallet_is_on() {
        let mut state = CoreAppState::default();
        state.wallets = vec![wallet(
            "w1",
            Chain::Bitcoin,
            &[(Chain::Bitcoin, "bc1main"), (Chain::BitcoinTestnet4, "tb1test")],
        )];

        let mainnet = refresh_entries_for(&state);
        assert_eq!(mainnet.len(), 1);
        assert_eq!(mainnet[0].address, "bc1main");

        // The app's selection moves the whole family.
        state.settings.network_chain_by_family.insert(
            Chain::Bitcoin.str_id().to_string(),
            Chain::BitcoinTestnet4.str_id().to_string(),
        );
        assert_eq!(refresh_entries_for(&state)[0].address, "tb1test");

        // A wallet's own network wins over the app's selection.
        state.wallets[0].network_mode = Some(Chain::Bitcoin.str_id().to_string());
        assert_eq!(refresh_entries_for(&state)[0].address, "bc1main");
    }

    /// A Bitcoin account xpub covers every address the wallet derives, so it is
    /// the fetch key. Only Bitcoin has one.
    #[test]
    fn a_bitcoin_xpub_is_the_fetch_key() {
        let mut state = CoreAppState::default();
        let mut btc = wallet("w1", Chain::Bitcoin, &[(Chain::Bitcoin, "bc1main")]);
        btc.xpub = Some("zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1ADqtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs".to_string());
        state.wallets = vec![btc];
        assert!(refresh_entries_for(&state)[0].address.starts_with("zpub"));

        // An empty one is not a key.
        state.wallets[0].xpub = Some("   ".to_string());
        assert_eq!(refresh_entries_for(&state)[0].address, "bc1main");
    }

    /// A wallet with no address is left out rather than refreshed with nothing.
    #[test]
    fn a_wallet_with_no_address_is_not_an_entry() {
        let mut state = CoreAppState::default();
        state.wallets = vec![
            wallet("w1", Chain::Solana, &[]),
            wallet("w2", Chain::Solana, &[(Chain::Solana, "So1")]),
        ];
        let entries = refresh_entries_for(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].wallet_id, "w2");
        assert_eq!(entries[0].chain_id, Chain::Solana.str_id());
    }

    /// The EVM family shares one address, so an Ethereum wallet's entry is its
    /// own — and an Arbitrum wallet reads the same slot.
    #[test]
    fn the_evm_family_shares_one_address() {
        let mut state = CoreAppState::default();
        state.wallets = vec![wallet("w1", Chain::Arbitrum, &[(Chain::Ethereum, "0xabc")])];
        let entries = refresh_entries_for(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].address, "0xabc");
        assert_eq!(entries[0].chain_id, Chain::Arbitrum.str_id());
    }
}
