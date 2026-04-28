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

// ── Per-chain `sign_and_send` parameter shapes ────────────────────────────
//
// Each chain's `sign_and_send` arm in `service::mod` historically read its
// inputs by pulling individual fields out of a `serde_json::Value` with
// inline `.as_str()` / `.as_u64()` / `try_into()` chains. That style hides
// the contract — a reader can't see at a glance what shape the Polkadot
// endpoint expects without scanning the full arm body.
//
// Defining a typed struct per chain reverses that: the type doc *is* the
// API contract, serde gives field-name-aware error messages for free, and
// the dispatch arm collapses to one `parse_params` call.
//
// These structs accept the same JSON shape Swift already produces so this
// migration is internal — no FFI signature changes.

/// `Chain::Polkadot` send parameters. `planck` is the smallest unit
/// (10⁻¹⁰ DOT). The 32-byte `private_key_hex` is the sr25519 mini-secret
/// produced by `derive_polkadot`, *not* a 64-byte ed25519 secret.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PolkadotSendParams {
    pub from: String,
    pub to: String,
    /// Accepts either a JSON string ("12500000000") or a JSON number for
    /// backward compatibility with Swift call sites that emitted both forms.
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub planck: u128,
    pub private_key_hex: String,
    pub public_key_hex: String,
}

/// `Chain::Bittensor` send parameters. `rao` is the smallest unit
/// (10⁻⁹ TAO). Same sr25519 32-byte mini-secret rules as Polkadot.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BittensorSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub rao: u128,
    pub private_key_hex: String,
    pub public_key_hex: String,
}

/// Accepts JSON `"12345"` or `12345` for u128 fields. Swift sends planck
/// values as strings (since u128 doesn't round-trip safely through JSON
/// numbers) but legacy call sites emitted them as `as_u64`-able numbers.
fn deserialize_u128_from_string_or_number<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(s) = value.as_str() {
        return s.parse::<u128>().map_err(D::Error::custom);
    }
    if let Some(n) = value.as_u64() {
        return Ok(n as u128);
    }
    Err(D::Error::custom("expected u128 as string or number"))
}
