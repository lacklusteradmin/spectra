// Rust-owned refresh loops. Rust drives both timers — the balance sweep and
// the maintenance tick — fetches, applies each result to its own state, and
// tells the observer what changed. The front end supplies device conditions
// and renders; it runs no loop of its own.

use crate::fetch::refresh_policy::DeviceConditions;
use crate::service::WalletService;
use crate::service::app_refresh::{AppRefreshIntent, AppRefreshResult};
use futures::stream::{self, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

// ── Internal state

struct Inner {
    wallet_service: Arc<WalletService>,
    observer: RwLock<Option<Arc<dyn RefreshObserver>>>,
    entries: RwLock<Vec<RefreshEntry>>,
    /// What the platform last said about the device. `None` until it says
    /// anything, which is how a one-shot caller like the CLI runs no loops.
    conditions: RwLock<Option<DeviceConditions>>,
    stop_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    maintenance_stop_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Whether the task forwarding Tor status to the observer is running.
    forwards_tor_status: AtomicBool,
    /// True while a refresh cycle is in flight. The timer tick path skips
    /// missed ticks via `MissedTickBehavior::Skip`, but `trigger_immediate`
    /// spawns its own task and can stack concurrent cycles when several
    /// callers fire in close succession. This flag de-dupes across both paths.
    is_cycle_running: AtomicBool,
    /// Set by `trigger_immediate` when a cycle is already in flight. The
    /// running cycle checks this flag before exiting and re-runs if set,
    /// ensuring that an entries update arriving mid-cycle is never dropped.
    pending_trigger: AtomicBool,
}

/// How long the maintenance loop waits after a refresh that failed outright.
const MAINTENANCE_RETRY_SECONDS: u64 = 60;

// ── RefreshEngine (UniFFI-exported object)

/// Attach an observer, then report device conditions: while the app is active
/// and a wallet has something to fetch, the engine sweeps balances and runs
/// maintenance ticks at the cadence core plans. A short-lived caller uses
/// `refresh_now` and never reports conditions.
#[derive(uniffi::Object)]
pub struct RefreshEngine {
    inner: Arc<Inner>,
}

#[uniffi::export(async_runtime = "tokio")]
impl RefreshEngine {
    #[uniffi::constructor]
    pub fn new(wallet_service: Arc<WalletService>) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                wallet_service,
                observer: RwLock::new(None),
                entries: RwLock::new(vec![]),
                conditions: RwLock::new(None),
                stop_tx: Mutex::new(None),
                maintenance_stop_tx: Mutex::new(None),
                forwards_tor_status: AtomicBool::new(false),
                is_cycle_running: AtomicBool::new(false),
                pending_trigger: AtomicBool::new(false),
            }),
        })
    }

    pub fn set_observer(&self, observer: Arc<dyn RefreshObserver>) {
        *self.inner.observer.write().unwrap() = Some(observer);
    }

    pub fn clear_observer(&self) {
        *self.inner.observer.write().unwrap() = None;
    }

    /// Rebuild refresh entries from core-owned wallets and their selected
    /// networks. `wallet_id` scopes the rebuild; returns the entry count.
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

    /// Adopt core's wallets and refresh only when fetch inputs change.
    /// Returns whether they changed. Balance-only updates must not retrigger
    /// a sweep, since sweeps themselves update wallet balances.
    pub async fn reconcile_wallets(&self) -> bool {
        let state = self.inner.wallet_service.app_state().await;
        let entries = refresh_entries_for(&state);
        if !self.replace_entries(entries) {
            return false;
        }
        if !self.has_entries() {
            self.stop();
        } else if self.app_is_active() {
            if self.is_running() {
                self.trigger_immediate().await;
            } else {
                self.start(super::refresh_policy::AUTOMATIC_REFRESH_SECONDS)
                    .await;
            }
        }
        self.reconcile_maintenance();
        true
    }

    /// What only the device knows. Coming to the foreground restarts the
    /// balance sweep, whose first tick runs at once; leaving it stops both
    /// loops. Other changes only reach the next maintenance tick.
    pub async fn set_device_conditions(&self, conditions: DeviceConditions) {
        self.forward_tor_status();
        let was_active = self.app_is_active();
        let is_active = conditions.app_is_active;
        *self.inner.conditions.write().unwrap() = Some(conditions);
        if !is_active {
            self.stop();
        } else if !was_active {
            self.stop();
            if self.sync_entries(None).await > 0 {
                self.start(super::refresh_policy::AUTOMATIC_REFRESH_SECONDS)
                    .await;
            }
        }
        self.reconcile_maintenance();
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

impl RefreshEngine {
    /// Store `entries` if they differ from the current list. Answers whether
    /// they did.
    fn replace_entries(&self, entries: Vec<RefreshEntry>) -> bool {
        let mut current = self.inner.entries.write().unwrap();
        if *current == entries {
            return false;
        }
        *current = entries;
        true
    }

    fn has_entries(&self) -> bool {
        !self.inner.entries.read().unwrap().is_empty()
    }

    fn app_is_active(&self) -> bool {
        self.inner
            .conditions
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|c| c.app_is_active)
    }

    fn is_running(&self) -> bool {
        self.inner.stop_tx.lock().unwrap().is_some()
    }

    /// Stop the periodic refresh loop. Safe to call even if not started.
    ///
    /// Internal: `reconcile_wallets` stops the engine when no wallet is left to
    /// fetch, and `set_device_conditions` when the app leaves the foreground.
    fn stop(&self) {
        if let Some(tx) = self.inner.stop_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }

    /// Hand the observer the Tor status now and on every change, for as long
    /// as the engine lives. Called from an async export, so a runtime is
    /// present for the spawn.
    fn forward_tor_status(&self) {
        if self.inner.forwards_tor_status.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut changes = crate::tor::subscribe_status();
            loop {
                let status = changes.borrow_and_update().clone();
                let Some(inner) = inner.upgrade() else { return };
                if let Some(observer) = inner.observer.read().unwrap().clone() {
                    observer.on_tor_status_changed(status);
                }
                drop(inner);
                if changes.changed().await.is_err() {
                    return;
                }
            }
        });
    }

    /// Run the maintenance loop exactly while the app is active and a wallet
    /// has something to fetch. Called from async exports, so a runtime is
    /// present for the spawn.
    fn reconcile_maintenance(&self) {
        let mut stop_lock = self.inner.maintenance_stop_tx.lock().unwrap();
        let wanted = self.app_is_active() && self.has_entries();
        if !wanted {
            if let Some(tx) = stop_lock.take() {
                let _ = tx.send(());
            }
            return;
        }
        if stop_lock.is_some() {
            return;
        }
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        *stop_lock = Some(tx);
        drop(stop_lock);
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            loop {
                let seconds =
                    match Self::refresh_and_notify(&inner, AppRefreshIntent::Scheduled).await {
                        Some(result) => result,
                        None => MAINTENANCE_RETRY_SECONDS,
                    };
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(seconds)) => {}
                    _ = &mut rx => break,
                }
            }
        });
    }

    /// One app refresh under the current conditions, handed to the observer.
    /// Answers the seconds core plans until the next tick, or `None` when there
    /// were no conditions or the refresh failed outright.
    async fn refresh_and_notify(inner: &Inner, intent: AppRefreshIntent) -> Option<u64> {
        let conditions = inner.conditions.read().unwrap().clone()?;
        let result = inner
            .wallet_service
            .refresh_app(intent, conditions)
            .await
            .ok()?;
        let seconds = result.poll_seconds;
        if let Some(observer) = inner.observer.read().unwrap().clone() {
            observer.on_refresh_complete(result);
        }
        Some(seconds)
    }

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

            // The service resolves, merges and commits each wallet before notification.
            let ws = Arc::clone(&inner.wallet_service);
            let results: Vec<Result<(String, String, WalletState), ()>> = stream::iter(entries)
                .map(|entry| {
                    let ws = Arc::clone(&ws);
                    async move {
                        let wallet_summary = ws
                            .refresh_wallet_balances(entry.wallet_id.clone())
                            .await
                            .map_err(|_| ())?;
                        Ok((
                            entry.holding_chain_id.clone(),
                            entry.wallet_id,
                            wallet_summary,
                        ))
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
            // New balances can cross an alert or a movement threshold. An app
            // that reported conditions gets that judgement with the sweep.
            Self::refresh_and_notify(inner, AppRefreshIntent::BalancesUpdated).await;

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

use crate::store::state::WalletState;

/// Callback interface implemented by Swift. Rust calls these from the tokio
/// task that owns the refresh timer loop. Implementations must be
/// `Send + Sync` (UniFFI enforces this for foreign trait objects).
///
/// The refresh engine applies the balance update to the Rust-owned wallet
/// state before invoking the callback, so Swift receives a typed
/// `WalletState` record directly — no JSON shuttle.
#[uniffi::export(with_foreign)]
pub trait RefreshObserver: Send + Sync {
    /// Called after each successful balance fetch within a cycle. `summary`
    /// is the updated `WalletState` (already applied to the Rust store), or
    /// `None` if the native amount could not be parsed or the wallet is not
    /// in the in-memory state.
    fn on_balance_updated(&self, chain_id: String, wallet_id: String, summary: Option<WalletState>);

    /// Called once the full sweep of all registered entries completes.
    fn on_refresh_cycle_complete(&self, refreshed: u32, errors: u32);

    /// Called after an app refresh the engine ran itself — a maintenance tick,
    /// or the judgement after a sweep — with what to notify the user about.
    fn on_refresh_complete(&self, result: AppRefreshResult);

    /// The embedded Tor client's status, sent once when the platform first
    /// reports conditions and then on every change.
    fn on_tor_status_changed(&self, status: crate::tor::TorStatus);
}

/// What to refresh for the wallets in `state`, one entry per wallet that has an
/// address to fetch.
///
/// A wallet with no address is not an error and not a log line — it is a
/// watch-only import that stored nothing, or a wallet on a chain the registry
/// does not know, and either way there is nothing to fetch.
pub(crate) fn refresh_entries_for(state: &crate::store::state::CoreAppState) -> Vec<RefreshEntry> {
    state.wallets.iter().filter_map(refresh_entry_for).collect()
}

pub(crate) fn refresh_entry_for(wallet: &crate::store::state::WalletState) -> Option<RefreshEntry> {
    use crate::registry::Chain;
    let chain = wallet.family()?;
    let address = wallet
        .xpub
        .as_deref()
        .map(str::trim)
        .filter(|xpub| chain == Chain::Bitcoin && !xpub.is_empty())
        .or_else(|| wallet.active_address())?;
    Some(RefreshEntry {
        holding_chain_id: chain.str_id().to_string(),
        chain_id: wallet.chain().unwrap_or(chain).str_id().to_string(),
        wallet_id: wallet.id.clone(),
        address: address.to_string(),
    })
}

/// One (chain, wallet, address) triple registered for periodic refresh.
///
/// For Bitcoin HD wallets: set `address` to the xpub/ypub/zpub.
/// `WalletService::fetch_native_balance_summary_auto` detects extended keys
/// automatically.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Deserialize, uniffi::Record)]
pub struct RefreshEntry {
    /// The chain the balance is *filed* under: the wallet's family, which is
    /// what its holding is named after and what pricing keys on.
    pub holding_chain_id: String,
    /// Network to fetch the balance from. Keep it distinct from the holding
    /// identity so testnet fetches use testnet endpoints without renaming assets.
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
    use crate::store::state::{CoreAppState, WalletAddress, WalletState};

    fn wallet(id: &str, chain: Chain, addresses: &[(Chain, &str)]) -> WalletState {
        WalletState {
            id: id.to_string(),
            name: id.to_string(),
            signing: crate::store::state::WalletSigning::SeedPhrase {
                password_protected: false,
            },
            include_in_portfolio_total: true,
            chain_id: chain.str_id().into(),
            xpub: None,
            derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
            derivation_path: None,
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: addresses
                .iter()
                .map(|(chain, address)| WalletAddress {
                    chain_id: chain.str_id().to_string(),
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
        let mut state = CoreAppState {
            wallets: vec![wallet(
                "w1",
                Chain::Bitcoin,
                &[
                    (Chain::Bitcoin, "bc1main"),
                    (Chain::BitcoinTestnet4, "tb1test"),
                ],
            )],
            ..Default::default()
        };

        let mainnet = refresh_entries_for(&state);
        assert_eq!(mainnet.len(), 1);
        assert_eq!(mainnet[0].address, "bc1main");

        // The app's selection moves the whole family.
        state.settings.selected_chain_by_family.insert(
            Chain::Bitcoin.str_id().to_string(),
            Chain::BitcoinTestnet4.str_id().to_string(),
        );
        assert_eq!(
            refresh_entries_for(&state)[0].address,
            "bc1main",
            "settings cannot retarget a stored wallet"
        );
        state.wallets[0].chain_id = "bitcoin-testnet-4".into();
        assert_eq!(refresh_entries_for(&state)[0].address, "tb1test");

        // A wallet's own network wins over the app's selection.
        state.wallets[0].chain_id = Chain::Bitcoin.str_id().to_string();
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
        let state = CoreAppState {
            wallets: vec![
                wallet("w1", Chain::Solana, &[]),
                wallet("w2", Chain::Solana, &[(Chain::Solana, "So1")]),
            ],
            ..Default::default()
        };
        let entries = refresh_entries_for(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].wallet_id, "w2");
        assert_eq!(entries[0].chain_id, Chain::Solana.str_id());
    }

    /// The EVM family shares one address, so an Ethereum wallet's entry is its
    /// own — and an Arbitrum wallet reads the same slot.
    #[test]
    fn the_evm_family_shares_one_address() {
        let state = CoreAppState {
            wallets: vec![wallet("w1", Chain::Arbitrum, &[(Chain::Ethereum, "0xabc")])],
            ..Default::default()
        };
        let entries = refresh_entries_for(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].address, "0xabc");
        assert_eq!(entries[0].chain_id, Chain::Arbitrum.str_id());
    }

    /// The maintenance loop runs while the app is active with something to
    /// fetch, reports each tick to the observer, and stops in the background.
    #[tokio::test]
    async fn device_conditions_start_and_stop_the_maintenance_loop() {
        use super::{RefreshEngine, RefreshObserver};
        use crate::fetch::refresh_policy::DeviceConditions;
        use crate::service::WalletService;
        use crate::service::app_refresh::AppRefreshResult;
        use crate::store::state::StateCommand;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::Duration;

        struct Ticks(AtomicU32, AtomicU32);
        impl RefreshObserver for Ticks {
            fn on_balance_updated(&self, _: String, _: String, _: Option<WalletState>) {}
            fn on_refresh_cycle_complete(&self, _: u32, _: u32) {}
            fn on_refresh_complete(&self, _: AppRefreshResult) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            fn on_tor_status_changed(&self, _: crate::tor::TorStatus) {
                self.1.fetch_add(1, Ordering::SeqCst);
            }
        }
        let conditions = |active: bool| DeviceConditions {
            app_is_active: active,
            // Offline: a tick answers without touching a network.
            is_network_reachable: false,
            is_constrained_network: false,
            is_expensive_network: false,
            is_low_power_mode: false,
            battery_level: 1.0,
            wants_price_refresh: true,
        };
        let service = WalletService::new(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "maintenance-loop-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        let engine = RefreshEngine::new(service.clone());
        let ticks = Arc::new(Ticks(AtomicU32::new(0), AtomicU32::new(0)));
        engine.set_observer(ticks.clone());

        engine.set_device_conditions(conditions(true)).await;
        assert!(
            engine.inner.maintenance_stop_tx.lock().unwrap().is_none(),
            "no wallets, no loop"
        );

        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletState::single_address(
                    "w",
                    "W",
                    "ethereum",
                    "0x1111111111111111111111111111111111111111",
                    None,
                    true,
                ),
            })
            .await
            .unwrap();
        engine.reconcile_wallets().await;
        for _ in 0..100 {
            if ticks.0.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            ticks.0.load(Ordering::SeqCst) > 0,
            "the first tick runs at once"
        );
        assert!(
            ticks.1.load(Ordering::SeqCst) > 0,
            "the observer is told the Tor status without asking"
        );

        engine.set_device_conditions(conditions(false)).await;
        assert!(engine.inner.maintenance_stop_tx.lock().unwrap().is_none());
        assert!(!engine.is_running());
    }

    /// Replacing the wallet list is not a reason to refresh unless what a sweep
    /// fetches changed. Answering yes to every replacement is what made the end
    /// of each sweep — which replaces the list with new balances — start the
    /// next one.
    #[tokio::test]
    async fn only_a_change_in_what_is_fetched_counts_as_a_change() {
        use super::RefreshEngine;
        use crate::service::WalletService;
        use crate::store::state::StateCommand;

        let service = WalletService::new(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "reconcile-wallets-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        let engine = RefreshEngine::new(service.clone());
        let upsert = |name: &str| StateCommand::UpsertWallet {
            wallet: WalletState::single_address(
                "w",
                name,
                "ethereum",
                "0x1111111111111111111111111111111111111111",
                None,
                true,
            ),
        };

        assert!(!engine.reconcile_wallets().await, "no wallets, no entries");
        service.apply_state_command(upsert("W")).await.unwrap();
        assert!(engine.reconcile_wallets().await, "a new wallet is a change");
        assert!(
            !engine.reconcile_wallets().await,
            "the same list again is not"
        );

        // A rename reaches the list and not the fetch.
        service
            .apply_state_command(upsert("Renamed"))
            .await
            .unwrap();
        assert!(!engine.reconcile_wallets().await);

        service
            .apply_state_command(StateCommand::RemoveWallet {
                wallet_id: "w".into(),
            })
            .await
            .unwrap();
        assert!(engine.reconcile_wallets().await, "a removal is a change");
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl RefreshEngine {
    /// Run one refresh cycle immediately without waiting for the next tick.
    /// Also `async` to guarantee Tokio context for `tokio::spawn`.
    ///
    /// If a cycle is already in flight, sets `pending_trigger` so the running
    /// cycle will re-run once it finishes (picks up any entry changes that
    /// arrived while the cycle was running). `refresh_now` is the variant that
    /// waits for the sweep instead of spawning it.
    pub async fn trigger_immediate(&self) {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            inner.pending_trigger.store(true, Ordering::Release);
            Self::run_cycle(&inner).await;
        });
    }
}
