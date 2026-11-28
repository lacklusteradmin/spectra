//! The user's settings: what each holds, its defaults and bounds, and the
//! commands that change them.

use crate::api::endpoint::EndpointCapability;
use serde::{Deserialize, Serialize};

use super::state::{ResidentState, StateCommand, StateEvent};
use super::token_preferences::MAX_TOKEN_DECIMALS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Settings that are part of the domain — every front end must agree on them,
/// and losing one on restart would be a bug.
///
/// Presentation preferences (theme, which rows are pinned, diagnostic
/// verbosity) are *not* domain state and stay on the platform. Do not add a
/// field here that only one front end reads.
pub struct AppSettings {
    pub custom_endpoints: Vec<crate::service::CustomEndpoint>,
    /// Networks whose reads and sends use the user's endpoints alone; the
    /// catalog's are not contacted.
    pub custom_endpoints_only: Vec<crate::registry::Chain>,
    /// The currency amounts are displayed in.
    pub fiat_currency: FiatCurrency,
    /// Token IDs pinned in display order. An empty list means no pins.
    /// Defaults are applied only when settings or this field are initialized.
    pub pinned_dashboard_token_ids: Vec<String>,

    // Tor routing preference. The platform manages the client lifecycle
    // and provides its writable directory.
    pub tor_enabled: bool,
    /// Use a SOCKS5 proxy the user runs (Orbot) instead of the embedded client.
    pub tor_use_custom_proxy: bool,
    /// Where that proxy is. Validated on write, so a value that cannot be a
    /// SOCKS5 endpoint is never stored and cannot be handed to the HTTP layer.
    pub tor_custom_proxy_address: String,

    // ── Alerting ──────────────────────────────────────────────────────────
    pub use_price_alerts: bool,
    pub use_transaction_status_notifications: bool,
    pub use_large_movement_notifications: bool,
    pub large_movement_alert_percent_threshold: f64,
    pub large_movement_alert_usd_threshold: f64,
}

// Bounds live here rather than in a front end's `didSet`, which is where they
// were: the reducer bounds values so no caller can store an out-of-range
// movement threshold.
pub const LARGE_MOVEMENT_PERCENT_RANGE: std::ops::RangeInclusive<f64> = 1.0..=90.0;

pub const LARGE_MOVEMENT_USD_RANGE: std::ops::RangeInclusive<f64> = 1.0..=100_000.0;

/// The bounds the reducer holds edits to, for the controls that set them.
/// Only bounds a front end's control reads are here; the reducer enforces
/// every bound whether or not a control knows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct InputBounds {
    /// The most decimal places a custom token may declare.
    pub max_token_decimals: u32,
    /// The large-movement alert threshold, in percent.
    pub large_movement_percent_min: f64,
    pub large_movement_percent_max: f64,
}

#[uniffi::export]
pub fn input_bounds() -> InputBounds {
    InputBounds {
        max_token_decimals: MAX_TOKEN_DECIMALS,
        large_movement_percent_min: *LARGE_MOVEMENT_PERCENT_RANGE.start(),
        large_movement_percent_max: *LARGE_MOVEMENT_PERCENT_RANGE.end(),
    }
}

/// A part of the app's data a reset can clear.
///
/// Strings before: `reset_data` checked them against a list and the platform
/// kept its own enum with the same names as raw values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ResetScope {
    WalletsAndSecrets,
    HistoryAndCache,
    AlertsAndContacts,
    SettingsAndEndpoints,
    DashboardCustomization,
}

impl ResetScope {
    pub const ALL: [ResetScope; 5] = [
        Self::WalletsAndSecrets,
        Self::HistoryAndCache,
        Self::AlertsAndContacts,
        Self::SettingsAndEndpoints,
        Self::DashboardCustomization,
    ];

    pub fn as_raw(self) -> &'static str {
        match self {
            Self::WalletsAndSecrets => "walletsAndSecrets",
            Self::HistoryAndCache => "historyAndCache",
            Self::AlertsAndContacts => "alertsAndContacts",
            Self::SettingsAndEndpoints => "settingsAndEndpoints",
            Self::DashboardCustomization => "dashboardCustomization",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|scope| scope.as_raw() == raw.trim())
    }
}

fn default_true() -> bool {
    true
}

/// Orbot's SOCKS5 port, which is what a user running their own proxy on a
/// phone almost always has.
fn default_tor_custom_proxy_address() -> String {
    "socks5://127.0.0.1:9150".to_string()
}

/// A SOCKS5 endpoint this app can actually hand to the HTTP layer: a
/// `socks5://` or `socks5h://` URL with a host and a port.
///
/// Validated here rather than at the toggle: a front end that stored
/// "127.0.0.1:9150" or a typo would leave Tor enabled and every request going
/// out of a proxy that cannot be built. The HTTP layer fails closed on an
/// unusable proxy, so the visible result is that nothing loads and nothing
/// says why.
pub(crate) fn parsed_socks5_proxy(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("socks5://")
        .or_else(|| trimmed.strip_prefix("socks5h://"))?;
    let (host, port) = rest.rsplit_once(':')?;
    if host.is_empty() || host.contains('/') {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    if port == 0 {
        return None;
    }
    Some(trimmed.to_string())
}

fn default_large_movement_percent() -> f64 {
    10.0
}

fn default_large_movement_usd() -> f64 {
    50.0
}

/// The display currencies this app quotes in.
///
/// The codes are the domain's, so they are here: the reducer cannot be handed
/// a currency that does not exist, and parsing a typed code is the front
/// end's job.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "UPPERCASE")]
pub enum FiatCurrency {
    #[default]
    Usd,
    Eur,
    Gbp,
    Jpy,
    Cny,
    Inr,
    Cad,
    Aud,
    Chf,
    Brl,
    Sgd,
    Aed,
}

impl FiatCurrency {
    pub const ALL: [FiatCurrency; 12] = [
        Self::Usd,
        Self::Eur,
        Self::Gbp,
        Self::Jpy,
        Self::Cny,
        Self::Inr,
        Self::Cad,
        Self::Aud,
        Self::Chf,
        Self::Brl,
        Self::Sgd,
        Self::Aed,
    ];

    /// The ISO 4217 code, which is what rate tables key on.
    pub fn code(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Eur => "EUR",
            Self::Gbp => "GBP",
            Self::Jpy => "JPY",
            Self::Cny => "CNY",
            Self::Inr => "INR",
            Self::Cad => "CAD",
            Self::Aud => "AUD",
            Self::Chf => "CHF",
            Self::Brl => "BRL",
            Self::Sgd => "SGD",
            Self::Aed => "AED",
        }
    }

    /// A typed code, trimmed and in any case, or `None` for one not quoted.
    pub fn from_code(code: &str) -> Option<Self> {
        let code = code.trim().to_uppercase();
        Self::ALL
            .into_iter()
            .find(|currency| currency.code() == code)
    }
}

/// Every reset scope, in the order a settings page lists them.
#[uniffi::export]
pub fn reset_scopes() -> Vec<ResetScope> {
    ResetScope::ALL.to_vec()
}

pub fn fiat_currency_codes() -> Vec<String> {
    FiatCurrency::ALL
        .iter()
        .map(|currency| currency.code().to_string())
        .collect()
}

/// What a dashboard pins before the user has pinned anything: a product
/// default every front end has to agree on.
pub const DEFAULT_PINNED_DASHBOARD_ASSETS: [&str; 4] =
    ["bitcoin", "ethereum", "tether", "usd-coin"];

fn default_pinned_dashboard_assets() -> Vec<String> {
    DEFAULT_PINNED_DASHBOARD_ASSETS
        .iter()
        .map(|id| id.to_string())
        .collect()
}

impl AppSettings {
    /// The exact saved selection, including an intentionally empty list.
    pub fn pinned_dashboard_assets(&self) -> Vec<String> {
        self.pinned_dashboard_token_ids.clone()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            fiat_currency: FiatCurrency::Usd,
            pinned_dashboard_token_ids: default_pinned_dashboard_assets(),
            custom_endpoints: Vec::new(),
            custom_endpoints_only: Vec::new(),
            use_price_alerts: default_true(),
            use_transaction_status_notifications: default_true(),
            use_large_movement_notifications: default_true(),
            large_movement_alert_percent_threshold: default_large_movement_percent(),
            large_movement_alert_usd_threshold: default_large_movement_usd(),
            tor_enabled: false,
            tor_use_custom_proxy: false,
            tor_custom_proxy_address: default_tor_custom_proxy_address(),
        }
    }
}

/// One settings field and its new value. Field-level updates avoid
/// overwriting unrelated settings with a caller's stale snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Enum)]
#[serde(tag = "field", rename_all = "camelCase")]
pub enum AppSettingUpdate {
    /// The display currency. Core's refresh engine values everything again
    /// in it, fetching rates if they are due.
    FiatCurrency {
        value: FiatCurrency,
    },
    AddCustomEndpoint {
        capabilities: Vec<EndpointCapability>,
        chain_id: crate::registry::Chain,
        api: crate::api::endpoint::EndpointApi,
        endpoint: String,
    },
    /// Remove one of the user's endpoints. Removing a network's last one
    /// also stops using only the user's endpoints there, which would leave
    /// the network with none to ask.
    RemoveCustomEndpoint {
        chain_id: crate::registry::Chain,
        endpoint: String,
    },
    /// Change one of the user's endpoints in place — its address, its API or
    /// what it is asked for — keeping its network and its place in the list.
    /// One update, so editing a network's only endpoint never leaves it,
    /// even for a moment, with none and "only mine" switched off.
    ReplaceCustomEndpoint {
        chain_id: crate::registry::Chain,
        /// The saved endpoint being changed.
        endpoint: String,
        api: crate::api::endpoint::EndpointApi,
        new_endpoint: String,
        capabilities: Vec<EndpointCapability>,
    },
    /// Use only the user's endpoints on a network, or the catalog's too.
    CustomEndpointsOnly {
        chain_id: crate::registry::Chain,
        value: bool,
    },
    UsePriceAlerts {
        value: bool,
    },
    UseTransactionStatusNotifications {
        value: bool,
    },
    UseLargeMovementNotifications {
        value: bool,
    },
    LargeMovementAlertPercentThreshold {
        value: f64,
    },
    LargeMovementAlertUsdThreshold {
        value: f64,
    },
    TorEnabled {
        value: bool,
    },
    TorUseCustomProxy {
        value: bool,
    },
    /// An empty value restores the default port. A value that is not a
    /// SOCKS5 URL is refused and nothing is stored.
    TorCustomProxyAddress {
        value: String,
    },
}

/// Apply a state command in place, returning only the events.
/// The settings a fresh install starts with.
#[uniffi::export]
pub fn app_settings_defaults() -> AppSettings {
    AppSettings::default()
}

/// `settings` with `update` applied by the reducer's own rule, or unchanged
/// when the rule refuses it.
///
/// For a front end to show an edit before the command that stores it returns,
/// without copying core's trims and clamps.
#[uniffi::export]
pub fn app_settings_applying(settings: AppSettings, update: AppSettingUpdate) -> AppSettings {
    let mut settings = settings;
    apply_app_setting(&mut settings, update);
    settings
}

/// Apply one settings update, trimming strings and bounding numbers.
/// Unknown chains and invalid SOCKS5 URLs leave state unchanged;
/// `false` tells the caller to emit a refusal event.
fn apply_app_setting(settings: &mut AppSettings, update: AppSettingUpdate) -> bool {
    fn clamp<T: PartialOrd>(value: T, range: std::ops::RangeInclusive<T>) -> T {
        let (low, high) = range.into_inner();
        if value < low {
            low
        } else if value > high {
            high
        } else {
            value
        }
    }
    match update {
        AppSettingUpdate::FiatCurrency { value } => settings.fiat_currency = value,
        AppSettingUpdate::AddCustomEndpoint {
            capabilities,
            chain_id,
            api,
            endpoint,
        } => {
            let Ok(endpoint) =
                crate::service::CustomEndpoint::validated(chain_id, api, endpoint, capabilities)
            else {
                return false;
            };
            if settings.custom_endpoints.iter().any(|saved| {
                saved.chain_id == endpoint.chain_id
                    && saved.api == endpoint.api
                    && saved.endpoint == endpoint.endpoint
            }) {
                return false;
            }
            settings.custom_endpoints.insert(0, endpoint);
        }
        AppSettingUpdate::RemoveCustomEndpoint { chain_id, endpoint } => {
            settings
                .custom_endpoints
                .retain(|saved| !(saved.chain_id == chain_id && saved.endpoint == endpoint));
            if !settings
                .custom_endpoints
                .iter()
                .any(|saved| saved.chain_id == chain_id)
            {
                settings
                    .custom_endpoints_only
                    .retain(|chain| *chain != chain_id);
            }
        }
        AppSettingUpdate::ReplaceCustomEndpoint {
            chain_id,
            endpoint,
            api,
            new_endpoint,
            capabilities,
        } => {
            let Some(index) = settings
                .custom_endpoints
                .iter()
                .position(|saved| saved.chain_id == chain_id && saved.endpoint == endpoint)
            else {
                return false;
            };
            let Ok(replacement) = crate::service::CustomEndpoint::validated(
                chain_id,
                api,
                new_endpoint,
                capabilities,
            ) else {
                return false;
            };
            // Another saved endpoint already says the same; the one edited
            // may of course say what it said before.
            if settings
                .custom_endpoints
                .iter()
                .enumerate()
                .any(|(other, saved)| {
                    other != index
                        && saved.chain_id == replacement.chain_id
                        && saved.api == replacement.api
                        && saved.endpoint == replacement.endpoint
                })
            {
                return false;
            }
            settings.custom_endpoints[index] = replacement;
        }
        AppSettingUpdate::CustomEndpointsOnly { chain_id, value } => {
            settings
                .custom_endpoints_only
                .retain(|chain| *chain != chain_id);
            if value {
                settings.custom_endpoints_only.push(chain_id);
            }
        }
        AppSettingUpdate::UsePriceAlerts { value } => settings.use_price_alerts = value,
        AppSettingUpdate::UseTransactionStatusNotifications { value } => {
            settings.use_transaction_status_notifications = value
        }
        AppSettingUpdate::UseLargeMovementNotifications { value } => {
            settings.use_large_movement_notifications = value
        }
        AppSettingUpdate::TorEnabled { value } => settings.tor_enabled = value,
        AppSettingUpdate::TorUseCustomProxy { value } => settings.tor_use_custom_proxy = value,
        AppSettingUpdate::TorCustomProxyAddress { value } => {
            if value.trim().is_empty() {
                settings.tor_custom_proxy_address = default_tor_custom_proxy_address();
            } else {
                let Some(parsed) = parsed_socks5_proxy(&value) else {
                    return false;
                };
                settings.tor_custom_proxy_address = parsed;
            }
        }
        AppSettingUpdate::LargeMovementAlertPercentThreshold { value } => {
            settings.large_movement_alert_percent_threshold =
                clamp(value, LARGE_MOVEMENT_PERCENT_RANGE)
        }
        AppSettingUpdate::LargeMovementAlertUsdThreshold { value } => {
            settings.large_movement_alert_usd_threshold = clamp(value, LARGE_MOVEMENT_USD_RANGE)
        }
    }
    true
}

/// Store a pin set: trimmed, de-duplicated with the first occurrence winning,
/// so display order is the order the assets were pinned in.
fn set_pinned_dashboard_assets(
    state: &mut ResidentState,
    token_ids: Vec<String>,
    events: &mut Vec<StateEvent>,
) {
    let mut seen = std::collections::HashSet::new();
    let normalized: Vec<String> = token_ids
        .into_iter()
        .filter_map(|id| {
            let id = id.trim().to_string();
            (!id.is_empty() && seen.insert(id.clone())).then_some(id)
        })
        .collect();
    if normalized != state.settings.pinned_dashboard_token_ids {
        state.settings.pinned_dashboard_token_ids = normalized;
        events.push(StateEvent::PinnedDashboardAssetsChanged);
    }
}

/// Apply a settings command; `reduce_state_in_place` routes only these here.
pub(super) fn reduce(
    state: &mut ResidentState,
    command: StateCommand,
    events: &mut Vec<StateEvent>,
) {
    match command {
        StateCommand::SetAppSetting { update } => {
            let before = state.settings.clone();
            let accepted = apply_app_setting(&mut state.settings, update);
            if !accepted {
                events.push(StateEvent::AppSettingRejected);
            } else if state.settings != before {
                events.push(StateEvent::AppSettingChanged);
            }
        }
        StateCommand::ResetAppSettings => {
            let before = std::mem::take(&mut state.settings);
            if state.settings != before {
                events.push(StateEvent::AppSettingChanged);
            }
        }
        StateCommand::ResetPinnedDashboardAssets => {
            set_pinned_dashboard_assets(state, default_pinned_dashboard_assets(), events)
        }
        StateCommand::SetDashboardAssetPinned {
            token_id,
            is_pinned,
        } => {
            let token_id = token_id.trim().to_string();
            let mut token_ids = state.settings.pinned_dashboard_assets();
            token_ids.retain(|id| *id != token_id);
            if is_pinned {
                token_ids.push(token_id);
            }
            set_pinned_dashboard_assets(state, token_ids, events)
        }
        _ => unreachable!("routed by reduce_state_in_place"),
    }
}
