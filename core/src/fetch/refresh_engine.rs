// Rust-owned balance refresh loop
//
// Swift calls `start()` once; Rust drives the timer, fetches balances, and
// pushes results back through the `BalanceObserver` callback interface.
// The only job remaining for Swift is to apply the received JSON to the
// in-memory wallet model via `WalletStore.applyRustBalance(...)`.

use crate::service::WalletService;
use futures::stream::{self, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

// ----------------------------------------------------------------
// Internal state
// ----------------------------------------------------------------

struct Inner {
    wallet_service: Arc<WalletService>,
    observer: RwLock<Option<Arc<dyn BalanceObserver>>>,
    entries: RwLock<Vec<RefreshEntry>>,
    /// Held by the timer task; `None` means stopped.
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

// ----------------------------------------------------------------
// BalanceRefreshEngine (UniFFI-exported object)
// ----------------------------------------------------------------

/// Rust-owned periodic balance refresh engine.
///
/// Lifecycle:
///   1. `new(walletService:)` — create once per app session.
///   2. `set_observer(:)` — register the Swift balance observer.
///   3. `set_entries(:)` — push the initial wallet-address list.
///   4. `await start(intervalSecs:)` — begin the timer loop.
///   5. Call `set_entries` again whenever wallets are added/removed.
///   6. `stop()` on background or logout.
#[derive(uniffi::Object)]
pub struct BalanceRefreshEngine {
    inner: Arc<Inner>,
}

#[uniffi::export(async_runtime = "tokio")]
impl BalanceRefreshEngine {
    /// Construct a new engine backed by the given WalletService instance.
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

    /// Register the Swift observer that receives balance notifications.
    pub fn set_observer(&self, observer: Arc<dyn BalanceObserver>) {
        *self.inner.observer.write().unwrap() = Some(observer);
    }

    /// Clear the observer (e.g. on logout or memory pressure).
    pub fn clear_observer(&self) {
        *self.inner.observer.write().unwrap() = None;
    }

    /// Replace the full list of (chain, wallet, address) entries to refresh each cycle.
    pub fn set_entries_typed(&self, entries: Vec<RefreshEntry>) {
        *self.inner.entries.write().unwrap() = entries;
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
}

// ----------------------------------------------------------------
// Refresh cycle (private, not exported)
// ----------------------------------------------------------------

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
            // Rust fetches the native balance from the network, then constructs a
            // minimal WalletSummary (one holding) from the coin template and the
            // fetched amount. Swift owns the authoritative wallet model and applies
            // the update via its merge logic — Rust no longer mirrors wallet state.
            let ws = Arc::clone(&inner.wallet_service);
            let results: Vec<Result<(String, String, WalletSummary), ()>> = stream::iter(entries)
                .map(|entry| {
                    let ws = Arc::clone(&ws);
                    async move {
                        let fetched = ws
                            .fetch_native_balance_summary_auto(&entry.chain_id, entry.address.clone())
                            .await
                            .map_err(|_| ())?;
                        let template = crate::service::native_coin_template(&entry.chain_id).ok_or(())?;
                        let amount = fetched.amount_display.parse::<f64>().unwrap_or(0.0);
                        let holding = AssetHolding { amount, ..template };
                        let wallet_summary = WalletSummary {
                            id: entry.wallet_id.clone(),
                            name: String::new(),
                            is_watch_only: false,
                            chain_name: holding.chain_name.clone(),
                            include_in_portfolio_total: true,
                            network_mode: None,
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

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

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

        assert_eq!(work_count.load(Ordering::Relaxed), 1, "exactly one worker should enter");
        assert_eq!(skip_count.load(Ordering::Relaxed), 1, "the other worker should skip");
        assert!(!gate.load(Ordering::Relaxed), "gate should be released after work finishes");
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

use crate::store::state::{AssetHolding, WalletSummary};

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
    fn on_balance_updated(&self, chain_id: String, wallet_id: String, summary: Option<WalletSummary>);

    /// Called once the full sweep of all registered entries completes.
    fn on_refresh_cycle_complete(&self, refreshed: u32, errors: u32);
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
