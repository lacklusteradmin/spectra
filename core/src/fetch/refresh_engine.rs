// Phase 3 — Rust-owned balance refresh loop
//
// Swift calls `start()` once; Rust drives the timer, fetches balances, and
// pushes results back through the `BalanceObserver` callback interface.
// The only job remaining for Swift is to apply the received JSON to the
// in-memory wallet model via `WalletStore.applyRustBalance(...)`.

use crate::balance_observer::{BalanceObserver, RefreshEntry};
use crate::service::WalletService;
use futures::stream::{self, StreamExt};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

// ----------------------------------------------------------------
// Internal state
// ----------------------------------------------------------------

struct Inner {
    wallet_service: Arc<WalletService>,
    observer: RwLock<Option<Arc<dyn BalanceObserver>>>,
    entries: RwLock<Vec<RefreshEntry>>,
    /// Held by the timer task; `None` means stopped.
    stop_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
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

    /// Replace the registered wallet-address entries.
    ///
    /// `entries_json` must be a JSON array of objects with fields:
    ///   `chain_id` (u32), `wallet_id` (String), `address` (String).
    ///
    /// Call this whenever wallets are added, removed, or their selected chain
    /// changes so the engine refreshes the correct set.
    pub fn set_entries(&self, entries_json: String) {
        match serde_json::from_str::<Vec<RefreshEntry>>(&entries_json) {
            Ok(entries) => *self.inner.entries.write().unwrap() = entries,
            Err(e) => eprintln!("BalanceRefreshEngine.set_entries: bad JSON — {e}"),
        }
    }

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
    pub async fn trigger_immediate(&self) {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            Self::run_cycle(&inner).await;
        });
    }
}

// ----------------------------------------------------------------
// Refresh cycle (private, not exported)
// ----------------------------------------------------------------

impl BalanceRefreshEngine {
    async fn run_cycle(inner: &Inner) {
        // Snapshot entries under a short lock hold, then release before I/O.
        let entries = inner.entries.read().unwrap().clone();
        if entries.is_empty() {
            return;
        }

        // Snapshot the observer Arc once before the loop instead of once per
        // entry — avoids N RwLock acquisitions during the hot path.
        let obs = inner.observer.read().unwrap().clone();

        // Fan out balance fetches with bounded concurrency (up to 8 in flight).
        // Clone the WalletService Arc once so each spawned future owns its handle
        // without borrowing `inner` (required for Send + 'static).
        let ws = Arc::clone(&inner.wallet_service);
        let results: Vec<Result<(u32, String, String), ()>> = stream::iter(entries)
            .map(|entry| {
                let ws = Arc::clone(&ws);
                async move {
                    match ws.fetch_balance_auto(entry.chain_id, entry.address.clone()).await {
                        Ok(json) => Ok((entry.chain_id, entry.wallet_id, json)),
                        Err(_) => Err(()),
                    }
                }
            })
            .buffer_unordered(8)
            .collect()
            .await;

        let mut refreshed: u32 = 0;
        let mut errors: u32 = 0;

        for result in results {
            match result {
                Ok((chain_id, wallet_id, json)) => {
                    if let Some(ref o) = obs {
                        o.on_balance_updated(chain_id, wallet_id, json);
                    }
                    refreshed += 1;
                }
                Err(()) => {
                    errors += 1;
                }
            }
        }

        if let Some(o) = obs {
            o.on_refresh_cycle_complete(refreshed, errors);
        }
    }
}
