//! The stateful object the shells talk to: `WalletService` owns the resident
//! state, the HTTP client and the endpoint lists, and every long-lived thing
//! core holds on a front end's behalf. Stateless helpers belong beside the
//! domain module that owns them.
//!
//! Split by owner, one module each:
//!
//! | module | owns |
//! |---|---|
//! | [`state`] | `ResidentState` projections and serialized persistence |
//! | [`network`] | endpoint health and transaction status |
//! | `network_balance`, `network_tokens`, `network_history`, `network_prices` | chain reads grouped by responsibility |
//! | [`send_preflight`] | send eligibility, routing and recipient warnings |
//! | [`send_preview`] | fee estimates and send previews |
//! | [`send_stages`] | prepared/signed artifacts and explicit submission |
//! | [`send_broadcast`] | rebroadcast of signed payloads |
//! | [`send_destination`] | resolution and review verification |
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

pub(crate) use crate::api::endpoint::EndpointCapability;
pub(crate) use crate::api::utxo::UtxoTxStatus;
pub(crate) use crate::api::{
    aptos_rest::AptosClient, blockbook::BlockbookClient, evm_json_rpc::EvmClient,
    horizon::HorizonClient, icp_rosetta::IcpClient, insight::InsightClient,
    kaspa_rest::KaspaClient, koios::KoiosClient, near_json_rpc::NearClient,
    solana_json_rpc::SolanaClient, substrate_json_rpc::SubstrateClient, sui_json_rpc::SuiClient,
    toncenter_v2::ToncenterV2Client, tron_http::TronHttpClient, xrpl_json_rpc::XrplClient,
};
pub(crate) use crate::error::SpectraBridgeError;
pub(crate) use crate::registry::Chain;
pub(crate) use crate::secrets::store::SecretStore;
pub(crate) use crate::service::history_store::HistoryPaginationStore;
pub(crate) use crate::store::state::{
    ResidentState, StateCommand, StateTransition, reduce_state_in_place,
};
pub(crate) use crate::store::wallet_domain::AssetHolding;
pub(crate) use crate::store::{TransactionStatusPollConfig, TransactionStatusTrackerState};

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

use crate::api::endpoint::EndpointApi;

pub(crate) mod address_discovery;
pub use address_discovery::WalletAddressDiscovery;
mod balance_refresh;
pub(crate) mod bitcoin_history;
mod diagnostic_state;
pub mod history_decode;
pub mod history_store;
pub mod refresh_engine;
pub mod refresh_policy;
pub use diagnostic_state::{
    ChainDegradation, ConfiguredSelfTestReport, DiagnosticCommand, DiagnosticLog,
    DiagnosticLogInput, DiagnosticLogLevel, DiagnosticState, DiagnosticsPlatformInfo,
};
mod funds_scan;
mod history_cursor;
pub(crate) mod history_derived;
pub use history_derived::ReplaceableSend;
mod history_query;
pub use history_query::{
    EndpointHolder, HISTORY_PAGE_MAX, HISTORY_SMALL_AMOUNT_THRESHOLD, HistoryPage, HistoryQuery,
    HistoryQueryFilter, TransactionEndpoints, TransactionSnapshot, WalletEarliestTransactionDate,
};
mod history_refresh;
pub use history_refresh::{HistoryRefreshOutcome, HistoryWalletDiagnostics};
mod history_operation;
pub use history_operation::HistoryRefreshScope;
mod keypool;
mod maintenance;
mod network;
mod network_balance;
mod network_history;
mod network_prices;
pub use network_prices::QuoteRefreshState;
mod endpoint_directory;
mod network_tokens;
pub use endpoint_directory::CustomEndpoint;
mod operational_events;
mod pending_status;
pub use pending_status::PendingMaintenanceResult;
mod movement;
pub use movement::LargeMovementEvaluation;
mod reset;
pub use reset::{ResetPlan, reset_dispatch};
mod send_broadcast;
mod send_destination;
mod send_execution;
mod send_identity;
mod send_near;
mod send_preflight;
mod send_preview;
mod send_records;
mod staking;
mod staking_icp;
mod staking_protocols;
mod staking_substrate;
mod valuation;
pub use movement::PortfolioMovementBaseline;
pub use valuation::PortfolioValuation;
mod state;
mod transaction_actions;
mod transaction_recheck;
pub use transaction_actions::TransactionActions;
mod transactions;
mod transport;
mod wallet_actions;
pub use wallet_actions::WalletActions;
mod wallet_approvals;
pub use wallet_approvals::TokenApprovals;
mod wallet_coins;
pub use wallet_coins::{AddressBranch, WalletCoins};
mod wallet_copy;
pub use wallet_copy::WalletCopyCommit;
mod wallet_keys;
pub use wallet_keys::WalletKeyKind;
mod wallet_closing;
mod wallet_import;
mod wallet_messages;
mod wallet_near_keys;
pub use wallet_near_keys::NearAccessKeys;
mod wallet_near_storage;
pub use wallet_near_storage::NearStorageDeposits;
mod wallet_network_account;
pub use wallet_network_account::WalletNetworkAccount;
mod wallet_nfts;
pub use wallet_nfts::WalletNfts;
mod wallet_sui_coins;
pub use wallet_sui_coins::SuiCoinType;
mod wallet_token_accounts;
pub use wallet_token_accounts::EmptyTokenAccounts;
mod wallet_trust_lines;
pub use wallet_trust_lines::WalletTrustLines;
pub(crate) mod worker;
mod zcash_shielded;
pub use zcash_shielded::ZcashShieldedStatus;
mod litecoin_mweb;
pub use litecoin_mweb::LitecoinMwebStatus;

use endpoint_directory::EndpointIndex;
pub use endpoint_directory::{ChainEndpoints, catalog_endpoints};
use keypool::keypool_key;
pub use network::EndpointProbe;
pub use network_balance::NativeBalanceSummary;
pub(crate) use network_tokens::MetadataRead;
pub use network_tokens::{TokenBalanceResult, TokenDescriptor};
pub use send_destination::{
    SendDestinationActivity, SendDestinationResolution, SendDestinationRisk,
};
#[cfg(test)]
use send_destination::{resolve_destination, verify_reviewed_destination};
pub use state::{PortfolioHolding, PortfolioSnapshot, WalletDerivedState};
/// The confirmation-poll outcome, which lives with the trackers it updates.
pub use transactions::StatusPollOutcome;
pub(crate) use wallet_actions::WalletAction;
pub use wallet_import::SeedPhraseReveal;

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
    pub(crate) trc20_metadata: Arc<crate::api::tron_metadata_cache::MetadataCache>,
    /// What this service's history runs and endpoint checks found.
    pub(crate) diagnostic_runs: Arc<crate::diagnostics::DiagnosticRuns>,

    /// Serializes persistent mutations, including database binding.
    pub(crate) state_writer: Arc<tokio::sync::Mutex<()>>,
    uses_catalog_endpoints: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) endpoints: Arc<AsyncRwLock<EndpointIndex>>,
    /// Per-wallet history pagination state (cursor / page / exhaustion).
    pub(crate) history_pagination: Arc<HistoryPaginationStore>,
    /// Optional Keychain delegate (set via `set_secret_store`).
    pub(crate) secret_store: Arc<std::sync::RwLock<Option<Arc<dyn SecretStore>>>>,
    /// Canonical in-memory wallet + holdings state.
    pub(crate) wallet_state: Arc<AsyncRwLock<ResidentState>>,
    /// The revision of each state `publish_state` commits — every state
    /// command, import, reset and open, but not a sweep's balances. The refresh
    /// engine follows it to learn that the wallets changed.
    pub(crate) published: Arc<tokio::sync::watch::Sender<u64>>,
    /// Bumped when a published state adds or removes a wallet or changes one
    /// in anything but its balances. Carried by the portfolio snapshot, so a
    /// view keyed on a wallet's name and addresses is not re-read for every
    /// balance a sweep lands.
    pub(crate) wallet_identity_revision: Arc<std::sync::atomic::AtomicU64>,
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
    /// means. Core holds it so every front end, the CLI included, can ask the
    /// scheduling question.
    pub(crate) refresh_clock: Arc<AsyncRwLock<crate::service::refresh_policy::RefreshClock>>,
}
#[uniffi::export]
impl WalletService {
    #[uniffi::constructor]
    pub fn new(endpoints: Vec<ChainEndpoints>) -> Result<Arc<Self>, SpectraBridgeError> {
        // Log to stderr, and stay quiet unless `RUST_LOG` asks: a library
        // writing to stdout would corrupt every `spectra --json` document, and
        // a caller cannot opt out of a `OnceLock`.
        static LOGGING: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        LOGGING.get_or_init(|| {
            use tracing_subscriber::{EnvFilter, fmt};
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
            trc20_metadata: Arc::new(crate::api::tron_metadata_cache::MetadataCache::default()),
            diagnostic_runs: Default::default(),
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
            wallet_state: Arc::new(AsyncRwLock::new(ResidentState::default())),
            published: Arc::new(tokio::sync::watch::Sender::new(0)),
            wallet_identity_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
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

#[cfg(test)]
#[path = "tests/app_boundary.rs"]
mod app_boundary_tests;

#[cfg(test)]
#[path = "tests/loopback_service.rs"]
pub(crate) mod loopback_service;

pub mod app_refresh;
mod owned_send;
pub use owned_send::{OwnedReplacementDraft, OwnedSendPreview};
pub mod send_review;
mod send_stage_aptos;
mod send_stage_cardano;
mod send_stage_evm;
mod send_stage_icp;
mod send_stage_litecoin;
mod send_stage_peercoin;
mod send_stage_protocols;
mod send_stage_solana;
mod send_stage_stellar;
mod send_stage_substrate;
mod send_stage_sui;
mod send_stage_ton;
mod send_stage_tron;
mod send_stage_utxo;
mod send_stage_xrp;
mod send_stages;
mod send_utxo_sources;
mod setup_summary;
pub use setup_summary::{CapabilityCoverage, WalletSetupLimit, WalletSetupSummary};

impl WalletService {
    pub(crate) fn secrets(&self) -> Result<Arc<dyn SecretStore>, SpectraBridgeError> {
        self.secret_store
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| SpectraBridgeError::failure("secret store not registered"))
    }
}

mod monero_wallet;
pub use monero_wallet::{MoneroPaymentProof, MoneroSyncStatus};
mod multisig;
pub use multisig::{MultisigAccount, MultisigSession, MultisigSigner, MultisigSpend};
mod multisig_aptos;
mod multisig_cardano;
mod multisig_psbt;
mod multisig_safe;
mod multisig_stellar;
mod multisig_substrate;
mod multisig_sui;
mod multisig_ton;
mod multisig_tron;
mod multisig_xrp;
