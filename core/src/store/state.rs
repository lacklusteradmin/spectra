use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletAddress {
    pub chain_name: String,
    pub address: String,
    pub kind: String,
    pub derivation_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletState {
    pub id: String,
    pub name: String,
    pub is_watch_only: bool,
    pub chain_name: String,
    pub include_in_portfolio_total: bool,
    pub network_id: String,
    pub xpub: Option<String>,
    #[serde(default)]
    pub derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset,
    /// The single path this wallet derives from. A wallet belongs to one chain,
    /// so it needs one path — not the whole per-chain table.
    pub derivation_path: Option<String>,
    /// Power-user derivation overrides, if the wallet was imported with any.
    pub derivation_overrides: crate::store::wallet_domain::CoreWalletDerivationOverrides,
    pub holdings: Vec<crate::store::wallet_domain::AssetHolding>,
    pub addresses: Vec<WalletAddress>,
}

// Plain `impl` — deliberately not `#[uniffi::export]`. These are Rust-side
// domain helpers; the FFI surface stays the record's fields.
impl WalletState {
    /// Build a summary for a wallet with one address on one chain.
    ///
    /// This is the shape most chains produce: a single derived address, no
    /// xpub, no network-mode variants. Multi-address wallets (Bitcoin and the
    /// other UTXO chains) push into `addresses` instead.
    ///
    /// A test constructor: production wallets are built by the import and
    /// derivation paths, which fill in the fields this shortcut defaults.
    #[cfg(test)]
    pub fn single_address(
        id: impl Into<String>,
        name: impl Into<String>,
        chain_name: impl Into<String>,
        address: impl Into<String>,
        derivation_path: Option<String>,
        is_watch_only: bool,
    ) -> Self {
        let chain_name = chain_name.into();
        Self {
            id: id.into(),
            name: name.into(),
            is_watch_only,
            chain_name: chain_name.clone(),
            include_in_portfolio_total: true,
            network_id: crate::registry::Chain::from_display_name(&chain_name)
                .map(|c| c.str_id().to_string())
                .unwrap_or_default(),
            xpub: None,
            derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
            derivation_path: derivation_path.clone(),
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: vec![WalletAddress {
                chain_name,
                address: address.into(),
                kind: "receive".to_string(),
                derivation_path,
            }],
        }
    }

    /// The address to show and query for this wallet.
    ///
    /// Prefers the first `"receive"` address and falls back to the first
    /// address of any kind, so a wallet whose addresses were built by a path
    /// that doesn't classify them still resolves. `None` only when the wallet
    /// has no addresses at all.
    pub fn primary_address(&self) -> Option<&str> {
        self.addresses
            .iter()
            .find(|a| a.kind == "receive")
            .or_else(|| self.addresses.first())
            .map(|a| a.address.as_str())
    }

    /// This wallet's address on a chain, or `None` if it has none there.
    ///
    /// Compares address *slots*, not names, which is what the slot is for: one
    /// derived secp256k1 address serves every EVM chain, so a wallet on
    /// Ethereum resolves an address for Arbitrum and a name comparison would
    /// say it does not.
    /// The network this wallet is on for its own family: the one it recorded
    /// at import if that is still a network of the family, otherwise whatever
    /// the app is set to.
    ///
    /// One rule, read by everything that fetches for a wallet — balances,
    /// history, diagnostics. Each of them used to work it out again, and they
    /// did not agree.
    pub fn network_chain(&self, _settings: &AppSettings) -> Option<crate::registry::Chain> {
        let chain = crate::registry::Chain::from_display_name(&self.chain_name)?;
        crate::registry::Chain::from_str_id(&self.network_id)
            .filter(|selected| selected.mainnet_counterpart() == chain.mainnet_counterpart())
    }

    /// This wallet's address on the network it is on, falling back to its own
    /// chain's slot.
    pub fn active_address(&self, settings: &AppSettings) -> Option<&str> {
        let chain = crate::registry::Chain::from_display_name(&self.chain_name)?;
        self.network_chain(settings)
            .and_then(|network| self.address_on(network))
            .or_else(|| self.address_on(chain))
    }

    pub fn address_on(&self, chain: crate::registry::Chain) -> Option<&str> {
        let slot = chain.address_slot();
        self.addresses
            .iter()
            .find(|a| {
                crate::registry::Chain::from_display_name(&a.chain_name)
                    .is_some_and(|stored| stored.address_slot() == slot)
            })
            .map(|a| a.address.as_str())
    }
}

/// A saved recipient.
///
/// `address` is stored already normalized for its chain, so comparisons are a
/// case-insensitive string match rather than a per-chain rule at every call
/// site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AddressBookEntry {
    pub id: String,
    pub name: String,
    pub chain_name: String,
    pub address: String,
    pub note: String,
}

/// Why an address-book entry was refused. Front ends map these to their own
/// wording; the decision itself is core's.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AddressBookRejection {
    EmptyName,
    InvalidAddress,
    DuplicateAddress,
}

/// How soon a chain's fee should get a transaction confirmed.
///
/// The three a fee picker offers. This was a free string in
/// [`AppSettings::fee_priority_by_chain`] and a second enum in the app, so
/// "which values exist" had two answers and the app's was the typed one.
/// Front ends name them; which ones exist is core's.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum FeePriority {
    Economy,
    Normal,
    Priority,
}

impl FeePriority {
    /// The stored spelling, and what a provider fee preview is asked for.
    pub fn as_raw(self) -> &'static str {
        match self {
            Self::Economy => "economy",
            Self::Normal => "normal",
            Self::Priority => "priority",
        }
    }
}

/// Read a fee priority written as text — a stored value, a CLI argument.
///
/// Anything the three do not name is the default rather than a stored value no
/// send path knows how to spend.
#[uniffi::export]
pub fn parse_fee_priority(raw: String) -> FeePriority {
    match raw.trim().to_ascii_lowercase().as_str() {
        "economy" => FeePriority::Economy,
        "priority" => FeePriority::Priority,
        _ => FeePriority::Normal,
    }
}

/// Why a token-preference change was refused. Front ends map these to their
/// own wording; the decision itself is core's.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum TokenPreferenceRejection {
    /// The chain does not host tokens at all.
    UnknownChain,
    EmptySymbol,
    /// Longer than a symbol is ever spelled; almost always a pasted name.
    SymbolTooLong,
    EmptyName,
    EmptyContract,
    /// Not a well-formed contract for the chain that would host it.
    InvalidContract,
    /// The chain already has a row for this contract.
    DuplicateToken,
    /// More places than any token has. A clamp here would silently read a
    /// balance at the wrong scale.
    TooManyDecimals,
    /// The catalog ships it, so it is not the user's to edit or remove.
    BuiltInToken,
    /// No row for that chain and contract.
    UnknownToken,
}

/// A token preference addressed by what it actually is, rather than by an id
/// two front ends have to spell the same way.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreTokenPreferenceKey {
    pub chain_name: String,
    pub contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
/// Settings that are part of the domain — every front end must agree on them,
/// and losing one on restart would be a bug.
///
/// Presentation preferences (theme, which rows are pinned, diagnostic
/// verbosity) are *not* domain state and stay on the platform. Do not add a
/// field here that only one front end reads.
///
/// Eighteen of these fields arrived from `PersistedAppSettings`, a
/// twenty-three field blob iOS loaded whole at launch and wrote back whole on
/// every change. Four of that blob's fields stayed on iOS, where they belong —
/// hiding balances, Face ID, auto-lock and biometric-gated sends are one front
/// end's presentation and one platform's capability. The rest decide what gets
/// fetched, what a send costs and when an alert fires, and the CLI had no way
/// to read or set any of them.
pub struct AppSettings {
    /// The currency amounts are displayed in.
    #[serde(default)]
    pub fiat_currency: FiatCurrency,
    /// Token IDs pinned in display order. An empty list means no pins.
    /// Defaults are applied only when settings or this field are initialized.
    #[serde(default = "default_pinned_dashboard_assets")]
    pub pinned_dashboard_token_ids: Vec<String>,
    /// Which network the user selected for each chain family that offers a
    /// choice, as `mainnet str_id -> selected str_id`.
    ///
    /// Absent means mainnet, so the map is empty for most users. One field
    /// rather than one per family: the three that had a choice were three
    /// settings, three enums and three hand-written pricing cases, and adding
    /// a fourth meant touching all of them.
    #[serde(default)]
    pub network_chain_by_family: std::collections::HashMap<String, String>,

    // ── Providers ─────────────────────────────────────────────────────────
    /// Which price source to quote from.
    /// Which source to take fiat cross-rates from.

    // ── Endpoints and credentials ─────────────────────────────────────────
    /// Custom RPC per chain, as `chain display name -> url`. Absent means the
    /// catalog's list.
    //
    /// One field rather than one per chain: this was `ethereum_rpc_endpoint`,
    /// a single String, and the Swift accessor that read it was
    /// `chainName == "Ethereum" ? … : nil` — so twenty-two of the twenty-three
    /// EVM mainnets could not be pointed at a private node at all.
    #[serde(default)]
    pub rpc_endpoint_by_chain: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub etherscan_api_key: String,
    #[serde(default)]
    pub monero_backend_base_url: String,
    #[serde(default)]
    pub monero_backend_api_key: String,
    /// Custom Esplora bases, comma/semicolon/newline separated, or empty.
    #[serde(default)]
    pub bitcoin_esplora_endpoints: String,
    /// How far past the last used address HD discovery keeps looking.
    #[serde(default = "default_bitcoin_stop_gap")]
    pub bitcoin_stop_gap: u32,

    // ── Fees ──────────────────────────────────────────────────────────────
    /// Confirmation preference per chain, as `chain display name -> one of
    /// "economy" / "normal" / "priority"`. Absent means `normal`, so the map
    /// is empty until the user picks something.
    //
    /// One field rather than one per chain: Bitcoin and Dogecoin each had
    /// their own settings field and their own Swift enum, while the other
    /// seventy-six shared a dictionary iOS persisted itself — three stores for
    /// one preference, and the front ends disagreed about which was canonical.
    #[serde(default, deserialize_with = "fee_priorities_read_leniently")]
    pub fee_priority_by_chain: std::collections::HashMap<String, FeePriority>,

    // ── Network and refresh policy ────────────────────────────────────────
    /// Refuse endpoints the user has not vetted.
    #[serde(default)]
    pub use_strict_rpc_only: bool,
    #[serde(default)]
    pub background_sync_profile: BackgroundSyncProfile,

    // ── Tor ───────────────────────────────────────────────────────────────
    /// Route traffic through Tor. The platform starts and stops the client —
    /// the embedded one needs a writable directory only it can name — but
    /// whether Tor is wanted at all is state, and it used to live in one front
    /// end's `UserDefaults`, where no other front end and no test could see it.
    #[serde(default)]
    pub tor_enabled: bool,
    /// Use a SOCKS5 proxy the user runs (Orbot) instead of the embedded client.
    #[serde(default)]
    pub tor_use_custom_proxy: bool,
    /// Where that proxy is. Validated on write, so a value that cannot be a
    /// SOCKS5 endpoint is never stored and cannot be handed to the HTTP layer.
    #[serde(default = "default_tor_custom_proxy_address")]
    pub tor_custom_proxy_address: String,
    /// Refuse network requests while Tor is wanted but not ready, rather than
    /// falling back to a direct connection.
    #[serde(default)]
    pub tor_kill_switch: bool,

    // ── Alerting ──────────────────────────────────────────────────────────
    #[serde(default = "default_true")]
    pub use_price_alerts: bool,
    #[serde(default = "default_true")]
    pub use_transaction_status_notifications: bool,
    #[serde(default = "default_true")]
    pub use_large_movement_notifications: bool,
    #[serde(default = "default_large_movement_percent")]
    pub large_movement_alert_percent_threshold: f64,
    #[serde(default = "default_large_movement_usd")]
    pub large_movement_alert_usd_threshold: f64,
}

// Bounds live here rather than in a front end's `didSet`, which is where they
// were: the reducer bounds values so no caller can store a stop gap of zero
// or an out-of-range movement threshold.
pub const BITCOIN_STOP_GAP_RANGE: std::ops::RangeInclusive<u32> = 1..=200;
pub const LARGE_MOVEMENT_PERCENT_RANGE: std::ops::RangeInclusive<f64> = 1.0..=90.0;
pub const LARGE_MOVEMENT_USD_RANGE: std::ops::RangeInclusive<f64> = 1.0..=100_000.0;

// One notion of "default" per field: `AppSettings::default()` calls these, and
// serde reads them for a field a stored row does not carry. Splitting the two
// is how a row written before `use_price_alerts` existed would have loaded with
// alerts silently off, rather than on as a fresh install has them.
/// Stored priorities, read through [`parse_fee_priority`] so a file written by
/// hand — or by a build that spelled a fourth value — opens rather than
/// refusing to decode.
fn fee_priorities_read_leniently<'de, D>(
    deserializer: D,
) -> Result<std::collections::HashMap<String, FeePriority>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    Ok(
        std::collections::HashMap::<String, String>::deserialize(deserializer)?
            .into_iter()
            .map(|(chain, value)| (chain, parse_fee_priority(value)))
            .collect(),
    )
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
    ProviderState,
}

impl ResetScope {
    pub const ALL: [ResetScope; 6] = [
        Self::WalletsAndSecrets,
        Self::HistoryAndCache,
        Self::AlertsAndContacts,
        Self::SettingsAndEndpoints,
        Self::DashboardCustomization,
        Self::ProviderState,
    ];

    pub fn as_raw(self) -> &'static str {
        match self {
            Self::WalletsAndSecrets => "walletsAndSecrets",
            Self::HistoryAndCache => "historyAndCache",
            Self::AlertsAndContacts => "alertsAndContacts",
            Self::SettingsAndEndpoints => "settingsAndEndpoints",
            Self::DashboardCustomization => "dashboardCustomization",
            Self::ProviderState => "providerState",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|scope| scope.as_raw() == raw.trim())
    }
}

/// How hard background refresh may work, traded against battery and data.
///
/// A free string before. Any trimmed value was stored, and the policy matched
/// the three names with a wildcard that read everything else — a typo from
/// the command line included — as `aggressive`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundSyncProfile {
    Conservative,
    #[default]
    Balanced,
    Aggressive,
}

impl BackgroundSyncProfile {
    pub const ALL: [BackgroundSyncProfile; 3] =
        [Self::Conservative, Self::Balanced, Self::Aggressive];

    pub fn as_raw(self) -> &'static str {
        match self {
            Self::Conservative => "conservative",
            Self::Balanced => "balanced",
            Self::Aggressive => "aggressive",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        let raw = raw.trim().to_lowercase();
        Self::ALL
            .into_iter()
            .find(|profile| profile.as_raw() == raw)
    }
}
fn default_true() -> bool {
    true
}
fn default_bitcoin_stop_gap() -> u32 {
    10
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

impl AppSettings {
    /// The chain the user is actually on for a family, defaulting to mainnet.
    pub fn network_chain(&self, chain: crate::registry::Chain) -> crate::registry::Chain {
        let family = chain.mainnet_counterpart();
        self.network_chain_by_family
            .get(family.str_id())
            .and_then(|id| crate::registry::Chain::from_str_id(id))
            .filter(|selected| selected.mainnet_counterpart() == family)
            .unwrap_or(family)
    }
}

/// The display currencies this app quotes in.
///
/// The list was a twelve-case Swift enum and nothing else, so
/// `SetFiatCurrency` stored whatever string it was handed — `spectra currency
/// ZZZ` set the display currency to `ZZZ`, which no rate table has, and every
/// amount then rendered unconverted. The codes are the domain's, so they are
/// here, and the reducer refuses one that is not in them.
///
/// An enum across the boundary too. `SetFiatCurrency` took a code and refused
/// the ones not listed, and the app kept a twelve-case enum of its own with
/// the same codes as raw values, so there were two lists and a string between
/// them. Parsing a typed code is now the front end's job, and the reducer
/// cannot be handed a currency that does not exist.
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

pub fn fiat_currency_codes() -> Vec<String> {
    FiatCurrency::ALL
        .iter()
        .map(|currency| currency.code().to_string())
        .collect()
}

/// Concrete testnet identities are unpriced regardless of selected wallet networks.
#[uniffi::export]
pub fn core_unpriced_chain_names() -> Vec<String> {
    crate::registry::Chain::testnets()
        .flat_map(|chain| {
            [
                chain.chain_display_name().to_string(),
                chain.str_id().to_string(),
            ]
        })
        .collect()
}

/// What a dashboard pins before the user has pinned anything.
///
/// A product default, and one every front end has to agree on: iOS held this
/// list, so its pin cards showed four assets that core's own grouping did not
/// order first, did not mark pinned, and gave no row to when the wallet held
/// none of them.
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
            network_chain_by_family: std::collections::HashMap::new(),
            rpc_endpoint_by_chain: std::collections::HashMap::new(),
            etherscan_api_key: String::new(),
            monero_backend_base_url: String::new(),
            monero_backend_api_key: String::new(),
            bitcoin_esplora_endpoints: String::new(),
            bitcoin_stop_gap: default_bitcoin_stop_gap(),
            fee_priority_by_chain: std::collections::HashMap::new(),
            use_strict_rpc_only: false,
            background_sync_profile: BackgroundSyncProfile::Balanced,
            use_price_alerts: default_true(),
            use_transaction_status_notifications: default_true(),
            use_large_movement_notifications: default_true(),
            large_movement_alert_percent_threshold: default_large_movement_percent(),
            large_movement_alert_usd_threshold: default_large_movement_usd(),
            tor_enabled: false,
            tor_use_custom_proxy: false,
            tor_custom_proxy_address: default_tor_custom_proxy_address(),
            tor_kill_switch: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreAppState {
    pub movement_baseline: Option<crate::service::PortfolioMovementBaseline>,
    #[serde(default)]
    pub quotes: crate::service::QuoteRefreshState,
    #[serde(default)]
    pub diagnostics: crate::service::DiagnosticState,
    pub schema_version: u32,
    pub wallets: Vec<WalletState>,
    pub selected_wallet_id: Option<String>,
    pub settings: AppSettings,
    /// Saved recipients, most recently added first.
    pub address_book: Vec<AddressBookEntry>,
    /// Which tokens the user tracks, and how many decimals each displays.
    #[serde(default)]
    pub token_preferences: Vec<crate::store::wallet_domain::CoreTokenPreferenceEntry>,
    /// Price alerts. Domain state by rule 4 — losing one on restart means an
    /// alert the user set never fires.
    #[serde(default)]
    pub price_alerts: Vec<crate::store::PriceAlertEvaluationAlert>,
    /// USD → display-currency cross rates, as `code -> rate`.
    ///
    /// Every quoted amount passes through these, and they are what the app
    /// shows while a refresh is in flight or the provider is down — so losing
    /// them on restart means every non-USD balance renders as USD until a
    /// network call lands. One front end kept them in its own SQLite blob,
    /// seeded from an older `UserDefaults` key that still won a race at launch.
    #[serde(default)]
    pub fiat_rates_from_usd: std::collections::HashMap<String, f64>,
}

impl Default for CoreAppState {
    fn default() -> Self {
        Self {
            movement_baseline: None,
            quotes: Default::default(),
            schema_version: 2,
            diagnostics: Default::default(),
            wallets: Vec::new(),
            selected_wallet_id: None,
            settings: AppSettings::default(),
            address_book: Vec::new(),
            token_preferences: Vec::new(),
            price_alerts: Vec::new(),
            fiat_rates_from_usd: std::collections::HashMap::new(),
        }
    }
}

/// Most tokens use 18 or fewer; the ceiling exists to stop a typo from
/// producing an unrenderable amount.
pub(crate) const MAX_TOKEN_DECIMALS: i32 = 30;

/// One settings field, and its new value.
///
/// A variant per field rather than a whole-record setter: the record was how
/// this state used to move — iOS built all twenty-three fields from its own
/// properties and wrote them together, so two screens changing two settings
/// raced, and the later write carried the earlier screen's stale copy of
/// everything else. Setting one field says one field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Enum)]
#[serde(tag = "field", rename_all = "camelCase")]
pub enum AppSettingUpdate {
    /// `chain` is a registry display name; an unknown one is refused. An empty
    /// value clears the override and falls back to the catalog.
    RpcEndpoint {
        chain: String,
        value: String,
    },
    EtherscanApiKey {
        value: String,
    },
    MoneroBackendBaseUrl {
        value: String,
    },
    MoneroBackendApiKey {
        value: String,
    },
    BitcoinEsploraEndpoints {
        value: String,
    },
    BitcoinStopGap {
        value: u32,
    },
    /// `chain` is a registry display name; an unknown one is refused and an
    /// unknown `value` falls back to `normal`.
    FeePriority {
        chain: String,
        value: FeePriority,
    },
    UseStrictRpcOnly {
        value: bool,
    },
    BackgroundSyncProfile {
        value: BackgroundSyncProfile,
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
    TorKillSwitch {
        value: bool,
    },
}

/// An intent to change the resident state.
///
/// `ReplaceState` and `UpsertWallet` carry whole records, so every value is as
/// large as those — clippy's `large_enum_variant`. Boxing them would not help:
/// this is a UniFFI enum, and what crosses the boundary is the encoded form,
/// not the Rust layout. Commands are constructed a handful of times per user
/// action, not in a loop.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Enum)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StateCommand {
    ReplaceState {
        state: CoreAppState,
    },
    RenameWallet {
        wallet_id: String,
        name: String,
    },
    SetWalletPortfolioInclusion {
        wallet_id: String,
        included: bool,
    },
    UpsertWallet {
        wallet: WalletState,
    },
    /// Update a wallet only if it is still stored.
    ///
    /// Balance refresh uses this: a refresh result that arrives after the user
    /// deleted the wallet must not bring it back. Creating is a separate
    /// intent, and `UpsertWallet` is the command for it.
    UpdateWalletIfPresent {
        wallet: WalletState,
    },
    SelectWallet {
        wallet_id: String,
    },
    RemoveWallet {
        wallet_id: String,
    },
    SetFiatCurrency {
        currency: FiatCurrency,
    },
    /// Change one settings field. Values are trimmed and bounded here, so a
    /// front end cannot store a stop gap of zero by writing to its own copy.
    SetAppSetting {
        update: AppSettingUpdate,
    },
    /// Put every setting core owns back to its default.
    ///
    /// The defaults are `AppSettings::default()` and nowhere else. iOS used to
    /// reset them by assigning each mirror the value it believed was the
    /// default — a literal per setting, in a file that had no way to know when
    /// one of them changed.
    ResetAppSettings,
    /// Replace the pinned dashboard set. Token IDs are trimmed
    /// and de-duplicated, first occurrence winning, so display order is the
    /// order the user pinned them in.
    SetPinnedDashboardAssets {
        token_ids: Vec<String>,
    },
    /// Restore the default dashboard pins explicitly.
    ResetPinnedDashboardAssets,
    /// Pin or unpin one asset against the saved selection.
    ///
    /// The app built the whole list itself for this, starting from its own
    /// copy of that default rule.
    SetDashboardAssetPinned {
        token_id: String,
        is_pinned: bool,
    },
    /// Pick which network of a chain family the user is on.
    ///
    /// `chain_id` is any chain in the family; the reducer files the choice
    /// under the family's mainnet. Selecting the mainnet clears the entry
    /// rather than storing it, so "no choice made" and "chose mainnet" are the
    /// same state and cannot drift apart.
    SelectNetworkChain {
        chain_id: String,
    },
    /// Add a token the catalog does not ship.
    ///
    /// The reducer trims, upper-cases the symbol, validates the contract with
    /// the chain's own `contract_validation_kind` and refuses a duplicate; a
    /// rejected token produces a `tokenPreferenceRejected` event and no
    /// change. Both front ends used to assemble the whole list and hand it
    /// back — with different duplicate rules, and only one of them checking
    /// the contract at all.
    AddCustomToken {
        chain_name: String,
        symbol: String,
        name: String,
        contract: String,
        coingecko_id: String,
        decimals: u32,
    },
    /// Forget a custom token. A built-in is the catalog's, not the user's.
    RemoveCustomToken {
        chain_name: String,
        contract: String,
    },
    /// Change a custom token's precision. Out of range is refused, not
    /// clamped: a clamp reads every later balance at the wrong scale.
    SetCustomTokenDecimals {
        chain_name: String,
        contract: String,
        decimals: u32,
    },
    /// Turn tokens on or off for balance reads and display. Takes a list
    /// because the registry screen toggles a whole group at once.
    SetTokenPreferencesEnabled {
        tokens: Vec<CoreTokenPreferenceKey>,
        is_enabled: bool,
    },
    /// Back to the catalog's own list, with every custom token dropped.
    ResetTokenPreferences,
    /// Fold this build's catalog into the stored preferences: a user's
    /// `is_enabled` survives, tokens the build added appear, and tokens the
    /// user added stay. Both lists are core's, so neither crosses the
    /// boundary — the caller used to fetch the catalog, reshape it and send
    /// both back for merging.
    MergeBuiltInTokens,
    /// Add a recipient. `address` is normalized and validated by the reducer;
    /// a rejected entry produces an `addressBookRejected` event and no change.
    AddAddressBookEntry {
        id: String,
        name: String,
        chain_name: String,
        address: String,
        note: String,
    },
    AddPriceAlert {
        holding_key: String,
        target_price: f64,
        currency: FiatCurrency,
        condition: crate::store::wallet_domain::CorePriceAlertCondition,
    },
    TogglePriceAlert {
        id: String,
    },
    RemovePriceAlert {
        id: String,
    },
    RenameAddressBookEntry {
        id: String,
        name: String,
    },
    RemoveAddressBookEntry {
        id: String,
    },
}

/// What a state change did, or why it was refused.
///
/// A record of a free-string `kind` and an optional `subject_id` before, so a
/// refusal's reason was a string inside a string, every front end matched on
/// spellings like `"addressBookRejected"`, and a misspelling compiled. The
/// serialized form keeps `kind` beside each variant's fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StateEvent {
    StateReplaced,
    DataReset,
    WalletAdded {
        wallet_id: String,
    },
    WalletUpdated {
        wallet_id: String,
    },
    WalletSelected {
        wallet_id: String,
    },
    WalletRemoved {
        wallet_id: String,
    },
    /// A balance refresh changed a wallet's holdings.
    WalletBalancesChanged {
        wallet_id: String,
    },
    AddressBookEntryAdded {
        id: String,
    },
    AddressBookEntryRenamed {
        id: String,
    },
    AddressBookEntryRemoved {
        id: String,
    },
    AddressBookRejected {
        reason: AddressBookRejection,
    },
    FiatCurrencyChanged {
        currency: FiatCurrency,
    },
    AppSettingChanged,
    AppSettingRejected,
    /// `symbol` names the one token changed, when one was.
    TokenPreferencesChanged {
        symbol: Option<String>,
    },
    TokenPreferenceRejected {
        reason: TokenPreferenceRejection,
    },
    PriceAlertAdded {
        id: String,
    },
    PriceAlertChanged {
        id: String,
    },
    PriceAlertRemoved {
        id: String,
    },
    PriceAlertRejected {
        reason: super::PriceAlertRejection,
    },
    PriceAlertsEvaluated,
    NetworkChainChanged {
        chain_id: String,
    },
    PinnedDashboardAssetsChanged,
    QuotesUpdated,
    FiatRatesChanged,
    DiagnosticsChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StateTransition {
    pub state: CoreAppState,
    pub events: Vec<StateEvent>,
}

/// Is this (chain, address) pair already saved? Addresses are stored
/// normalized, so this is a case-insensitive compare rather than a per-chain
/// rule. `excluding` skips one entry, for edit-in-place checks.
fn address_book_contains(
    state: &CoreAppState,
    chain_name: &str,
    normalized_address: &str,
    excluding: Option<&str>,
) -> bool {
    if normalized_address.is_empty() {
        return false;
    }
    state.address_book.iter().any(|entry| {
        Some(entry.id.as_str()) != excluding
            && entry.chain_name == chain_name
            && entry.address.eq_ignore_ascii_case(normalized_address)
    })
}

/// Longer than any token symbol is spelled. Past this the field holds a pasted
/// name or a whole contract address.
pub(crate) const MAX_TOKEN_SYMBOL_CHARS: usize = 12;

/// Where a (chain, contract) pair sits in the preference list, if at all.
///
/// Matched on the *normalized* contract, which is the chain's own rule — a TON
/// jetton address is case-significant and an EVM one is not. Swift compared
/// normalized identifiers and the CLI compared symbols case-insensitively, so
/// the two front ends disagreed about what a duplicate even was.
fn token_preference_index(state: &CoreAppState, chain_name: &str, contract: &str) -> Option<usize> {
    let hosting = crate::store::wallet_domain::CoreTokenHostingChain::from_chain_name(chain_name)?;
    token_preference_row(state, hosting, contract)
}

fn token_preference_row(
    state: &CoreAppState,
    hosting: crate::store::wallet_domain::CoreTokenHostingChain,
    contract: &str,
) -> Option<usize> {
    let needle = crate::tokens::normalize_token_identifier(
        Some(contract.to_string()),
        hosting.chain_name().to_string(),
    )?;
    state.token_preferences.iter().position(|entry| {
        entry.hosting_chain() == Some(hosting)
            && crate::tokens::normalize_token_identifier(
                Some(entry.token.contract.clone()),
                hosting.chain_name().to_string(),
            )
            .as_deref()
                == Some(needle.as_str())
    })
}

fn token_preference_rejected(reason: TokenPreferenceRejection) -> StateEvent {
    StateEvent::TokenPreferenceRejected { reason }
}

/// Chain, then the catalog's own rows before the user's, then symbol.
///
/// The same order `plan_merge_built_in_token_preferences` produces, so a list
/// that has just been added to still matches the one a reload builds. Swift
/// re-sorted with its own copy of this comparator after every insert.
fn sort_token_preferences(entries: &mut [crate::store::wallet_domain::CoreTokenPreferenceEntry]) {
    entries.sort_by(|lhs, rhs| {
        lhs.token
            .chain
            .cmp(&rhs.token.chain)
            .then_with(|| rhs.is_built_in.cmp(&lhs.is_built_in))
            .then_with(|| lhs.token.symbol.cmp(&rhs.token.symbol))
    });
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
/// For a front end to show an edit before the command that stores it returns.
/// Its settings screen was eighteen mirrored properties, a hand-written diff
/// against core's last answer and a hand-written adoption back, because
/// showing the value core would store meant knowing core's trims and clamps.
/// Asking the rule is cheaper than copying it.
#[uniffi::export]
pub fn app_settings_applying(settings: AppSettings, update: AppSettingUpdate) -> AppSettings {
    let mut settings = settings;
    apply_app_setting(&mut settings, update);
    settings
}

/// Apply one settings update, trimming strings and bounding numbers, and
/// answer whether it was accepted.
///
/// The clamps were `didSet` bodies on the iOS side — the only copy, so a value
/// out of range was only out of range where someone had remembered to check.
///
/// An update naming a chain the registry does not have, or a proxy address
/// that is not a SOCKS5 URL, changes nothing — and used to say nothing either,
/// so a front end or a script could set a value, be told the command
/// succeeded, and read back the old one. The caller turns `false` into a
/// refusal event.
fn apply_app_setting(settings: &mut AppSettings, update: AppSettingUpdate) -> bool {
    fn trimmed(value: String) -> String {
        value.trim().to_string()
    }
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
    // An endpoint the app would fail to reach is refused rather than stored:
    // every read of it would otherwise have to re-check, and a front end that
    // shows the error while the user types needs nothing but this rule.
    fn valid_endpoint(field: crate::tokens::EndpointField, value: &str) -> bool {
        crate::tokens::endpoint_validation_error(field, value.to_string()).is_none()
    }
    match update {
        AppSettingUpdate::RpcEndpoint { chain, value } => {
            let Some(chain) = crate::registry::Chain::from_display_name(&chain) else {
                return false;
            };
            let value = trimmed(value);
            if !valid_endpoint(crate::tokens::EndpointField::EvmRpc, &value) {
                return false;
            }
            if value.is_empty() {
                settings
                    .rpc_endpoint_by_chain
                    .remove(chain.chain_display_name());
            } else {
                settings
                    .rpc_endpoint_by_chain
                    .insert(chain.chain_display_name().to_string(), value);
            }
        }
        AppSettingUpdate::EtherscanApiKey { value } => settings.etherscan_api_key = trimmed(value),
        AppSettingUpdate::MoneroBackendBaseUrl { value } => {
            let value = trimmed(value);
            if !valid_endpoint(crate::tokens::EndpointField::MoneroBackend, &value) {
                return false;
            }
            settings.monero_backend_base_url = value
        }
        AppSettingUpdate::MoneroBackendApiKey { value } => {
            settings.monero_backend_api_key = trimmed(value)
        }
        // Not trimmed as a whole: this is a separated list, and the parser
        // trims each entry. Trimming the list would only drop its outer edges.
        AppSettingUpdate::BitcoinEsploraEndpoints { value } => {
            if !valid_endpoint(crate::tokens::EndpointField::BitcoinEsploraList, &value) {
                return false;
            }
            settings.bitcoin_esplora_endpoints = value
        }
        AppSettingUpdate::BitcoinStopGap { value } => {
            settings.bitcoin_stop_gap = clamp(value, BITCOIN_STOP_GAP_RANGE)
        }
        AppSettingUpdate::FeePriority { chain, value } => {
            let Some(chain) = crate::registry::Chain::from_display_name(&chain) else {
                return false;
            };
            if value == FeePriority::Normal {
                settings
                    .fee_priority_by_chain
                    .remove(chain.chain_display_name());
            } else {
                settings
                    .fee_priority_by_chain
                    .insert(chain.chain_display_name().to_string(), value);
            }
        }
        AppSettingUpdate::UseStrictRpcOnly { value } => settings.use_strict_rpc_only = value,
        AppSettingUpdate::BackgroundSyncProfile { value } => {
            settings.background_sync_profile = value
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
        AppSettingUpdate::TorKillSwitch { value } => settings.tor_kill_switch = value,
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
    state: &mut CoreAppState,
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

pub fn reduce_state_in_place(state: &mut CoreAppState, command: StateCommand) -> Vec<StateEvent> {
    let mut events = Vec::new();

    match command {
        StateCommand::ReplaceState { state: next_state } => {
            *state = next_state;
            events.push(StateEvent::StateReplaced);
        }
        StateCommand::RenameWallet { wallet_id, name } => {
            let name = name.trim();
            if !name.is_empty() {
                if let Some(wallet) = state.wallets.iter_mut().find(|w| w.id == wallet_id) {
                    if wallet.name != name {
                        wallet.name = name.to_owned();
                        events.push(StateEvent::WalletUpdated { wallet_id });
                    }
                }
            }
        }
        StateCommand::SetWalletPortfolioInclusion {
            wallet_id,
            included,
        } => {
            if let Some(wallet) = state.wallets.iter_mut().find(|w| w.id == wallet_id) {
                if wallet.include_in_portfolio_total != included {
                    wallet.include_in_portfolio_total = included;
                    events.push(StateEvent::WalletUpdated { wallet_id });
                }
            }
        }
        StateCommand::UpsertWallet { wallet } => {
            let wallet_id = wallet.id.clone();
            if let Some(index) = state
                .wallets
                .iter()
                .position(|candidate| candidate.id == wallet_id)
            {
                state.wallets[index] = wallet;
                events.push(StateEvent::WalletUpdated {
                    wallet_id: wallet_id.clone(),
                });
            } else {
                state.wallets.push(wallet);
                events.push(StateEvent::WalletAdded {
                    wallet_id: wallet_id.clone(),
                });
            }

            if state.selected_wallet_id.is_none() {
                state.selected_wallet_id = Some(wallet_id);
            }
        }
        StateCommand::UpdateWalletIfPresent { wallet } => {
            if let Some(index) = state.wallets.iter().position(|w| w.id == wallet.id) {
                if state.wallets[index] != wallet {
                    let wallet_id = wallet.id.clone();
                    state.wallets[index] = wallet;
                    events.push(StateEvent::WalletUpdated { wallet_id });
                }
            }
        }
        StateCommand::SelectWallet { wallet_id } => {
            if state.wallets.iter().any(|wallet| wallet.id == wallet_id) {
                state.selected_wallet_id = Some(wallet_id.clone());
                events.push(StateEvent::WalletSelected { wallet_id });
            }
        }
        StateCommand::RemoveWallet { wallet_id } => {
            let before = state.wallets.len();
            state.wallets.retain(|wallet| wallet.id != wallet_id);
            if state.wallets.len() != before {
                if state.selected_wallet_id.as_deref() == Some(wallet_id.as_str()) {
                    state.selected_wallet_id =
                        state.wallets.first().map(|wallet| wallet.id.clone());
                }
                events.push(StateEvent::WalletRemoved { wallet_id });
            }
        }
        StateCommand::AddAddressBookEntry {
            id,
            name,
            chain_name,
            address,
            note,
        } => {
            let name = name.trim().to_string();
            let address = crate::send::flow::normalize_address(&chain_name, &address);

            // Refusals are reported, not silently dropped: a front end that
            // ignored the result would otherwise show a saved contact that was
            // never saved.
            let rejection = if name.is_empty() {
                Some(AddressBookRejection::EmptyName)
            } else if !crate::send::flow::is_valid_send_address(chain_name.clone(), address.clone())
            {
                Some(AddressBookRejection::InvalidAddress)
            } else if address_book_contains(state, &chain_name, &address, None) {
                Some(AddressBookRejection::DuplicateAddress)
            } else {
                None
            };

            match rejection {
                Some(reason) => events.push(StateEvent::AddressBookRejected { reason }),
                None => {
                    // Newest first: the list is a recency-ordered shortlist,
                    // not an archive.
                    state.address_book.insert(
                        0,
                        AddressBookEntry {
                            id: id.clone(),
                            name,
                            chain_name,
                            address,
                            note: note.trim().to_string(),
                        },
                    );
                    events.push(StateEvent::AddressBookEntryAdded { id });
                }
            }
        }
        StateCommand::RenameAddressBookEntry { id, name } => {
            let name = name.trim().to_string();
            if name.is_empty() {
                events.push(StateEvent::AddressBookRejected {
                    reason: AddressBookRejection::EmptyName,
                });
            } else if let Some(entry) = state.address_book.iter_mut().find(|e| e.id == id) {
                if entry.name != name {
                    entry.name = name;
                    events.push(StateEvent::AddressBookEntryRenamed { id });
                }
            }
        }
        StateCommand::RemoveAddressBookEntry { id } => {
            let before = state.address_book.len();
            state.address_book.retain(|entry| entry.id != id);
            if state.address_book.len() != before {
                events.push(StateEvent::AddressBookEntryRemoved { id });
            }
        }
        StateCommand::SetFiatCurrency { currency } => {
            if currency != state.settings.fiat_currency {
                state.settings.fiat_currency = currency;
                events.push(StateEvent::FiatCurrencyChanged { currency });
            }
        }
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
        StateCommand::AddPriceAlert {
            holding_key,
            target_price,
            currency,
            condition,
        } => events.extend(super::price_alerts::add(
            state,
            holding_key,
            target_price,
            currency,
            condition,
        )),
        StateCommand::TogglePriceAlert { id } => {
            events.extend(super::price_alerts::toggle(state, id))
        }
        StateCommand::RemovePriceAlert { id } => {
            events.extend(super::price_alerts::remove(state, id))
        }
        StateCommand::AddCustomToken {
            chain_name,
            symbol,
            name,
            contract,
            coingecko_id,
            decimals,
        } => {
            let symbol = symbol.trim().to_uppercase();
            let name = name.trim().to_string();
            let contract =
                crate::tokens::normalize_token_identifier(Some(contract), chain_name.clone())
                    .unwrap_or_default();
            let hosting =
                crate::store::wallet_domain::CoreTokenHostingChain::from_chain_name(&chain_name);

            let rejection = match hosting {
                None => Some(TokenPreferenceRejection::UnknownChain),
                Some(_) if symbol.is_empty() => Some(TokenPreferenceRejection::EmptySymbol),
                // Twelve characters is longer than any symbol is spelled; past
                // it the field has a pasted name or a whole address in it.
                Some(_) if symbol.chars().count() > MAX_TOKEN_SYMBOL_CHARS => {
                    Some(TokenPreferenceRejection::SymbolTooLong)
                }
                Some(_) if name.is_empty() => Some(TokenPreferenceRejection::EmptyName),
                Some(_) if contract.is_empty() => Some(TokenPreferenceRejection::EmptyContract),
                Some(_) if decimals > MAX_TOKEN_DECIMALS as u32 => {
                    Some(TokenPreferenceRejection::TooManyDecimals)
                }
                Some(hosting)
                    if !crate::validation::address::validate_address(
                        crate::validation::address::AddressValidationRequest {
                            kind: hosting.contract_validation_kind().to_string(),
                            value: contract.clone(),
                        },
                    )
                    .is_valid =>
                {
                    Some(TokenPreferenceRejection::InvalidContract)
                }
                Some(hosting) if token_preference_row(state, hosting, &contract).is_some() => {
                    Some(TokenPreferenceRejection::DuplicateToken)
                }
                Some(_) => None,
            };

            match (rejection, hosting) {
                (Some(reason), _) => events.push(token_preference_rejected(reason)),
                (None, Some(hosting)) => {
                    state.token_preferences.push(
                        crate::store::wallet_domain::CoreTokenPreferenceEntry {
                            category:
                                crate::store::wallet_domain::CoreTokenPreferenceCategory::Custom,
                            is_built_in: false,
                            is_enabled: true,
                            token: crate::tokens::TokenEntry {
                                id: format!(
                                    "{}:{}:{}",
                                    crate::registry::Chain::from_display_name(hosting.chain_name())
                                        .unwrap()
                                        .str_id(),
                                    hosting.token_standard().to_lowercase(),
                                    contract
                                ),
                                token_id: format!(
                                    "custom:{}:{}:{}",
                                    crate::registry::Chain::from_display_name(hosting.chain_name())
                                        .unwrap()
                                        .str_id(),
                                    hosting.token_standard().to_lowercase(),
                                    contract
                                ),
                                kind: crate::tokens::TokenKind::Protocol {
                                    standard: hosting.token_standard(),
                                    identifier: contract.clone(),
                                },
                                chain: hosting.chain_name().to_string(),
                                name,
                                symbol: symbol.clone(),
                                token_standard: hosting.token_standard().to_string(),
                                contract,
                                coingecko_id: coingecko_id.trim().to_string(),
                                decimals,
                                tags: Vec::new(),
                                color: None,
                                artwork_name: String::new(),
                                enabled: true,
                            },
                        },
                    );
                    sort_token_preferences(&mut state.token_preferences);
                    events.push(StateEvent::TokenPreferencesChanged {
                        symbol: Some(symbol),
                    });
                }
                (None, None) => unreachable!("an unknown chain is rejected above"),
            }
        }
        StateCommand::RemoveCustomToken {
            chain_name,
            contract,
        } => match token_preference_index(state, &chain_name, &contract) {
            None => events.push(token_preference_rejected(
                TokenPreferenceRejection::UnknownToken,
            )),
            Some(index) if state.token_preferences[index].is_built_in => events.push(
                token_preference_rejected(TokenPreferenceRejection::BuiltInToken),
            ),
            Some(index) => {
                let removed = state.token_preferences.remove(index);
                events.push(StateEvent::TokenPreferencesChanged {
                    symbol: Some(removed.token.symbol),
                });
            }
        },
        StateCommand::SetCustomTokenDecimals {
            chain_name,
            contract,
            decimals,
        } => match token_preference_index(state, &chain_name, &contract) {
            None => events.push(token_preference_rejected(
                TokenPreferenceRejection::UnknownToken,
            )),
            Some(index) if state.token_preferences[index].is_built_in => events.push(
                token_preference_rejected(TokenPreferenceRejection::BuiltInToken),
            ),
            Some(_) if decimals > MAX_TOKEN_DECIMALS as u32 => events.push(
                token_preference_rejected(TokenPreferenceRejection::TooManyDecimals),
            ),
            Some(index) => {
                if state.token_preferences[index].token.decimals != decimals {
                    state.token_preferences[index].token.decimals = decimals;
                    events.push(StateEvent::TokenPreferencesChanged {
                        symbol: Some(state.token_preferences[index].token.symbol.clone()),
                    });
                }
            }
        },
        StateCommand::SetTokenPreferencesEnabled { tokens, is_enabled } => {
            let mut changed = false;
            for key in tokens {
                let Some(index) = token_preference_index(state, &key.chain_name, &key.contract)
                else {
                    events.push(token_preference_rejected(
                        TokenPreferenceRejection::UnknownToken,
                    ));
                    continue;
                };
                if state.token_preferences[index].is_enabled != is_enabled {
                    state.token_preferences[index].is_enabled = is_enabled;
                    changed = true;
                }
            }
            if changed {
                events.push(StateEvent::TokenPreferencesChanged { symbol: None });
            }
        }
        StateCommand::MergeBuiltInTokens => {
            let merged = crate::store::plan_merge_built_in_token_preferences(
                crate::store::built_in_token_preferences(),
                std::mem::take(&mut state.token_preferences),
            );
            if merged != state.token_preferences {
                state.token_preferences = merged;
                events.push(StateEvent::TokenPreferencesChanged { symbol: None });
            } else {
                state.token_preferences = merged;
            }
        }
        StateCommand::ResetTokenPreferences => {
            let defaults = crate::store::built_in_token_preferences();
            if defaults != state.token_preferences {
                state.token_preferences = defaults;
                sort_token_preferences(&mut state.token_preferences);
                events.push(StateEvent::TokenPreferencesChanged { symbol: None });
            }
        }
        StateCommand::SelectNetworkChain { chain_id } => {
            if let Some(chosen) = crate::registry::Chain::from_str_id(&chain_id) {
                let family = chosen.mainnet_counterpart();
                let before = state.settings.network_chain_by_family.clone();
                state
                    .settings
                    .network_chain_by_family
                    .insert(family.str_id().into(), chosen.str_id().into());
                for wallet in &mut state.wallets {
                    if crate::registry::Chain::from_display_name(&wallet.chain_name)
                        .is_some_and(|c| c.mainnet_counterpart() == family)
                    {
                        wallet.network_id = chosen.str_id().into();
                    }
                }
                if before != state.settings.network_chain_by_family {
                    events.push(StateEvent::NetworkChainChanged {
                        chain_id: chosen.str_id().to_string(),
                    });
                }
            }
        }
        StateCommand::ResetPinnedDashboardAssets => {
            set_pinned_dashboard_assets(state, default_pinned_dashboard_assets(), &mut events)
        }
        StateCommand::SetPinnedDashboardAssets { token_ids } => {
            set_pinned_dashboard_assets(state, token_ids, &mut events)
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
            set_pinned_dashboard_assets(state, token_ids, &mut events)
        }
    }

    events
}

#[cfg(test)]
mod fee_priority_tests {
    use super::*;

    #[test]
    fn the_three_are_read_by_name_and_everything_else_is_the_default() {
        assert_eq!(parse_fee_priority(" Economy ".into()), FeePriority::Economy);
        assert_eq!(parse_fee_priority("priority".into()), FeePriority::Priority);
        assert_eq!(parse_fee_priority("NORMAL".into()), FeePriority::Normal);
        for raw in ["lightspeed", "", "   ", "instant", "priority!"] {
            assert_eq!(
                parse_fee_priority(raw.to_string()),
                FeePriority::Normal,
                "{raw:?}"
            );
        }
    }

    #[test]
    fn a_stored_value_the_three_do_not_name_opens_as_the_default() {
        let mut file = serde_json::to_value(AppSettings::default()).expect("settings serialize");
        file["feePriorityByChain"] =
            serde_json::json!({ "Bitcoin": "priority", "Dogecoin": "lightspeed" });
        let settings: AppSettings = serde_json::from_value(file)
            .expect("a settings file with a fourth priority still opens");
        assert_eq!(
            settings.fee_priority_by_chain.get("Bitcoin"),
            Some(&FeePriority::Priority)
        );
        assert_eq!(
            settings.fee_priority_by_chain.get("Dogecoin"),
            Some(&FeePriority::Normal)
        );
    }

    #[test]
    fn the_stored_spelling_is_the_one_the_setting_had() {
        assert_eq!(
            serde_json::to_string(&FeePriority::Priority).expect("serializes"),
            "\"priority\""
        );
        assert_eq!(FeePriority::Economy.as_raw(), "economy");
    }

    #[test]
    fn an_endpoint_that_is_not_a_url_is_refused_not_stored() {
        let mut settings = AppSettings::default();
        let refused = [
            AppSettingUpdate::RpcEndpoint {
                chain: "Base".into(),
                value: "base.internal".into(),
            },
            AppSettingUpdate::MoneroBackendBaseUrl {
                value: "ftp://node.example".into(),
            },
            AppSettingUpdate::BitcoinEsploraEndpoints {
                value: "https://a.example, not a url".into(),
            },
        ];
        for update in refused {
            assert!(!apply_app_setting(&mut settings, update));
        }
        assert_eq!(settings, AppSettings::default());

        // Clearing is always allowed: empty means "use the catalog".
        assert!(apply_app_setting(
            &mut settings,
            AppSettingUpdate::RpcEndpoint {
                chain: "Base".into(),
                value: " https://base.internal ".into(),
            }
        ));
        assert!(apply_app_setting(
            &mut settings,
            AppSettingUpdate::MoneroBackendBaseUrl { value: "".into() }
        ));
        assert_eq!(
            settings
                .rpc_endpoint_by_chain
                .get("Base")
                .map(String::as_str),
            Some("https://base.internal")
        );
    }

    #[test]
    fn picking_the_default_stops_storing_a_choice() {
        let mut settings = AppSettings::default();
        assert!(apply_app_setting(
            &mut settings,
            AppSettingUpdate::FeePriority {
                chain: "Dogecoin".into(),
                value: FeePriority::Economy,
            }
        ));
        assert_eq!(
            settings.fee_priority_by_chain.get("Dogecoin"),
            Some(&FeePriority::Economy)
        );
        assert!(apply_app_setting(
            &mut settings,
            AppSettingUpdate::FeePriority {
                chain: "Dogecoin".into(),
                value: FeePriority::Normal,
            }
        ));
        assert!(settings.fee_priority_by_chain.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reduce_state(mut state: CoreAppState, command: StateCommand) -> StateTransition {
        let events = reduce_state_in_place(&mut state, command);
        StateTransition { state, events }
    }

    fn test_wallet(id: &str, chain: &str) -> WalletState {
        WalletState {
            id: id.to_string(),
            name: "Main".to_string(),
            is_watch_only: false,
            chain_name: chain.to_string(),
            include_in_portfolio_total: true,
            network_id: crate::registry::Chain::from_display_name(chain)
                .unwrap()
                .str_id()
                .into(),
            xpub: None,
            derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
            derivation_overrides: Default::default(),
            derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
            holdings: Vec::new(),
            addresses: vec![WalletAddress {
                chain_name: chain.to_string(),
                address: "bc1qexample".to_string(),
                kind: "address".to_string(),
                derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
            }],
        }
    }

    fn add_token(chain: &str, symbol: &str, contract: &str, decimals: u32) -> StateCommand {
        StateCommand::AddCustomToken {
            chain_name: chain.to_string(),
            symbol: symbol.to_string(),
            name: "A Token".to_string(),
            contract: contract.to_string(),
            coingecko_id: String::new(),
            decimals,
        }
    }

    fn rejection(transition: &StateTransition) -> Option<TokenPreferenceRejection> {
        transition.events.iter().find_map(|event| match event {
            StateEvent::TokenPreferenceRejected { reason } => Some(*reason),
            _ => None,
        })
    }

    const EVM_CONTRACT: &str = "0x742d35cc6634c0532925a3b844bc454e4438f44e";

    /// A contract is judged by the chain that would host it, not by whichever
    /// arm a switch fell into. The composer's `default` assumed EVM and the
    /// CLI checked nothing at all, so a Solana mint went into the Base list
    /// and every balance read for it failed.
    #[test]
    fn a_contract_is_judged_by_the_chain_that_hosts_it() {
        let solana_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let wrong_chain = reduce_state(
            CoreAppState::default(),
            add_token("Base", "USDC", solana_mint, 6),
        );
        assert_eq!(
            rejection(&wrong_chain),
            Some(TokenPreferenceRejection::InvalidContract)
        );
        assert!(wrong_chain.state.token_preferences.is_empty());

        let wrong_way_round = reduce_state(
            CoreAppState::default(),
            add_token("Solana", "USDC", EVM_CONTRACT, 6),
        );
        assert_eq!(
            rejection(&wrong_way_round),
            Some(TokenPreferenceRejection::InvalidContract)
        );

        let right = reduce_state(
            CoreAppState::default(),
            add_token("Solana", "USDC", solana_mint, 6),
        );
        assert_eq!(rejection(&right), None);
        assert_eq!(right.state.token_preferences.len(), 1);
    }

    /// The symbol is trimmed and upper-cased, and a pasted name is refused
    /// rather than stored as one.
    #[test]
    fn a_symbol_is_normalized_and_a_pasted_name_is_not_one() {
        let added = reduce_state(
            CoreAppState::default(),
            add_token("Base", "  moon ", EVM_CONTRACT, 18),
        );
        assert_eq!(added.state.token_preferences[0].token.symbol, "MOON");
        // The catalog's standard comes from the chain, not the caller.
        assert_eq!(
            added.state.token_preferences[0].token.token_standard,
            crate::store::wallet_domain::CoreTokenHostingChain::Base.token_standard()
        );
        assert!(!added.state.token_preferences[0].is_built_in);

        let pasted = reduce_state(
            CoreAppState::default(),
            add_token("Base", "Moonbeam Network Token", EVM_CONTRACT, 18),
        );
        assert_eq!(
            rejection(&pasted),
            Some(TokenPreferenceRejection::SymbolTooLong)
        );

        let empty = reduce_state(
            CoreAppState::default(),
            add_token("Base", "  ", EVM_CONTRACT, 18),
        );
        assert_eq!(
            rejection(&empty),
            Some(TokenPreferenceRejection::EmptySymbol)
        );
    }

    /// A duplicate is the same *contract* on the same chain, under the chain's
    /// own normalization — so an EVM address in another case is one and the
    /// CLI's symbol compare was answering a different question.
    #[test]
    fn a_duplicate_is_the_same_contract_however_it_is_spelled() {
        let first = reduce_state(
            CoreAppState::default(),
            add_token("Base", "MOON", EVM_CONTRACT, 18),
        );
        let again = reduce_state(
            first.state.clone(),
            add_token("Base", "SUN", &EVM_CONTRACT.to_uppercase(), 18),
        );
        assert_eq!(
            rejection(&again),
            Some(TokenPreferenceRejection::DuplicateToken)
        );
        assert_eq!(again.state.token_preferences.len(), 1);

        // Same contract string, different chain: two different tokens.
        let elsewhere = reduce_state(first.state, add_token("Arbitrum", "MOON", EVM_CONTRACT, 18));
        assert_eq!(rejection(&elsewhere), None);
        assert_eq!(elsewhere.state.token_preferences.len(), 2);
    }

    /// The catalog's rows are not the user's to edit or delete.
    #[test]
    fn a_built_in_token_is_not_editable() {
        let mut state = CoreAppState::default();
        reduce_state_in_place(&mut state, StateCommand::MergeBuiltInTokens);
        let built_in = state
            .token_preferences
            .iter()
            .find(|entry| entry.is_built_in)
            .expect("the catalog ships tokens")
            .clone();
        let count = state.token_preferences.len();

        let removed = reduce_state(
            state.clone(),
            StateCommand::RemoveCustomToken {
                chain_name: built_in.token.chain.clone(),
                contract: built_in.token.contract.clone(),
            },
        );
        assert_eq!(
            rejection(&removed),
            Some(TokenPreferenceRejection::BuiltInToken)
        );
        assert_eq!(removed.state.token_preferences.len(), count);

        let rescaled = reduce_state(
            state,
            StateCommand::SetCustomTokenDecimals {
                chain_name: built_in.token.chain.clone(),
                contract: built_in.token.contract.clone(),
                decimals: 2,
            },
        );
        assert_eq!(
            rejection(&rescaled),
            Some(TokenPreferenceRejection::BuiltInToken)
        );
    }

    /// Turning a token off is not deleting it: the row stays, so the catalog
    /// merge keeps the choice and the list does not have to be rebuilt.
    #[test]
    fn tracking_is_a_flag_and_a_group_moves_together() {
        let mut state = CoreAppState::default();
        reduce_state_in_place(&mut state, StateCommand::MergeBuiltInTokens);
        let count = state.token_preferences.len();
        let keys: Vec<CoreTokenPreferenceKey> = state
            .token_preferences
            .iter()
            .filter(|entry| entry.is_enabled)
            .take(3)
            .map(|entry| CoreTokenPreferenceKey {
                chain_name: entry.token.chain.clone(),
                contract: entry.token.contract.clone(),
            })
            .collect();
        assert_eq!(keys.len(), 3, "the catalog ships enabled tokens");

        let disabled_before = state
            .token_preferences
            .iter()
            .filter(|entry| !entry.is_enabled)
            .count();

        let off = reduce_state(
            state,
            StateCommand::SetTokenPreferencesEnabled {
                tokens: keys.clone(),
                is_enabled: false,
            },
        );
        assert_eq!(
            off.state.token_preferences.len(),
            count,
            "untracking is not deleting"
        );
        assert_eq!(
            off.state
                .token_preferences
                .iter()
                .filter(|entry| !entry.is_enabled)
                .count(),
            disabled_before + keys.len(),
            "the whole group moved"
        );
        assert_eq!(
            off.events
                .iter()
                .filter(|event| matches!(event, StateEvent::TokenPreferencesChanged { .. }))
                .count(),
            1,
            "one change, however many rows it touched"
        );

        // Applying the same value again changes nothing and says so.
        let again = reduce_state(
            off.state,
            StateCommand::SetTokenPreferencesEnabled {
                tokens: keys,
                is_enabled: false,
            },
        );
        assert!(again.events.is_empty());
    }

    /// A reset goes back to the catalog and takes the custom rows with it.
    #[test]
    fn a_reset_drops_what_the_user_added() {
        let added = reduce_state(
            CoreAppState::default(),
            add_token("Base", "MOON", EVM_CONTRACT, 18),
        );
        let reset = reduce_state(added.state, StateCommand::ResetTokenPreferences);
        assert!(
            !reset
                .state
                .token_preferences
                .iter()
                .any(|entry| entry.token.symbol == "MOON"),
            "a custom token survived the reset"
        );
        assert!(reset.state.token_preferences.iter().all(|e| e.is_built_in));
    }

    #[test]
    fn upsert_wallet_selects_first_wallet() {
        let state = CoreAppState::default();
        let transition = reduce_state(
            state,
            StateCommand::UpsertWallet {
                wallet: test_wallet("wallet-1", "Bitcoin"),
            },
        );

        assert_eq!(
            transition.state.selected_wallet_id.as_deref(),
            Some("wallet-1")
        );
        assert!(matches!(
            transition.events[0],
            StateEvent::WalletAdded { .. }
        ));
    }
}
