//! The stateful object the shells talk to: `WalletService` owns the resident
//! state, the HTTP client and the endpoint lists, and every long-lived thing
//! core holds on a front end's behalf. Stateless helpers belong beside the
//! domain module that owns them.
//!
//! Split by owner, one module each:
//!
//! | module | owns |
//! |---|---|
//! | [`state`] | the resident `CoreAppState`, its persistence, transactions, keypool |
//! | [`network`] | every read that talks to a chain, and the per-chain dispatch |
//! | [`send`] | fee estimation, signing, broadcast, and send previews |
//! | [`helpers`] | parsing, scaling and SQLite plumbing the three share |
//! | [`types`] | the records and enums that cross the FFI |
//! | [`standalone`] | exports that need no service state at all |
//!
//! Rust permits many `impl` blocks per type and UniFFI exports them as one, so
//! a method's module says who owns it and nothing else.
//!
//! `WalletService` owns no long-lived secrets: signing material arrives per
//! call and is scrubbed after use.
//!
//! Methods that are `pub(crate)` rather than exported are internal — UniFFI
//! exports *every* method of an `#[uniffi::export]` block regardless of
//! visibility, so anything that should stay off the FFI lives in a plain
//! `impl` block.

pub(crate) use crate::fetch::chains::{
    aptos::AptosClient, bitcoin::BitcoinClient, bitcoin::UtxoTxStatus,
    bitcoin_cash::BitcoinCashClient, bitcoin_gold::BitcoinGoldClient, bitcoin_sv::BitcoinSvClient,
    bittensor::BittensorClient, cardano::CardanoClient, dash::DashClient, decred::DecredClient,
    dogecoin::DogecoinClient, evm::EvmClient, icp::IcpClient, kaspa::KaspaClient,
    litecoin::LitecoinClient, monero::MoneroClient, near::NearClient, polkadot::PolkadotClient,
    solana::SolanaClient, stellar::StellarClient, sui::SuiClient, ton::TonClient, tron::TronClient,
    xrp::XrpClient, zcash::ZcashClient,
};
pub(crate) use crate::fetch::history_store::HistoryPaginationStore;
pub(crate) use crate::http::HttpClient;
pub(crate) use crate::registry::{Chain, EndpointSlot};
pub(crate) use crate::send::chains::bitcoin::{
    sign_and_broadcast as bitcoin_sign_and_broadcast, BitcoinSendParams,
};
pub(crate) use crate::state::{
    reduce_state_in_place, CoreAppState, StateCommand, StateTransition,
};
pub(crate) use crate::store::wallet_domain::AssetHolding;
pub(crate) use crate::store::secret_store::SecretStore;
pub(crate) use crate::store::{TransactionStatusPollConfig, TransactionStatusTrackerState};
pub(crate) use crate::SpectraBridgeError;

pub(crate) use serde_json::json;
pub(crate) use std::collections::HashMap;
pub(crate) use std::sync::Arc;
/// `WalletService`'s own resident state uses this — tokio's async lock,
/// held across `.await` where a mutation needs to write through to SQLite
/// before releasing it.
///
/// Named `AsyncRwLock` rather than re-exported as the bare `RwLock` on
/// purpose: two of `WalletService`'s ten locked fields
/// (`secret_store`, `etherscan_api_key`) are `std::sync::RwLock` instead —
/// a synchronous lock, because `set_secret_store` and `set_etherscan_api_key`
/// are plain `pub fn`s Swift calls without `await`, and switching their lock
/// would force them async and cascade into every synchronous call site that
/// reaches them. A bare `RwLock<T>` field type reads as "the normal one" no
/// matter which it is; spelling out which kind a field holds means a reader
/// never has to open this file's imports to find out.
pub(crate) use tokio::sync::RwLock as AsyncRwLock;

pub(crate) use serde::{Deserialize, Serialize};

mod helpers;
mod history_derived;
mod history_cursor;
mod maintenance;
mod network;
mod send;
mod send_execution;
mod send_params;
mod standalone;
mod state;
mod types;

pub(crate) use helpers::*;
pub use standalone::*;
/// The confirmation-poll outcome, which lives with the trackers it updates.
pub use state::StatusPollOutcome;
pub use types::*;

// ── Endpoint index (internal — pre-indexed for O(1) chain_id lookup) ──────

#[derive(Debug, Clone, Default)]
pub(crate) struct EndpointIndex {
    endpoints: std::collections::HashMap<String, Arc<Vec<String>>>,
    api_keys: std::collections::HashMap<String, String>,
}

impl EndpointIndex {
    fn from_list(list: Vec<ChainEndpoints>) -> Self {
        let mut endpoints = std::collections::HashMap::with_capacity(list.len());
        let mut api_keys = std::collections::HashMap::new();
        for entry in list {
            endpoints.insert(entry.chain_id.clone(), Arc::new(entry.endpoints));
            if let Some(key) = entry.api_key {
                api_keys.insert(entry.chain_id, key);
            }
        }
        Self {
            endpoints,
            api_keys,
        }
    }
}

// ── WalletService — primary UniFFI-exported object ────────────────────────

/// Swift holds one instance for the lifetime of the app session.
#[derive(uniffi::Object)]
pub struct WalletService {
    pub(crate) endpoints: Arc<AsyncRwLock<EndpointIndex>>,
    /// Per-wallet history pagination state (cursor / page / exhaustion).
    pub(crate) history_pagination: Arc<HistoryPaginationStore>,
    /// Optional Keychain delegate (set via `set_secret_store`).
    pub(crate) secret_store: Arc<std::sync::RwLock<Option<Arc<dyn SecretStore>>>>,
    /// Canonical in-memory wallet + holdings state.
    pub(crate) wallet_state: Arc<AsyncRwLock<CoreAppState>>,
    /// Where `wallet_state` is persisted. `None` until `open_state` is called,
    /// in which case commands apply in memory only — the shape tests and
    /// short-lived tools want.
    pub(crate) state_db_path: Arc<AsyncRwLock<Option<String>>>,
    /// User's Etherscan V2 API key. Shared across all EVM chains: Etherscan v2
    /// dispatches by `chainid` parameter against a single host.
    pub(crate) etherscan_api_key: Arc<std::sync::RwLock<String>>,
    /// Confirmation-poll backoff state, keyed by transaction id. Not persisted:
    /// a restart should re-poll every pending transaction immediately, which is
    /// what an absent tracker already means.
    pub(crate) status_trackers: Arc<AsyncRwLock<HashMap<String, TransactionStatusTrackerState>>>,
    /// Keypool indices, keyed by `wallet_id|chain_name`.
    ///
    /// Unlike `status_trackers` this IS persisted — losing it means reissuing
    /// an address that was already handed out. Held in memory so that
    /// reserve-and-increment can happen atomically under one lock; every
    /// mutation writes through to `wallet_keypool` before returning.
    pub(crate) keypool: Arc<AsyncRwLock<HashMap<String, crate::wallet_db::KeypoolState>>>,
    /// Addresses this wallet is known to own, keyed by chain name.
    ///
    /// Persisted like the keypool and for the same reason: the keypool
    /// baseline is computed from the highest index already handed out, so
    /// losing the table means reissuing an address. Held in memory because
    /// every keypool operation reads it under the keypool's own lock.
    pub(crate) owned_addresses:
        Arc<AsyncRwLock<HashMap<String, Vec<crate::wallet_db::OwnedAddressRecord>>>>,
    /// Per-chain operational log, newest first, capped per chain.
    ///
    /// Not in `CoreAppState`: 200 entries × every chain is too much to clone
    /// on an unrelated `SetFiatCurrency`. Same in-memory + write-through shape
    /// as the keypool.
    pub(crate) operational_events:
        Arc<AsyncRwLock<HashMap<String, Vec<crate::store::ChainOperationalEventRecord>>>>,
    /// When each kind of refresh last ran, in unix seconds.
    ///
    /// Not persisted, and that is the whole difference from the keypool: a
    /// restart should refresh, which is exactly what an empty clock already
    /// means. It was five `Date?` properties and two dictionaries on the iOS
    /// side, handed back to core as arguments on every scheduling question —
    /// so the answer was only as current as the caller's copy, and the CLI,
    /// which has no such properties, could not ask the question at all.
    pub(crate) refresh_clock: Arc<AsyncRwLock<crate::fetch::refresh::policy::RefreshClock>>,
}
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    #[uniffi::constructor]
    pub fn new_typed(endpoints: Vec<ChainEndpoints>) -> Result<Arc<Self>, SpectraBridgeError> {
        // A library installing a global subscriber is already a liberty; one
        // that writes to *stdout* at *debug* is a bug. It corrupted every
        // `spectra --json` run — core's connection logs landed in the middle of
        // the document — and a caller has no way to opt out of a `OnceLock`.
        //
        // Now: stderr, and quiet unless asked. `RUST_LOG=debug` restores what
        // debug builds used to do by default.
        static LOGGING: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        LOGGING.get_or_init(|| {
            use tracing_subscriber::{fmt, EnvFilter};
            let filter =
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
            let _ = fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .without_time()
                .with_ansi(false)
                .try_init();
        });
        Ok(Arc::new(Self {
            endpoints: Arc::new(AsyncRwLock::new(EndpointIndex::from_list(endpoints))),
            history_pagination: Arc::new(HistoryPaginationStore::new()),
            secret_store: Arc::new(std::sync::RwLock::new(None)),
            wallet_state: Arc::new(AsyncRwLock::new(CoreAppState::default())),
            state_db_path: Arc::new(AsyncRwLock::new(None)),
            etherscan_api_key: Arc::new(std::sync::RwLock::new(String::new())),
            status_trackers: Arc::new(AsyncRwLock::new(HashMap::new())),
            keypool: Arc::new(AsyncRwLock::new(HashMap::new())),
            owned_addresses: Arc::new(AsyncRwLock::new(HashMap::new())),
            refresh_clock: Arc::new(AsyncRwLock::new(Default::default())),
            operational_events: Arc::new(AsyncRwLock::new(HashMap::new())),
        }))
    }

    pub async fn update_endpoints_typed(
        &self,
        endpoints: Vec<ChainEndpoints>,
    ) -> Result<(), SpectraBridgeError> {
        let mut guard = self.endpoints.write().await;
        *guard = EndpointIndex::from_list(endpoints);
        Ok(())
    }

    /// Swift pushes the user's Etherscan V2 API key here. Used for EVM history
    /// fetches across every indexed EVM chain (chainid is passed as a query
    /// param, so one key covers all of them).
    pub fn set_etherscan_api_key(&self, key: String) {
        if let Ok(mut guard) = self.etherscan_api_key.write() {
            *guard = key;
        }
    }

    // `fetch_native_balance_summary_auto` lives in the plain-impl block below
    // — an internal helper, not exported to Swift.

    /// Register the platform Keychain implementation. Must be called once at
    /// app start before any code path that reads or writes secrets. Rust code
    /// that needs secret I/O calls the delegate directly via `self.secret_store`;
    /// there are deliberately no pass-through FFI wrappers — all secret traffic
    /// is driven by Rust.
    pub fn set_secret_store(&self, store: Arc<dyn SecretStore>) {
        if let Ok(mut guard) = self.secret_store.write() {
            *guard = Some(store);
        }
    }

    /// The registered delegate, or an error naming the reason.
    ///
    /// Every secret read and write goes through this rather than through a
    /// key the front end computed: the key layout is core's, and there were
    /// two of them for as long as the front end owned one.
    pub(crate) fn secrets(&self) -> Result<Arc<dyn SecretStore>, SpectraBridgeError> {
        self.secret_store
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| SpectraBridgeError::from("secret store not registered".to_string()))
    }
}

impl WalletService {
    pub(crate) async fn endpoints_for(&self, chain_id: &str) -> Arc<Vec<String>> {
        let guard = self.endpoints.read().await;
        guard
            .endpoints
            .get(chain_id)
            .cloned()
            .unwrap_or_else(|| Arc::new(Vec::new()))
    }

    pub(crate) async fn api_key_for(&self, chain_id: &str) -> Option<String> {
        let guard = self.endpoints.read().await;
        guard.api_keys.get(chain_id).cloned()
    }
}
