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
pub struct AssetHolding {
    pub name: String,
    pub symbol: String,
    pub coin_gecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
    pub price_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletSummary {
    pub id: String,
    pub name: String,
    pub is_watch_only: bool,
    pub chain_name: String,
    pub include_in_portfolio_total: bool,
    pub network_mode: Option<String>,
    pub xpub: Option<String>,
    pub derivation_preset: String,
    /// The single path this wallet derives from. A wallet belongs to one chain,
    /// so it needs one path — not the whole per-chain table.
    pub derivation_path: Option<String>,
    /// Power-user derivation overrides, if the wallet was imported with any.
    pub derivation_overrides: crate::store::wallet_domain::CoreWalletDerivationOverrides,
    pub holdings: Vec<AssetHolding>,
    pub addresses: Vec<WalletAddress>,
}

// Plain `impl` — deliberately not `#[uniffi::export]`. These are Rust-side
// domain helpers; the FFI surface stays the record's fields.
impl WalletSummary {
    /// Build a summary for a wallet with one address on one chain.
    ///
    /// This is the shape most chains produce: a single derived address, no
    /// xpub, no network-mode variants. Multi-address wallets (Bitcoin and the
    /// other UTXO chains) push into `addresses` instead.
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
            network_mode: None,
            xpub: None,
            derivation_preset: "default".to_string(),
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
    /// ISO 4217 code the user wants amounts displayed in.
    pub fiat_currency_code: String,
    /// Asset symbols the user pinned to the dashboard, in display order.
    /// Empty means "not chosen yet" — the front end shows its own defaults.
    ///
    /// `default` so that a state file written before this field existed still
    /// loads. Not a migration shim — the struct simply grows, and an absent
    /// list is exactly the same as an empty one.
    #[serde(default)]
    pub pinned_dashboard_asset_symbols: Vec<String>,
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
    #[serde(default = "default_pricing_provider")]
    pub pricing_provider: String,
    /// Which source to take fiat cross-rates from.
    #[serde(default = "default_fiat_rate_provider")]
    pub fiat_rate_provider: String,

    // ── Endpoints and credentials ─────────────────────────────────────────
    /// Custom Ethereum RPC, or empty for the catalog's list.
    #[serde(default)]
    pub ethereum_rpc_endpoint: String,
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
    ///
    /// One field rather than one per chain: Bitcoin and Dogecoin each had
    /// their own settings field and their own Swift enum, while the other
    /// seventy-six shared a dictionary iOS persisted itself — three stores for
    /// one preference, and the front ends disagreed about which was canonical.
    #[serde(default)]
    pub fee_priority_by_chain: std::collections::HashMap<String, String>,

    // ── Network and refresh policy ────────────────────────────────────────
    /// Refuse endpoints the user has not vetted.
    #[serde(default)]
    pub use_strict_rpc_only: bool,
    #[serde(default = "default_background_sync_profile")]
    pub background_sync_profile: String,
    #[serde(default = "default_refresh_frequency_minutes")]
    pub automatic_refresh_frequency_minutes: u32,

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
// were: a value outside them is refused at the reducer, so no caller can store
// a stop gap of zero or a refresh interval that would hammer an endpoint.
pub const BITCOIN_STOP_GAP_RANGE: std::ops::RangeInclusive<u32> = 1..=200;
pub const REFRESH_FREQUENCY_MINUTES_RANGE: std::ops::RangeInclusive<u32> = 5..=60;
pub const LARGE_MOVEMENT_PERCENT_RANGE: std::ops::RangeInclusive<f64> = 1.0..=90.0;
pub const LARGE_MOVEMENT_USD_RANGE: std::ops::RangeInclusive<f64> = 1.0..=100_000.0;

// One notion of "default" per field: `AppSettings::default()` calls these, and
// serde reads them for a field a stored row does not carry. Splitting the two
// is how a row written before `use_price_alerts` existed would have loaded with
// alerts silently off, rather than on as a fresh install has them.
fn default_pricing_provider() -> String {
    "CoinGecko".to_string()
}
fn default_fiat_rate_provider() -> String {
    "Open ER".to_string()
}
fn default_fee_priority() -> String {
    "normal".to_string()
}

/// The three the picker offers. Anything else is the default rather than a
/// stored value no send path knows how to spend.
fn normalized_fee_priority(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "economy" => "economy".to_string(),
        "priority" => "priority".to_string(),
        _ => default_fee_priority(),
    }
}
fn default_background_sync_profile() -> String {
    "balanced".to_string()
}
fn default_true() -> bool {
    true
}
fn default_bitcoin_stop_gap() -> u32 {
    10
}
fn default_refresh_frequency_minutes() -> u32 {
    5
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

/// Chains whose selected network is a testnet, by display name — their coins
/// have no price.
///
/// A free function over the settings rather than a service method, so a caller
/// can apply it in the same synchronous step that adopts a new state. Asking
/// core asynchronously left the render path briefly quoting a testnet at
/// mainnet prices right after a network switch.
///
/// This replaced `core_priced_chain(chain_name, bitcoin_mode, ethereum_mode)`,
/// which was the *second* copy of the rule that quoted Dogecoin testnet as
/// mainnet: it named two families and let every other chain through as priced.
#[uniffi::export]
pub fn core_unpriced_chain_names(settings: AppSettings) -> Vec<String> {
    crate::registry::Chain::mainnets()
        .filter(|chain| settings.network_chain(*chain).is_testnet())
        .map(|chain| chain.chain_display_name().to_string())
        .collect()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            fiat_currency_code: "USD".to_string(),
            pinned_dashboard_asset_symbols: Vec::new(),
            network_chain_by_family: std::collections::HashMap::new(),
            pricing_provider: default_pricing_provider(),
            fiat_rate_provider: default_fiat_rate_provider(),
            ethereum_rpc_endpoint: String::new(),
            etherscan_api_key: String::new(),
            monero_backend_base_url: String::new(),
            monero_backend_api_key: String::new(),
            bitcoin_esplora_endpoints: String::new(),
            bitcoin_stop_gap: default_bitcoin_stop_gap(),
            fee_priority_by_chain: std::collections::HashMap::new(),
            use_strict_rpc_only: false,
            background_sync_profile: default_background_sync_profile(),
            automatic_refresh_frequency_minutes: default_refresh_frequency_minutes(),
            use_price_alerts: default_true(),
            use_transaction_status_notifications: default_true(),
            use_large_movement_notifications: default_true(),
            large_movement_alert_percent_threshold: default_large_movement_percent(),
            large_movement_alert_usd_threshold: default_large_movement_usd(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreAppState {
    pub schema_version: u32,
    pub wallets: Vec<WalletSummary>,
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
}

impl Default for CoreAppState {
    fn default() -> Self {
        Self {
            schema_version: 2,
            wallets: Vec::new(),
            selected_wallet_id: None,
            settings: AppSettings::default(),
            address_book: Vec::new(),
            token_preferences: Vec::new(),
            price_alerts: Vec::new(),
        }
    }
}

/// Most tokens use 18 or fewer; the ceiling exists to stop a typo from
/// producing an unrenderable amount.
const MAX_TOKEN_DECIMALS: i32 = 30;

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
    PricingProvider { value: String },
    FiatRateProvider { value: String },
    EthereumRpcEndpoint { value: String },
    EtherscanApiKey { value: String },
    MoneroBackendBaseUrl { value: String },
    MoneroBackendApiKey { value: String },
    BitcoinEsploraEndpoints { value: String },
    BitcoinStopGap { value: u32 },
    /// `chain` is a registry display name; an unknown one is refused and an
    /// unknown `value` falls back to `normal`.
    FeePriority { chain: String, value: String },
    UseStrictRpcOnly { value: bool },
    BackgroundSyncProfile { value: String },
    AutomaticRefreshFrequencyMinutes { value: u32 },
    UsePriceAlerts { value: bool },
    UseTransactionStatusNotifications { value: bool },
    UseLargeMovementNotifications { value: bool },
    LargeMovementAlertPercentThreshold { value: f64 },
    LargeMovementAlertUsdThreshold { value: f64 },
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
    UpsertWallet {
        wallet: WalletSummary,
    },
    /// Update a wallet only if it is still stored.
    ///
    /// Balance refresh uses this: a refresh result that arrives after the user
    /// deleted the wallet must not bring it back. Creating is a separate
    /// intent, and `UpsertWallet` is the command for it.
    UpdateWalletIfPresent {
        wallet: WalletSummary,
    },
    SelectWallet {
        wallet_id: String,
    },
    RemoveWallet {
        wallet_id: String,
    },
    SetFiatCurrency {
        fiat_currency_code: String,
    },
    /// Change one settings field. Values are trimmed and bounded here, so a
    /// front end cannot store a stop gap of zero by writing to its own copy.
    SetAppSetting {
        update: AppSettingUpdate,
    },
    /// Replace the pinned dashboard set. Symbols are normalised to upper case
    /// and de-duplicated, first occurrence winning, so display order is the
    /// order the user pinned them in.
    SetPinnedDashboardAssets {
        symbols: Vec<String>,
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
    /// Replace the tracked-token list. Core clamps the decimal fields, so a
    /// caller cannot store a token that displays more places than it has.
    SetTokenPreferences {
        entries: Vec<crate::store::wallet_domain::CoreTokenPreferenceEntry>,
    },
    /// Add a recipient. `address` is normalized and validated by the reducer;
    /// a rejected entry produces an `addressBookRejected` event and no change.
    AddAddressBookEntry {
        id: String,
        name: String,
        chain_name: String,
        address: String,
        note: String,
    },
    /// Replace the price-alert list. Core normalises: an alert whose target is
    /// not a positive number cannot fire, so it is refused rather than stored.
    SetPriceAlerts {
        alerts: Vec<crate::store::PriceAlertEvaluationAlert>,
    },
    RenameAddressBookEntry {
        id: String,
        name: String,
    },
    RemoveAddressBookEntry {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StateEvent {
    pub kind: String,
    pub subject_id: Option<String>,
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

/// Apply a state command in place, returning only the events.
/// Avoids deep-cloning the entire CoreAppState on every mutation.
/// Apply one settings update, trimming strings and bounding numbers.
///
/// The clamps were `didSet` bodies on the iOS side — the only copy, so a value
/// out of range was only out of range where someone had remembered to check.
fn apply_app_setting(settings: &mut AppSettings, update: AppSettingUpdate) {
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
    match update {
        AppSettingUpdate::PricingProvider { value } => settings.pricing_provider = trimmed(value),
        AppSettingUpdate::FiatRateProvider { value } => settings.fiat_rate_provider = trimmed(value),
        AppSettingUpdate::EthereumRpcEndpoint { value } => {
            settings.ethereum_rpc_endpoint = trimmed(value)
        }
        AppSettingUpdate::EtherscanApiKey { value } => settings.etherscan_api_key = trimmed(value),
        AppSettingUpdate::MoneroBackendBaseUrl { value } => {
            settings.monero_backend_base_url = trimmed(value)
        }
        AppSettingUpdate::MoneroBackendApiKey { value } => {
            settings.monero_backend_api_key = trimmed(value)
        }
        // Not trimmed as a whole: this is a separated list, and the parser
        // trims each entry. Trimming the list would only drop its outer edges.
        AppSettingUpdate::BitcoinEsploraEndpoints { value } => {
            settings.bitcoin_esplora_endpoints = value
        }
        AppSettingUpdate::BitcoinStopGap { value } => {
            settings.bitcoin_stop_gap = clamp(value, BITCOIN_STOP_GAP_RANGE)
        }
        AppSettingUpdate::FeePriority { chain, value } => {
            let Some(chain) = crate::registry::Chain::from_display_name(&chain) else {
                return;
            };
            let value = normalized_fee_priority(&value);
            if value == default_fee_priority() {
                settings.fee_priority_by_chain.remove(chain.chain_display_name());
            } else {
                settings
                    .fee_priority_by_chain
                    .insert(chain.chain_display_name().to_string(), value);
            }
        }
        AppSettingUpdate::UseStrictRpcOnly { value } => settings.use_strict_rpc_only = value,
        AppSettingUpdate::BackgroundSyncProfile { value } => {
            settings.background_sync_profile = trimmed(value)
        }
        AppSettingUpdate::AutomaticRefreshFrequencyMinutes { value } => {
            settings.automatic_refresh_frequency_minutes =
                clamp(value, REFRESH_FREQUENCY_MINUTES_RANGE)
        }
        AppSettingUpdate::UsePriceAlerts { value } => settings.use_price_alerts = value,
        AppSettingUpdate::UseTransactionStatusNotifications { value } => {
            settings.use_transaction_status_notifications = value
        }
        AppSettingUpdate::UseLargeMovementNotifications { value } => {
            settings.use_large_movement_notifications = value
        }
        AppSettingUpdate::LargeMovementAlertPercentThreshold { value } => {
            settings.large_movement_alert_percent_threshold =
                clamp(value, LARGE_MOVEMENT_PERCENT_RANGE)
        }
        AppSettingUpdate::LargeMovementAlertUsdThreshold { value } => {
            settings.large_movement_alert_usd_threshold = clamp(value, LARGE_MOVEMENT_USD_RANGE)
        }
    }
}

pub fn reduce_state_in_place(state: &mut CoreAppState, command: StateCommand) -> Vec<StateEvent> {
    let mut events = Vec::new();

    match command {
        StateCommand::ReplaceState { state: next_state } => {
            *state = next_state;
            events.push(StateEvent {
                kind: "stateReplaced".to_string(),
                subject_id: None,
            });
        }
        StateCommand::UpsertWallet { wallet } => {
            let wallet_id = wallet.id.clone();
            if let Some(index) = state
                .wallets
                .iter()
                .position(|candidate| candidate.id == wallet_id)
            {
                state.wallets[index] = wallet;
                events.push(StateEvent {
                    kind: "walletUpdated".to_string(),
                    subject_id: Some(wallet_id.clone()),
                });
            } else {
                state.wallets.push(wallet);
                events.push(StateEvent {
                    kind: "walletAdded".to_string(),
                    subject_id: Some(wallet_id.clone()),
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
                    events.push(StateEvent {
                        kind: "walletUpdated".to_string(),
                        subject_id: Some(wallet_id),
                    });
                }
            }
        }
        StateCommand::SelectWallet { wallet_id } => {
            if state.wallets.iter().any(|wallet| wallet.id == wallet_id) {
                state.selected_wallet_id = Some(wallet_id.clone());
                events.push(StateEvent {
                    kind: "walletSelected".to_string(),
                    subject_id: Some(wallet_id),
                });
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
                events.push(StateEvent {
                    kind: "walletRemoved".to_string(),
                    subject_id: Some(wallet_id),
                });
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
            } else if !crate::send::flow::is_valid_send_address(chain_name.clone(), address.clone()) {
                Some(AddressBookRejection::InvalidAddress)
            } else if address_book_contains(state, &chain_name, &address, None) {
                Some(AddressBookRejection::DuplicateAddress)
            } else {
                None
            };

            match rejection {
                Some(reason) => events.push(StateEvent {
                    kind: "addressBookRejected".to_string(),
                    subject_id: Some(
                        serde_json::to_value(reason)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_default(),
                    ),
                }),
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
                    events.push(StateEvent {
                        kind: "addressBookEntryAdded".to_string(),
                        subject_id: Some(id),
                    });
                }
            }
        }
        StateCommand::RenameAddressBookEntry { id, name } => {
            let name = name.trim().to_string();
            if name.is_empty() {
                events.push(StateEvent {
                    kind: "addressBookRejected".to_string(),
                    subject_id: Some("emptyName".to_string()),
                });
            } else if let Some(entry) = state.address_book.iter_mut().find(|e| e.id == id) {
                if entry.name != name {
                    entry.name = name;
                    events.push(StateEvent {
                        kind: "addressBookEntryRenamed".to_string(),
                        subject_id: Some(id),
                    });
                }
            }
        }
        StateCommand::RemoveAddressBookEntry { id } => {
            let before = state.address_book.len();
            state.address_book.retain(|entry| entry.id != id);
            if state.address_book.len() != before {
                events.push(StateEvent {
                    kind: "addressBookEntryRemoved".to_string(),
                    subject_id: Some(id),
                });
            }
        }
        StateCommand::SetFiatCurrency { fiat_currency_code } => {
            let normalized = fiat_currency_code.trim().to_uppercase();
            if normalized != state.settings.fiat_currency_code {
                state.settings.fiat_currency_code = normalized.clone();
                events.push(StateEvent {
                    kind: "fiatCurrencyChanged".to_string(),
                    subject_id: Some(normalized),
                });
            }
        }
        StateCommand::SetAppSetting { update } => {
            let before = state.settings.clone();
            apply_app_setting(&mut state.settings, update);
            if state.settings != before {
                events.push(StateEvent {
                    kind: "appSettingChanged".to_string(),
                    subject_id: None,
                });
            }
        }
        StateCommand::SetPriceAlerts { alerts } => {
            let kept: Vec<_> = alerts
                .into_iter()
                .filter(|alert| alert.target_price > 0.0 && !alert.holding_key.trim().is_empty())
                .collect();
            if kept != state.price_alerts {
                state.price_alerts = kept;
                events.push(StateEvent {
                    kind: "priceAlertsChanged".to_string(),
                    subject_id: None,
                });
            }
        }
        StateCommand::SetTokenPreferences { entries } => {
            let normalized: Vec<_> = entries
                .into_iter()
                .map(|mut entry| {
                    entry.decimals = entry.decimals.clamp(0, MAX_TOKEN_DECIMALS);
                    // Displaying more places than the token has is meaningless,
                    // and negative places are not a thing.
                    entry.display_decimals = entry
                        .display_decimals
                        .map(|display| display.clamp(0, entry.decimals));
                    entry
                })
                .collect();
            if normalized != state.token_preferences {
                state.token_preferences = normalized;
                events.push(StateEvent {
                    kind: "tokenPreferencesChanged".to_string(),
                    subject_id: None,
                });
            }
        }
        StateCommand::SelectNetworkChain { chain_id } => {
            if let Some(chosen) = crate::registry::Chain::from_str_id(&chain_id) {
                let family = chosen.mainnet_counterpart();
                let before = state.settings.network_chain_by_family.clone();
                if chosen == family {
                    state
                        .settings
                        .network_chain_by_family
                        .remove(family.str_id());
                } else {
                    state
                        .settings
                        .network_chain_by_family
                        .insert(family.str_id().to_string(), chosen.str_id().to_string());
                }
                if before != state.settings.network_chain_by_family {
                    events.push(StateEvent {
                        kind: "networkChainChanged".to_string(),
                        subject_id: Some(chosen.str_id().to_string()),
                    });
                }
            }
        }
        StateCommand::SetPinnedDashboardAssets { symbols } => {
            let mut seen = std::collections::HashSet::new();
            let normalized: Vec<String> = symbols
                .into_iter()
                .filter_map(|symbol| {
                    let symbol = symbol.trim().to_uppercase();
                    if symbol.is_empty() || !seen.insert(symbol.clone()) {
                        return None;
                    }
                    Some(symbol)
                })
                .collect();
            if normalized != state.settings.pinned_dashboard_asset_symbols {
                state.settings.pinned_dashboard_asset_symbols = normalized;
                events.push(StateEvent {
                    kind: "pinnedDashboardAssetsChanged".to_string(),
                    subject_id: None,
                });
            }
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reduce_state(mut state: CoreAppState, command: StateCommand) -> StateTransition {
        let events = reduce_state_in_place(&mut state, command);
        StateTransition { state, events }
    }

    fn test_wallet(id: &str, chain: &str) -> WalletSummary {
        WalletSummary {
            id: id.to_string(),
            name: "Main".to_string(),
            is_watch_only: false,
            chain_name: chain.to_string(),
            include_in_portfolio_total: true,
            network_mode: if chain == "Bitcoin" {
                Some("mainnet".to_string())
            } else {
                None
            },
            xpub: None,
            derivation_preset: "standard".to_string(),
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
        assert_eq!(transition.events[0].kind, "walletAdded");
    }
}
