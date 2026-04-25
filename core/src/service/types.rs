use serde::{Deserialize, Serialize};

/// Typed app-settings record for UniFFI — Rust handles JSON serialization to/from SQLite.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAppSettings {
    pub pricing_provider: String,
    pub selected_fiat_currency: String,
    pub fiat_rate_provider: String,
    #[serde(rename = "ethereumRPCEndpoint")]
    pub ethereum_rpc_endpoint: String,
    pub ethereum_network_mode: String,
    #[serde(rename = "etherscanAPIKey")]
    pub etherscan_api_key: String,
    #[serde(rename = "moneroBackendBaseURL")]
    pub monero_backend_base_url: String,
    #[serde(rename = "moneroBackendAPIKey")]
    pub monero_backend_api_key: String,
    pub bitcoin_network_mode: String,
    pub dogecoin_network_mode: String,
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
    pub pinned_dashboard_asset_symbols: Vec<String>,
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

/// Endpoint configuration passed in from Swift at construction time and
/// rebuilt via `update_endpoints_typed`.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct ChainEndpoints {
    pub chain_id: u32,
    pub endpoints: Vec<String>,
    /// Optional API key for services that require one (Blockfrost, Subscan, etc.).
    pub api_key: Option<String>,
}
