use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WalletAddress {
    pub chain_name: String,
    pub address: String,
    pub kind: String,
    pub derivation_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssetHolding {
    pub name: String,
    pub symbol: String,
    pub market_data_id: String,
    pub coin_gecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
    pub price_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WalletSummary {
    pub id: String,
    pub name: String,
    pub is_watch_only: bool,
    pub selected_chain: Option<String>,
    pub include_in_portfolio_total: bool,
    pub bitcoin_network_mode: String,
    pub dogecoin_network_mode: String,
    pub bitcoin_xpub: Option<String>,
    pub derivation_preset: String,
    pub derivation_paths: BTreeMap<String, String>,
    pub holdings: Vec<AssetHolding>,
    pub addresses: Vec<WalletAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub preferred_locale: String,
    pub fiat_currency_code: String,
    pub diagnostics_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            preferred_locale: "en".to_string(),
            fiat_currency_code: "USD".to_string(),
            diagnostics_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CoreAppState {
    pub schema_version: u32,
    pub wallets: Vec<WalletSummary>,
    pub selected_wallet_id: Option<String>,
    pub settings: AppSettings,
}

impl Default for CoreAppState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            wallets: Vec::new(),
            selected_wallet_id: None,
            settings: AppSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StateCommand {
    ReplaceState { state: CoreAppState },
    UpsertWallet { wallet: WalletSummary },
    SelectWallet { wallet_id: String },
    RemoveWallet { wallet_id: String },
    SetPreferredLocale { locale_identifier: String },
    SetFiatCurrency { fiat_currency_code: String },
    SetDiagnosticsEnabled { is_enabled: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StateEvent {
    pub kind: String,
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StateTransition {
    pub state: CoreAppState,
    pub events: Vec<StateEvent>,
}

pub fn reduce_state(mut state: CoreAppState, command: StateCommand) -> StateTransition {
    let mut events = Vec::new();

    match command {
        StateCommand::ReplaceState { state: next_state } => {
            state = next_state;
            events.push(StateEvent {
                kind: "stateReplaced".to_string(),
                subject_id: None,
            });
        }
        StateCommand::UpsertWallet { wallet } => {
            if let Some(index) = state
                .wallets
                .iter()
                .position(|candidate| candidate.id == wallet.id)
            {
                state.wallets[index] = wallet.clone();
                events.push(StateEvent {
                    kind: "walletUpdated".to_string(),
                    subject_id: Some(wallet.id.clone()),
                });
            } else {
                state.wallets.push(wallet.clone());
                events.push(StateEvent {
                    kind: "walletAdded".to_string(),
                    subject_id: Some(wallet.id.clone()),
                });
            }

            if state.selected_wallet_id.is_none() {
                state.selected_wallet_id = Some(wallet.id);
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
        StateCommand::SetPreferredLocale { locale_identifier } => {
            state.settings.preferred_locale = locale_identifier.clone();
            events.push(StateEvent {
                kind: "preferredLocaleChanged".to_string(),
                subject_id: Some(locale_identifier),
            });
        }
        StateCommand::SetFiatCurrency { fiat_currency_code } => {
            state.settings.fiat_currency_code = fiat_currency_code.clone();
            events.push(StateEvent {
                kind: "fiatCurrencyChanged".to_string(),
                subject_id: Some(fiat_currency_code),
            });
        }
        StateCommand::SetDiagnosticsEnabled { is_enabled } => {
            state.settings.diagnostics_enabled = is_enabled;
            events.push(StateEvent {
                kind: "diagnosticsToggleChanged".to_string(),
                subject_id: None,
            });
        }
    }

    StateTransition { state, events }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_wallet_selects_first_wallet() {
        let state = CoreAppState::default();
        let transition = reduce_state(
            state,
            StateCommand::UpsertWallet {
                wallet: WalletSummary {
                    id: "wallet-1".to_string(),
                    name: "Main".to_string(),
                    is_watch_only: false,
                    selected_chain: Some("Bitcoin".to_string()),
                    include_in_portfolio_total: true,
                    bitcoin_network_mode: "mainnet".to_string(),
                    dogecoin_network_mode: "mainnet".to_string(),
                    bitcoin_xpub: None,
                    derivation_preset: "standard".to_string(),
                    derivation_paths: BTreeMap::from([(
                        "Bitcoin".to_string(),
                        "m/84'/0'/0'/0/0".to_string(),
                    )]),
                    holdings: Vec::new(),
                    addresses: vec![WalletAddress {
                        chain_name: "Bitcoin".to_string(),
                        address: "bc1qexample".to_string(),
                        kind: "address".to_string(),
                        derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
                    }],
                },
            },
        );

        assert_eq!(
            transition.state.selected_wallet_id.as_deref(),
            Some("wallet-1")
        );
        assert_eq!(transition.events[0].kind, "walletAdded");
    }
}
