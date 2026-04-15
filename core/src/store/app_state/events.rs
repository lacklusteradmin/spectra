// Phase-1 AppState event bus.
//
// Goal: make Rust the authoritative orchestrator of the three core SwiftUI
// mirror collections (wallets, transactions, addressBook) plus the diagnostics
// registry. Every mutating `store_*` writer publishes a typed event on an
// internal `tokio::sync::broadcast` channel. Swift registers an
// `AppStateObserver` via UniFFI's foreign-trait support; a Rust pump task
// converts broadcast messages into observer callbacks so Swift's @Published
// mirrors update automatically — without Swift having to re-read the store or
// manually call `objectWillChange.send()`.
//
// The channel is process-global and lazily initialised. The pump task is
// owned by an `ObserverHandle` returned from `register_app_state_observer`;
// dropping the handle (or calling `unregister()`) stops the pump.

use std::sync::{Arc, OnceLock};

use tokio::sync::broadcast;

use crate::persistence::models::{
    CorePersistedAddressBookEntry, CorePersistedTransactionRecord,
};
use crate::wallet_domain::CoreImportedWallet;

// ── Event payload ───────────────────────────────────────────────────────
#[derive(Clone, Debug)]
pub enum AppStateEvent {
    WalletsChanged(Vec<CoreImportedWallet>),
    TransactionsChanged(Vec<CorePersistedTransactionRecord>),
    AddressBookChanged(Vec<CorePersistedAddressBookEntry>),
    DiagnosticsChanged,
}

const CHANNEL_CAPACITY: usize = 256;

fn channel() -> &'static broadcast::Sender<AppStateEvent> {
    static TX: OnceLock<broadcast::Sender<AppStateEvent>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        tx
    })
}

/// Publish an event. Silently drops if there are no subscribers (expected
/// during early app startup or in unit tests without an observer).
pub fn publish(event: AppStateEvent) {
    let _ = channel().send(event);
}

/// Test-only: fresh receiver.
#[cfg(test)]
pub fn subscribe() -> broadcast::Receiver<AppStateEvent> {
    channel().subscribe()
}

// ── Foreign-trait observer ──────────────────────────────────────────────
/// Implemented by Swift's `AppState`. Each method must be cheap and must hop
/// to `@MainActor` before mutating any `@Published` property.
#[uniffi::export(with_foreign)]
pub trait AppStateObserver: Send + Sync {
    fn wallets_changed(&self, wallets: Vec<CoreImportedWallet>);
    fn transactions_changed(&self, transactions: Vec<CorePersistedTransactionRecord>);
    fn address_book_changed(&self, entries: Vec<CorePersistedAddressBookEntry>);
    fn diagnostics_changed(&self);
}

/// Handle returned from `register_app_state_observer`. Dropping it stops the
/// pump task. Swift keeps this alive for the lifetime of `AppState`.
#[derive(uniffi::Object)]
pub struct AppStateObserverHandle {
    stop_tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[uniffi::export]
impl AppStateObserverHandle {
    /// Explicit early unregister. Safe to call multiple times.
    pub fn unregister(&self) {
        if let Some(tx) = self.stop_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

/// Register a Swift observer. Spawns a tokio task that pumps broadcast events
/// into observer callbacks. Returns a handle that Swift must retain; dropping
/// it stops the pump.
///
/// `async` so it runs inside UniFFI's tokio runtime (required for
/// `tokio::spawn`).
#[uniffi::export(async_runtime = "tokio")]
pub async fn register_app_state_observer(
    observer: Arc<dyn AppStateObserver>,
) -> Arc<AppStateObserverHandle> {
    let mut rx = channel().subscribe();
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                msg = rx.recv() => match msg {
                    Ok(AppStateEvent::WalletsChanged(w)) => observer.wallets_changed(w),
                    Ok(AppStateEvent::TransactionsChanged(t)) => observer.transactions_changed(t),
                    Ok(AppStateEvent::AddressBookChanged(e)) => observer.address_book_changed(e),
                    Ok(AppStateEvent::DiagnosticsChanged) => observer.diagnostics_changed(),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // We fell behind; re-sync from current store state.
                        observer.wallets_changed(super::store::store_wallets_get_all());
                        observer.transactions_changed(super::store::store_transactions_get_all());
                        observer.address_book_changed(super::store::store_address_book_get_all());
                        observer.diagnostics_changed();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });

    Arc::new(AppStateObserverHandle {
        stop_tx: std::sync::Mutex::new(Some(stop_tx)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct MockObserver {
        wallets: StdMutex<Vec<Vec<CoreImportedWallet>>>,
        transactions: StdMutex<Vec<Vec<CorePersistedTransactionRecord>>>,
        address_book: StdMutex<Vec<Vec<CorePersistedAddressBookEntry>>>,
        diagnostics: StdMutex<u32>,
    }

    impl MockObserver {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                wallets: StdMutex::new(vec![]),
                transactions: StdMutex::new(vec![]),
                address_book: StdMutex::new(vec![]),
                diagnostics: StdMutex::new(0),
            })
        }
    }

    impl AppStateObserver for MockObserver {
        fn wallets_changed(&self, w: Vec<CoreImportedWallet>) {
            self.wallets.lock().unwrap().push(w);
        }
        fn transactions_changed(&self, t: Vec<CorePersistedTransactionRecord>) {
            self.transactions.lock().unwrap().push(t);
        }
        fn address_book_changed(&self, e: Vec<CorePersistedAddressBookEntry>) {
            self.address_book.lock().unwrap().push(e);
        }
        fn diagnostics_changed(&self) {
            *self.diagnostics.lock().unwrap() += 1;
        }
    }

    #[tokio::test]
    async fn observer_receives_all_event_kinds() {
        let mock = MockObserver::new();
        let handle =
            register_app_state_observer(mock.clone() as Arc<dyn AppStateObserver>).await;

        // Give the pump task a tick to subscribe.
        tokio::task::yield_now().await;

        publish(AppStateEvent::WalletsChanged(vec![]));
        publish(AppStateEvent::TransactionsChanged(vec![]));
        publish(AppStateEvent::AddressBookChanged(vec![]));
        publish(AppStateEvent::DiagnosticsChanged);

        // Let the pump drain.
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }

        // Other tests may publish on the global channel concurrently; assert
        // we observed at least our own publishes.
        assert!(mock.wallets.lock().unwrap().len() >= 1);
        assert!(mock.transactions.lock().unwrap().len() >= 1);
        assert!(mock.address_book.lock().unwrap().len() >= 1);
        assert!(*mock.diagnostics.lock().unwrap() >= 1);

        handle.unregister();
    }

    #[tokio::test]
    async fn unregister_stops_pump() {
        let mock = MockObserver::new();
        let handle =
            register_app_state_observer(mock.clone() as Arc<dyn AppStateObserver>).await;
        tokio::task::yield_now().await;
        handle.unregister();
        // Allow a generous window for the stop signal; even if other
        // concurrent publishers race in before the select! observes it,
        // the count should stabilise.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        let stable = *mock.diagnostics.lock().unwrap();
        // Publish a burst; after stop, the pump must not continue counting.
        for _ in 0..10 {
            publish(AppStateEvent::DiagnosticsChanged);
        }
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        let after = *mock.diagnostics.lock().unwrap();
        // Allow at most 1 in-flight delta (signal race); reject 10+ which
        // would mean the pump is still running.
        assert!(
            after <= stable + 1,
            "pump should be stopped after unregister (stable={stable}, after={after})"
        );
    }
}
