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
//! | [`send_preflight`] | send eligibility, routing and recipient warnings |
//! | [`send_preview`] | fee estimates and send previews |
//! | [`send_stages`] | prepared/signed artifacts and explicit submission |
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

pub(crate) use crate::fetch::history_store::HistoryPaginationStore;
pub(crate) use crate::fetch::http::HttpClient;
pub(crate) use crate::fetch::{
    aptos::AptosClient, bitcoin::BitcoinClient, bitcoin::UtxoTxStatus, bitcoin_sv::BitcoinSvClient,
    bittensor::BittensorClient, blockbook::BlockbookClient, cardano::CardanoClient,
    decred::DecredClient, dogecoin::DogecoinClient, evm::EvmClient, icp::IcpClient,
    kaspa::KaspaClient, monero::MoneroClient, near::NearClient, polkadot::PolkadotClient,
    solana::SolanaClient, stellar::StellarClient, sui::SuiClient, ton::TonClient, tron::TronClient,
    xrp::XrpClient,
};
pub(crate) use crate::registry::{Chain, EndpointSlot};
pub(crate) use crate::store::secret_store::SecretStore;
pub(crate) use crate::store::state::{
    reduce_state_in_place, CoreAppState, StateCommand, StateTransition,
};
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
/// purpose: `secret_store` uses a synchronous lock because its setter is
/// called without `await`. A bare `RwLock<T>` field type reads as "the normal one" no
/// matter which it is; spelling out which kind a field holds means a reader
/// never has to open this file's imports to find out.
pub(crate) use tokio::sync::RwLock as AsyncRwLock;

pub(crate) use serde::{Deserialize, Serialize};

mod address_discovery;
mod balance_refresh;
mod diagnostic_state;
pub use diagnostic_state::{
    ConfiguredSelfTestReport, DiagnosticCommand, DiagnosticLog, DiagnosticLogInput,
    DiagnosticLogLevel, DiagnosticState,
};
mod funds_scan;
pub use funds_scan::{FundsScan, FundsScanProgress, FundsScanRead};
mod helpers;
mod history_bitcoin;
mod history_cursor;
pub(crate) mod history_derived;
mod history_query;
pub use history_query::{HistoryPage, HistoryQuery, HistoryQueryFilter, TransactionSnapshot};
mod history_refresh;
pub use history_refresh::{HistoryRefreshOutcome, HistoryWalletDiagnostics};
mod history_operation;
pub use history_operation::{ChainHistoryRefresh, HistoryRefreshScope};
mod keypool;
mod maintenance;
mod network;
mod network_balance;
mod network_hd;
mod network_history;
mod network_prices;
pub use network_prices::{fetch_fiat_rates, fetch_prices, QuoteRefreshState};
mod endpoint_directory;
mod network_tokens;
pub use endpoint_directory::{CustomEndpoint, EndpointDirectoryEntry};
mod operational_events;
mod pending_status;
pub use pending_status::{PendingMaintenanceFailure, PendingMaintenanceResult};
mod movement;
mod valuation;
pub use state::PortfolioSnapshot;
pub use valuation::{PortfolioValuation, QuotedTotal};
mod reset;
mod send_broadcast;
mod send_destination;
mod send_execution;
mod send_identity;
mod send_preflight;
mod send_preview;
mod send_records;
mod staking;
mod standalone;
pub use movement::PortfolioMovementBaseline;
pub use reset::ResetOutcome;
mod state;
mod transaction_actions;
mod transaction_recheck;
pub use transaction_actions::TransactionActions;
mod transactions;
mod transport;
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
    capabilities: std::collections::HashMap<String, Vec<String>>,
    endpoints: std::collections::HashMap<String, Arc<Vec<String>>>,
}

impl EndpointIndex {
    fn from_list(list: Vec<ChainEndpoints>) -> Result<Self, SpectraBridgeError> {
        for row in &list {
            if row
                .capabilities
                .iter()
                .any(|c| !crate::app_core::ENDPOINT_CAPABILITIES.contains(&c.as_str()))
            {
                return Err("Unknown endpoint capability".into());
            }
            let (chain_id, slot) = match row.chain_id.split_once(':') {
                Some((chain_id, "secondary")) => (chain_id, EndpointSlot::Secondary),
                Some((chain_id, "explorer")) => (chain_id, EndpointSlot::Explorer),
                _ => (row.chain_id.as_str(), EndpointSlot::Primary),
            };
            if let Some(chain) = Chain::from_str_id(chain_id) {
                for url in &row.endpoints {
                    crate::endpoint_api::validate_configured_endpoint(chain, slot, url)?;
                }
            }
        }

        let mut endpoints = std::collections::HashMap::with_capacity(list.len());
        let mut capabilities = std::collections::HashMap::new();
        for entry in list {
            capabilities.insert(entry.chain_id.clone(), entry.capabilities);
            endpoints.insert(entry.chain_id.clone(), Arc::new(entry.endpoints));
        }
        Ok(Self {
            endpoints,
            capabilities,
        })
    }
}

// ── WalletService — primary UniFFI-exported object ────────────────────────

/// Swift holds one instance for the lifetime of the app session.
#[derive(Clone, uniffi::Object)]
pub struct WalletService {
    transport_cache_dir: Arc<parking_lot::Mutex<Option<String>>>,
    pub(crate) send_reviews: Arc<tokio::sync::Mutex<HashMap<String, send_review::ReviewedSend>>>,
    pub(crate) projection_sequence: Arc<std::sync::atomic::AtomicU64>,
    app_refresh_lock: Arc<tokio::sync::Mutex<()>>,
    send_execute_lock: Arc<tokio::sync::Mutex<()>>,
    quote_refresh_lock: Arc<tokio::sync::Mutex<()>>,
    balance_refreshes: Arc<balance_refresh::BalanceRefreshes>,
    pub(crate) trc20_metadata: Arc<crate::fetch::tron_metadata_cache::MetadataCache>,

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
    pub(crate) refresh_clock: Arc<AsyncRwLock<crate::fetch::refresh_policy::RefreshClock>>,
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
            transport_cache_dir: Arc::new(parking_lot::Mutex::new(None)),
            trc20_metadata: Arc::new(crate::fetch::tron_metadata_cache::MetadataCache::default()),
            send_reviews: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            app_refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            send_execute_lock: Arc::new(tokio::sync::Mutex::new(())),
            quote_refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            balance_refreshes: Arc::new(balance_refresh::BalanceRefreshes::default()),
            projection_sequence: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            state_writer: Arc::new(tokio::sync::Mutex::new(())),
            uses_catalog_endpoints: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            endpoints: Arc::new(AsyncRwLock::new(EndpointIndex::from_list(endpoints)?)),
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
    /// Resolve the adapter and URLs from one configuration snapshot.
    /// Unknown custom URLs use the chain's declared default API.
    pub(crate) async fn fetch_endpoints(
        &self,
        chain: Chain,
        required: &[&str],
    ) -> Result<(crate::EndpointApi, Arc<Vec<String>>), SpectraBridgeError> {
        let urls = self.endpoints_for(chain.str_id(), required).await;
        let catalog = crate::app_core::endpoint_catalog()?;
        let api = urls
            .iter()
            .find_map(|url| {
                catalog
                    .endpoint_records
                    .iter()
                    .find(|row| row.chain_id == chain.str_id() && row.endpoint == *url)
                    .and_then(|row| row.api)
            })
            .or_else(|| chain.endpoint_api(EndpointSlot::Primary))
            .ok_or_else(|| format!("No fetch API for {}", chain.str_id()))?;
        Ok((api, urls))
    }

    pub(crate) async fn configured_endpoint_urls(&self, chain_id: &str) -> Arc<Vec<String>> {
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
            let (network_id, slot) = match chain_id.split_once(':') {
                Some((network, "secondary")) => (network, EndpointSlot::Secondary),
                Some((network, "explorer")) => (network, EndpointSlot::Explorer),
                _ => (chain_id, EndpointSlot::Primary),
            };
            if let Some(chain) = Chain::from_str_id(network_id) {
                if let Some(api) = chain.endpoint_api(slot) {
                    let mut custom = self.custom_api_endpoints(chain, api, &[]).await;
                    if !custom.is_empty() {
                        for url in base.iter() {
                            if !custom.contains(url) {
                                custom.push(url.clone());
                            }
                        }
                        return Arc::new(custom);
                    }
                }
            }
        }
        base
    }
}

/// Catalog transport configuration for a non-platform front end.
pub fn catalog_endpoints() -> Result<Vec<ChainEndpoints>, SpectraBridgeError> {
    let mut endpoints = Vec::new();
    for chain in Chain::all() {
        let records = crate::filtered_endpoint_records_for_chain(chain.str_id().into(), 0)?;
        for slot in [
            EndpointSlot::Primary,
            EndpointSlot::Secondary,
            EndpointSlot::Explorer,
        ] {
            let Some(api) = chain.endpoint_api(slot) else {
                continue;
            };
            endpoints.push(ChainEndpoints {
                capabilities: vec![],
                chain_id: chain.endpoint_str_id(slot),
                endpoints: records
                    .iter()
                    .filter(|record| {
                        record.api == Some(api)
                            && (slot != EndpointSlot::Primary
                                || record
                                    .capabilities
                                    .iter()
                                    .any(|c| matches!(c.as_str(), "balance" | "fee" | "broadcast")))
                    })
                    .map(|record| record.endpoint.clone())
                    .collect(),
            });
        }
    }
    Ok(endpoints)
}

#[cfg(test)]
mod a_primary_endpoint_can_serve_a_primary_read {
    use super::*;

    /// Operation URL prefixes and incompatible API families are never passed
    /// to a primary client as base URLs, even when they share a chain.
    #[test]
    fn no_chain_is_offered_an_endpoint_that_answers_none_of_them() {
        let mut checked = 0;
        for row in catalog_endpoints().expect("catalog endpoints") {
            if Chain::from_str_id(&row.chain_id).is_none() {
                continue; // a `:secondary` or `:explorer` slot, not the primary list
            }
            for endpoint in &row.endpoints {
                let chain = Chain::from_str_id(&row.chain_id).unwrap();
                let record = crate::filtered_endpoint_records_for_chain(row.chain_id.clone(), 0)
                    .unwrap()
                    .into_iter()
                    .find(|record| &record.endpoint == endpoint)
                    .unwrap();
                assert_eq!(record.api, chain.endpoint_api(EndpointSlot::Primary));
                assert!(
                    record
                        .capabilities
                        .iter()
                        .any(|c| matches!(c.as_str(), "balance" | "fee" | "broadcast")),
                    "{} lists operation-only URL {endpoint} as a base",
                    row.chain_id
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
    pub async fn update_endpoints(
        &self,
        endpoints: Vec<ChainEndpoints>,
    ) -> Result<(), SpectraBridgeError> {
        let index = EndpointIndex::from_list(endpoints)?;
        self.uses_catalog_endpoints
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let mut guard = self.endpoints.write().await;
        *guard = index;
        Ok(())
    }
}

pub mod app_refresh;
mod owned_send;
pub mod send_review;
mod send_stage_protocols;
mod send_stage_utxo;
mod send_stages;

pub use owned_send::{OwnedReplacementDraft, OwnedSendPreview, OwnedSendQuote};

impl WalletService {
    pub(crate) fn secrets(&self) -> Result<Arc<dyn SecretStore>, SpectraBridgeError> {
        self.secret_store
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| SpectraBridgeError::from("secret store not registered".to_string()))
    }
}

mod monero_wallet;
pub use monero_wallet::MoneroSyncStatus;
