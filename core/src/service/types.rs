//! The records and enums `WalletService` hands across the FFI.
//!
//! A type belongs here when it is the *shape of an answer*; the per-chain send
//! parameter records — the shape of a *request* — live in
//! [`super::send_params`].

use super::*;

/// Everything the wallet list implies, with holdings already resolved.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletDerivedState {
    pub included_portfolio_holdings: Vec<crate::store::wallet_domain::CoreCoin>,
    pub unique_price_request_coins: Vec<crate::store::wallet_domain::CoreCoin>,
    /// One entry per asset, amounts summed across wallets.
    pub portfolio: Vec<crate::store::wallet_domain::CoreCoin>,
    pub send_coins_by_wallet_id: HashMap<String, Vec<crate::store::wallet_domain::CoreCoin>>,
    pub receive_coins_by_wallet_id: HashMap<String, Vec<crate::store::wallet_domain::CoreCoin>>,
    pub receive_chains_by_wallet_id: HashMap<String, Vec<String>>,
    pub send_enabled_wallet_ids: Vec<String>,
    pub receive_enabled_wallet_ids: Vec<String>,
    pub refreshable_chain_names: Vec<String>,
    pub signing_material_wallet_ids: Vec<String>,
    pub private_key_backed_wallet_ids: Vec<String>,
}
/// The platform-owned settings blob.
///
/// Anything every front end must agree on lives in `AppSettings` instead —
/// the fiat currency, the pinned dashboard, and the selected network, which
/// used to be three `*_network_mode` strings here and is now one core-owned
/// chain id per family.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAppSettings {
    pub pricing_provider: String,
    pub selected_fiat_currency: String,
    pub fiat_rate_provider: String,
    #[serde(rename = "ethereumRPCEndpoint")]
    pub ethereum_rpc_endpoint: String,
    #[serde(rename = "etherscanAPIKey")]
    pub etherscan_api_key: String,
    #[serde(rename = "moneroBackendBaseURL")]
    pub monero_backend_base_url: String,
    #[serde(rename = "moneroBackendAPIKey")]
    pub monero_backend_api_key: String,
    pub bitcoin_esplora_endpoints: String,
    pub bitcoin_stop_gap: i32,
    pub bitcoin_fee_priority: String,
    pub dogecoin_fee_priority: String,
    pub hide_balances: bool,
    #[serde(rename = "useFaceID")]
    pub use_face_id: bool,
    pub use_auto_lock: bool,
    #[serde(rename = "useStrictRPCOnly")]
    pub use_strict_rpc_only: bool,
    pub require_biometric_for_send_actions: bool,
    pub use_price_alerts: bool,
    pub use_transaction_status_notifications: bool,
    pub use_large_movement_notifications: bool,
    pub automatic_refresh_frequency_minutes: i32,
    pub background_sync_profile: String,
    pub large_movement_alert_percent_threshold: f64,
    #[serde(rename = "largeMovementAlertUSDThreshold")]
    pub large_movement_alert_usd_threshold: f64,
}

/// Token descriptor passed across UniFFI without JSON-shuttle marshalling.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenDescriptor {
    pub contract: String,
    pub symbol: String,
    pub decimals: u8,
    pub name: Option<String>,
}

/// Typed token-balance result returned via UniFFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenBalanceResult {
    pub contract_address: String,
    pub symbol: String,
    pub decimals: u8,
    pub balance_raw: String,
    pub balance_display: String,
}

/// Unified per-chain native balance projection used by `fetch_native_balance_summary`.
/// `smallest_unit` is a base-10 integer string (sats, lamports, wei, yocto-NEAR, …);
/// `amount_display` is the chain's human-readable native amount.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NativeBalanceSummary {
    pub smallest_unit: String,
    pub amount_display: String,
    pub utxo_count: u32,
}

/// EVM-address probe output. Used by chain-risk warnings to decide whether a
/// destination looks "fresh" (zero balance + zero nonce).
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmAddressProbe {
    pub nonce: i64,
    pub balance_eth: f64,
}

/// What a [`TransactionCommand`] changed, by id.
///
/// Deliberately not the resulting list: history is unbounded, so a command that
/// returned it would make every write cost the size of the whole store.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct TransactionChange {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
}

impl TransactionChange {
    /// True when the store is unchanged, so a caller can skip re-reading.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}

/// Mutations of the transaction store.
///
/// `Upsert` covers recording a send, merging a fetched history page, and
/// updating a status — in every case the caller supplies the record it wants
/// stored, and core works out whether that is an addition or an update.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum TransactionCommand {
    Upsert {
        records: Vec<crate::store::persistence_models::CorePersistedTransactionRecord>,
    },
    /// Merge a freshly fetched page into what is already stored.
    ///
    /// Core reads its own records to merge against, so only the incoming page
    /// crosses the FFI. Previously a front end shipped its entire history over,
    /// received the merged list back, and diffed it — the whole history moved
    /// three times per refresh.
    /// Merge freshly fetched history for a chain into what is stored.
    ///
    /// The merge strategy is not a parameter: it is a property of the chain and
    /// comes from `registry::Chain`. Callers used to pass it, which is how a
    /// chain could be wired to the wrong one.
    Merge {
        incoming: Vec<crate::fetch::transactions::CoreTransactionRecord>,
        chain_name: String,
        preserve_created_at_sentinel_unix: Option<f64>,
    },
    Remove {
        ids: Vec<String>,
    },
    RemoveForWallet {
        wallet_id: String,
    },
    Clear,
}

/// Endpoint configuration passed in from Swift at construction time and
/// rebuilt via `update_endpoints_typed`.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct ChainEndpoints {
    pub chain_id: String,
    pub endpoints: Vec<String>,
    /// Optional API key for services that require one (Blockfrost, Subscan, etc.).
    pub api_key: Option<String>,
}

// Per-chain send parameter records live in `super::send_params`.
