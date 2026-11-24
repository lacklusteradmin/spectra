use serde::{Deserialize, Serialize};

pub use super::address_book::{AddressBookEntry, AddressBookRejection};
pub use super::settings::{
    AppSettingUpdate, AppSettings, BITCOIN_STOP_GAP_RANGE, BackgroundSyncProfile,
    DEFAULT_PINNED_DASHBOARD_ASSETS, FiatCurrency, InputBounds, LARGE_MOVEMENT_PERCENT_RANGE,
    LARGE_MOVEMENT_USD_RANGE, ResetScope, app_settings_applying, app_settings_defaults,
    fiat_currency_codes, input_bounds,
};
pub use super::token_preferences::{
    MAX_TOKEN_DECIMALS, TokenPreferenceRejection, built_in_token_preferences, with_custom_tokens,
};
pub use super::wallet_domain::{WalletSigning, WalletState};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResidentState {
    /// Session-local committed-state version. Never persisted or supplied by a client.
    #[serde(skip)]
    pub revision: u64,
    pub movement_baseline: Option<crate::service::PortfolioMovementBaseline>,
    pub quotes: crate::service::QuoteRefreshState,
    pub diagnostics: crate::service::DiagnosticState,
    pub schema_version: u32,
    pub wallets: Vec<WalletState>,
    pub selected_wallet_id: Option<String>,
    pub settings: AppSettings,
    /// Saved recipients, most recently added first.
    pub address_book: Vec<AddressBookEntry>,
    /// The built-in catalog's tokens and the ones the user added, in
    /// `sort_token_preferences` order. Only the user's are stored; the
    /// catalog's are its own each time state is built.
    pub token_preferences: Vec<crate::store::wallet_domain::TokenPreferenceEntry>,
    /// Price alerts. Domain state by rule 4 — losing one on restart means an
    /// alert the user set never fires.
    pub price_alerts: Vec<crate::store::PriceAlertRule>,
    /// USD → display-currency cross rates, as `code -> rate`.
    ///
    /// Every quoted amount passes through these, and they are what the app
    /// shows while a refresh is in flight or the provider is down — so losing
    /// them on restart means every non-USD balance renders as USD until a
    /// network call lands. One front end kept them in its own SQLite blob,
    /// seeded from an older `UserDefaults` key that still won a race at launch.
    pub fiat_rates_from_usd: std::collections::HashMap<String, f64>,
}

pub(crate) const APP_STATE_SCHEMA_VERSION: u32 = 3;

impl Default for ResidentState {
    fn default() -> Self {
        Self {
            revision: 0,
            movement_baseline: None,
            quotes: Default::default(),
            schema_version: APP_STATE_SCHEMA_VERSION,
            diagnostics: Default::default(),
            wallets: Vec::new(),
            selected_wallet_id: None,
            settings: AppSettings::default(),
            address_book: Vec::new(),
            token_preferences: built_in_token_preferences(),
            price_alerts: Vec::new(),
            fiat_rates_from_usd: std::collections::HashMap::new(),
        }
    }
}

/// An intent to change the resident state: what a front end may ask for.
///
/// None carries a whole wallet or the whole state. A front end changes a
/// wallet through these intents and core decides the record; core's own
/// writes of a whole wallet go through `WalletService::upsert_wallet`, which
/// does not cross the FFI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Enum)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StateCommand {
    RenameWallet {
        wallet_id: String,
        name: String,
    },
    SetWalletPortfolioInclusion {
        wallet_id: String,
        included: bool,
    },
    /// Hide one of a wallet's holdings from its total and the portfolio, or
    /// show it again. Only a holding the wallet has can be hidden.
    SetHoldingHidden {
        wallet_id: String,
        deployment_id: String,
        hidden: bool,
    },
    SelectWallet {
        wallet_id: String,
    },
    RemoveWallet {
        wallet_id: String,
    },
    /// Change one settings field. Values are trimmed and bounded here, so a
    /// front end cannot store a stop gap of zero by writing to its own copy.
    SetAppSetting {
        update: AppSettingUpdate,
    },
    /// Reset core-owned settings to `AppSettings::default()`.
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
    SetDashboardAssetPinned {
        token_id: String,
        is_pinned: bool,
    },
    /// Add a custom token under `standard`, one of the network's token
    /// protocols — the network's only one when `None`. Trim input, uppercase
    /// the symbol, validate the identifier by that protocol's rule, and
    /// reject duplicates.
    /// Rejection emits `tokenPreferenceRejected` without changing state.
    AddCustomToken {
        #[uniffi(default = None)]
        standard: Option<crate::registry::TokenStandard>,
        chain_id: crate::registry::Chain,
        symbol: String,
        name: String,
        contract: String,
        coingecko_id: String,
        coinpaprika_id: String,
        decimals: Option<u32>,
    },
    UpdateCustomToken {
        chain_id: crate::registry::Chain,
        contract: String,
        symbol: String,
        name: String,
        coingecko_id: String,
        coinpaprika_id: String,
        decimals: Option<u32>,
    },
    /// Forget a custom token. A built-in is the catalog's, not the user's.
    RemoveCustomToken {
        chain_id: crate::registry::Chain,
        contract: String,
    },
    /// Change a custom token's precision. Out of range is refused, not
    /// clamped: a clamp reads every later balance at the wrong scale.
    SetCustomTokenDecimals {
        chain_id: crate::registry::Chain,
        contract: String,
        decimals: u32,
    },
    /// Back to the catalog's own list, with every custom token dropped.
    ResetTokenPreferences,
    /// Add a recipient. `address` is normalized and validated by the reducer,
    /// which also assigns the entry's id; a rejected entry produces an
    /// `addressBookRejected` event and no change.
    AddAddressBookEntry {
        name: String,
        chain_id: crate::registry::Chain,
        address: String,
        note: String,
    },
    AddPriceAlert {
        holding_key: String,
        /// A decimal in `currency`, as the user typed it with `.` for the
        /// decimal point. Core parses it; a front end does not.
        target_price: String,
        currency: FiatCurrency,
        condition: crate::store::wallet_domain::PriceAlertCondition,
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
/// The serialized form keeps `kind` beside each variant's fields.
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
    PinnedDashboardAssetsChanged,
    QuotesUpdated,
    FiatRatesChanged,
    DiagnosticsChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StateTransition {
    pub state: ResidentState,
    pub events: Vec<StateEvent>,
}

/// Store `wallet` whole: replace the stored wallet with its id, or add it
/// and select it when nothing is selected. Core's own writes only; see
/// `WalletService::upsert_wallet`.
pub(crate) fn upsert_wallet_in_place(
    state: &mut ResidentState,
    wallet: WalletState,
) -> Vec<StateEvent> {
    let wallet_id = wallet.id.clone();
    let event = if let Some(index) = state
        .wallets
        .iter()
        .position(|candidate| candidate.id == wallet_id)
    {
        state.wallets[index] = wallet;
        StateEvent::WalletUpdated {
            wallet_id: wallet_id.clone(),
        }
    } else {
        state.wallets.push(wallet);
        StateEvent::WalletAdded {
            wallet_id: wallet_id.clone(),
        }
    };
    if state.selected_wallet_id.is_none() {
        state.selected_wallet_id = Some(wallet_id);
    }
    vec![event]
}

pub fn reduce_state_in_place(state: &mut ResidentState, command: StateCommand) -> Vec<StateEvent> {
    let mut events = Vec::new();

    match command {
        StateCommand::RenameWallet { wallet_id, name } => {
            let name = name.trim();
            if !name.is_empty()
                && let Some(wallet) = state.wallets.iter_mut().find(|w| w.id == wallet_id)
                && wallet.name != name
            {
                wallet.name = name.to_owned();
                events.push(StateEvent::WalletUpdated { wallet_id });
            }
        }
        StateCommand::SetWalletPortfolioInclusion {
            wallet_id,
            included,
        } => {
            if let Some(wallet) = state.wallets.iter_mut().find(|w| w.id == wallet_id)
                && wallet.include_in_portfolio_total != included
            {
                wallet.include_in_portfolio_total = included;
                events.push(StateEvent::WalletUpdated { wallet_id });
            }
        }
        StateCommand::SetHoldingHidden {
            wallet_id,
            deployment_id,
            hidden,
        } => {
            if let Some(wallet) = state.wallets.iter_mut().find(|w| w.id == wallet_id) {
                let listed = wallet.hidden_holdings.contains(&deployment_id);
                let holds = wallet
                    .holdings
                    .iter()
                    .any(|holding| holding.deployment_id() == deployment_id);
                if hidden && !listed && holds {
                    wallet.hidden_holdings.push(deployment_id);
                    wallet.hidden_holdings.sort();
                    events.push(StateEvent::WalletUpdated { wallet_id });
                } else if !hidden && listed {
                    wallet.hidden_holdings.retain(|id| *id != deployment_id);
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
        command @ (StateCommand::AddAddressBookEntry { .. }
        | StateCommand::RenameAddressBookEntry { .. }
        | StateCommand::RemoveAddressBookEntry { .. }) => {
            super::address_book::reduce(state, command, &mut events)
        }
        command @ (StateCommand::SetAppSetting { .. }
        | StateCommand::ResetAppSettings
        | StateCommand::ResetPinnedDashboardAssets
        | StateCommand::SetPinnedDashboardAssets { .. }
        | StateCommand::SetDashboardAssetPinned { .. }) => {
            super::settings::reduce(state, command, &mut events)
        }
        command @ (StateCommand::AddCustomToken { .. }
        | StateCommand::UpdateCustomToken { .. }
        | StateCommand::RemoveCustomToken { .. }
        | StateCommand::SetCustomTokenDecimals { .. }
        | StateCommand::ResetTokenPreferences) => {
            super::token_preferences::reduce(state, command, &mut events)
        }
    }

    events
}

#[cfg(test)]
#[path = "tests/state_reducer.rs"]
mod tests;
