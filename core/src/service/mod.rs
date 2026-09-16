//! The stateful object the shells talk to: `WalletService` owns the resident
//! state, the HTTP client and the endpoint lists, and every long-lived thing
//! core holds on a front end's behalf. Stateless helpers belong beside the
//! domain module that owns them.
//!
//! Split by owner, one module each:
//!
//! | module | owns |
//! |---|---|
//! | [`state`] | resident `CoreAppState` projections and serialized persistence |
//! | [`network`] | endpoint health and transaction status |
//! | `network_balance`, `network_tokens`, `network_history`, `network_hd`, `network_prices` | chain reads grouped by responsibility |
//! | [`send_preview`] | fee estimates and send previews |
//! | [`send_signing`] | protocol signing and submission dispatch |
//! | [`send_broadcast`] | rebroadcast of signed payloads |
//! | [`send_destination`] | resolution and review verification |
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
pub(crate) use crate::state::{reduce_state_in_place, CoreAppState, StateCommand, StateTransition};
pub(crate) use crate::store::secret_store::SecretStore;
pub(crate) use crate::store::wallet_domain::AssetHolding;
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

mod address_discovery;
mod balance_refresh;
mod diagnostic_state;
pub use diagnostic_state::{DiagnosticCommand, DiagnosticLog, DiagnosticLogInput, DiagnosticState};
mod funds_scan;
pub use funds_scan::{FundsScan, FundsScanProgress, FundsScanRead};
mod helpers;
mod history_bitcoin;
mod history_cursor;
mod history_derived;
mod history_refresh;
pub use history_refresh::HistoryRefreshOutcome;
mod history_operation;
pub use history_operation::{ChainHistoryRefresh, HistoryRefreshScope};
mod keypool;
mod maintenance;
mod network;
mod network_balance;
mod network_hd;
mod network_history;
mod network_prices;
pub use network_prices::{fetch_fiat_rates_typed, fetch_prices_typed, QuoteRefreshState};
mod network_tokens;
mod operational_events;
mod pending_status;
pub use pending_status::{PendingMaintenanceFailure, PendingMaintenanceResult};
mod movement;
mod reset;
mod send_broadcast;
mod send_destination;
mod send_execution;
mod send_identity;
mod send_params;
mod send_preview;
mod send_records;
mod send_result;
mod send_signing;
mod staking;
mod standalone;
pub use movement::PortfolioMovementBaseline;
pub use reset::ResetOutcome;
mod state;
mod transaction_recheck;
mod transactions;
mod types;
mod wallet_import;

pub(crate) use helpers::*;
use keypool::keypool_key;
#[cfg(test)]
use send_destination::{resolve_destination, verify_reviewed_destination};
pub use standalone::*;
/// The confirmation-poll outcome, which lives with the trackers it updates.
pub use transactions::StatusPollOutcome;
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
#[derive(Clone, uniffi::Object)]
pub struct WalletService {
    pub(crate) send_reviews: Arc<tokio::sync::Mutex<HashMap<String, send_review::ReviewedSend>>>,
    app_refresh_lock: Arc<tokio::sync::Mutex<()>>,
    quote_refresh_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) trc20_metadata: Arc<crate::fetch::chains::tron::MetadataCache>,

    /// Serializes persistent mutations, including database binding.
    pub(crate) state_writer: Arc<tokio::sync::Mutex<()>>,
    uses_catalog_endpoints: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) endpoints: Arc<AsyncRwLock<EndpointIndex>>,
    /// Per-wallet history pagination state (cursor / page / exhaustion).
    pub(crate) history_pagination: Arc<HistoryPaginationStore>,
    /// Optional Keychain delegate (set via `set_secret_store`).
    pub(crate) secret_store: Arc<std::sync::RwLock<Option<Arc<dyn SecretStore>>>>,
    /// Canonical in-memory wallet + holdings state.
    pub(crate) wallet_state: Arc<AsyncRwLock<CoreAppState>>,
    /// Database handle for persistent state and key/value storage.
    /// Unbound until `open_state` is called, in which case commands apply in
    /// memory only — the shape tests and short-lived tools want that.
    pub(crate) state_binding: Arc<crate::service::state::StateBinding>,
    /// User's Etherscan V2 API key. Shared across all EVM chains: Etherscan v2
    /// dispatches by `chainid` parameter against a single host.
    /// Confirmation-poll backoff state, keyed by transaction id. Not persisted:
    /// a restart should re-poll every pending transaction immediately, which is
    /// what an absent tracker already means.
    pub(crate) status_trackers: Arc<AsyncRwLock<HashMap<String, TransactionStatusTrackerState>>>,
    /// Keypool indices and the addresses already issued from them.
    ///
    /// Persisted, unlike `status_trackers`, because losing either table means
    /// handing out an address somebody already holds. Held in memory so that
    /// reserve-and-increment happens atomically under one lock; every mutation
    /// writes through to `wallet_keypool` before returning. The two tables are
    /// one type and one lock — see [`crate::service::keypool::Keypool`].
    pub(crate) keypool: Arc<crate::service::keypool::Keypool>,
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
#[uniffi::export]
impl WalletService {
    #[uniffi::constructor]
    pub fn new(endpoints: Vec<ChainEndpoints>) -> Result<Arc<Self>, SpectraBridgeError> {
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
            trc20_metadata: Arc::new(crate::fetch::chains::tron::MetadataCache::default()),
            send_reviews: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            app_refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            quote_refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            state_writer: Arc::new(tokio::sync::Mutex::new(())),
            uses_catalog_endpoints: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            endpoints: Arc::new(AsyncRwLock::new(EndpointIndex::from_list(endpoints))),
            history_pagination: Arc::new(HistoryPaginationStore::new()),
            secret_store: Arc::new(std::sync::RwLock::new(None)),
            wallet_state: Arc::new(AsyncRwLock::new(CoreAppState::default())),
            state_binding: Arc::new(crate::service::state::StateBinding::default()),
            status_trackers: Arc::new(AsyncRwLock::new(HashMap::new())),
            keypool: Arc::new(crate::service::keypool::Keypool::default()),
            refresh_clock: Arc::new(AsyncRwLock::new(Default::default())),
        }))
    }

    #[uniffi::constructor]
    pub fn new_catalog() -> Result<Arc<Self>, SpectraBridgeError> {
        let service = Self::new(catalog_endpoints()?)?;
        service
            .uses_catalog_endpoints
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(service)
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
}

impl WalletService {
    pub(crate) async fn endpoints_for(&self, chain_id: &str) -> Arc<Vec<String>> {
        let base = self
            .endpoints
            .read()
            .await
            .endpoints
            .get(chain_id)
            .cloned()
            .unwrap_or_default();
        if self
            .uses_catalog_endpoints
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            if let Some(chain) = Chain::from_str_id(chain_id) {
                let custom = self
                    .wallet_state
                    .read()
                    .await
                    .settings
                    .rpc_endpoint_by_chain
                    .get(chain.chain_display_name())
                    .cloned();
                if let Some(custom) = custom.filter(|v| !v.trim().is_empty()) {
                    let mut endpoints = vec![custom.clone()];
                    endpoints.extend(base.iter().filter(|v| **v != custom).cloned());
                    return Arc::new(endpoints);
                }
            }
        }
        base
    }

    pub(crate) async fn api_key_for(&self, chain_id: &str) -> Option<String> {
        let guard = self.endpoints.read().await;
        guard.api_keys.get(chain_id).cloned()
    }
}

/// Catalog transport configuration for a non-platform front end.
pub fn catalog_endpoints() -> Result<Vec<ChainEndpoints>, SpectraBridgeError> {
    let mut endpoints = Vec::new();
    for row in crate::app_core_chain_endpoints()? {
        let chain = Chain::from_str_id(&row.chain_id).expect("catalog chain");
        let primary = if chain.is_evm() {
            row.evm_rpc
        } else {
            crate::endpoint_records_for_chain_masked(
                row.chain_name,
                crate::app_core::ENDPOINT_ROLE_RPC
                    | crate::app_core::ENDPOINT_ROLE_BALANCE
                    | crate::app_core::ENDPOINT_ROLE_BACKEND,
                false,
            )?
            .into_iter()
            .map(|r| r.endpoint)
            .collect()
        };
        endpoints.push(ChainEndpoints {
            chain_id: row.chain_id,
            endpoints: primary,
            api_key: None,
        });
        if !row.explorer_supplemental.is_empty() {
            endpoints.push(ChainEndpoints {
                chain_id: chain.endpoint_str_id(chain.supplemental_endpoint_slot()),
                endpoints: row.explorer_supplemental,
                api_key: None,
            });
        }
        if !chain.secondary_endpoint_ids().is_empty() {
            endpoints.push(ChainEndpoints {
                chain_id: chain.endpoint_str_id(crate::registry::EndpointSlot::Secondary),
                endpoints: crate::app_core_endpoints_for_ids(
                    chain
                        .secondary_endpoint_ids()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                )?,
                api_key: None,
            });
        }
    }
    Ok(endpoints)
}

#[cfg(test)]
mod a_primary_endpoint_can_serve_a_primary_read {
    use super::*;

    /// A chain's primary list holds only endpoints that answer one of the
    /// roles it was filtered on.
    ///
    /// `catalog_endpoints` asks a non-EVM chain for `RPC | BALANCE | BACKEND`
    /// and hands the result to `with_fallback`, which tries them top to bottom
    /// for reads. An endpoint that serves none of the three is not a slower
    /// fallback, it is a wrong one: `ENDPOINT_ROLE_BACKEND` was written
    /// `1 << 9` like `ENDPOINT_ROLE_INDEXER`, so the mask also matched every
    /// indexer, and Bitcoin Cash's list picked up
    /// `…/push/transaction` (broadcast only) and
    /// `…/dashboards/transaction/` (a verification URL prefix) as its second
    /// and third choices for a balance read.
    ///
    /// Stated over the catalog rather than over the constants, so it holds
    /// whatever the mask is next written as. Supplemental and secondary rows
    /// are deliberately excluded — those carry `web-link` explorers on
    /// purpose, and they are the `:explorer` / `:secondary` ids here.
    #[test]
    fn no_chain_is_offered_an_endpoint_that_answers_none_of_them() {
        let mut checked = 0;
        for row in catalog_endpoints().expect("catalog endpoints") {
            if Chain::from_str_id(&row.chain_id).is_none() {
                continue; // a `:secondary` or `:explorer` slot, not the primary list
            }
            for endpoint in &row.endpoints {
                let Some(tag) = crate::app_core_endpoint_tag(endpoint.clone()) else {
                    continue; // no catalog row: a user-typed RPC or an assembled base
                };
                let serves = tag.kind == "rpc-node"
                    || tag.kind == "backend"
                    || tag.capabilities.iter().any(|c| c == "balance");
                assert!(
                    serves,
                    "{} lists {endpoint} as a primary endpoint, but it is a {:?} \
                     claiming only {:?}",
                    row.chain_id, tag.kind, tag.capabilities
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no primary endpoint was checked at all");
    }
}

#[cfg(test)]
mod app_boundary_tests;

pub use address_discovery::WalletAddressDiscovery;

impl WalletService {
    pub async fn update_endpoints_typed(
        &self,
        endpoints: Vec<ChainEndpoints>,
    ) -> Result<(), SpectraBridgeError> {
        self.uses_catalog_endpoints
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let mut guard = self.endpoints.write().await;
        *guard = EndpointIndex::from_list(endpoints);
        Ok(())
    }

    async fn owned_etherscan_api_key(&self) -> String {
        self.wallet_state
            .read()
            .await
            .settings
            .etherscan_api_key
            .clone()
    }
}

pub mod app_refresh;
mod owned_send;
pub mod send_review;

pub use owned_send::{OwnedReplacementDraft, OwnedSendQuote};

impl WalletService {
    pub(crate) fn secrets(&self) -> Result<Arc<dyn SecretStore>, SpectraBridgeError> {
        self.secret_store
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| SpectraBridgeError::from("secret store not registered".to_string()))
    }
}
