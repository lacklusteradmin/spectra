use crate::store::state::{CoreAppState, StateCommand, reduce_state_in_place};
use crate::store::wallet_domain::CorePriceAlertCondition;
use crate::wallet_db;

fn tmp_db() -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-resident-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

#[test]
fn an_alert_that_cannot_fire_is_refused() {
    let mut state = CoreAppState::default();
    for target in ["0", "-5", "inf", "NaN", "1e3", "", "0.000001"] {
        reduce_state_in_place(
            &mut state,
            StateCommand::AddPriceAlert {
                holding_key: "bitcoin:native".into(),
                target_price: target.into(),
                currency: crate::store::state::FiatCurrency::Usd,
                condition: CorePriceAlertCondition::Above,
            },
        );
    }
    assert_eq!(state.price_alerts.len(), 1);
    assert_eq!(state.price_alerts[0].target_price, 0.000001);
}

/// Alerts and contacts retain their full payload alongside the chosen currency.
#[test]
fn alerts_contacts_and_currency_survive_reopening() {
    let db = tmp_db();
    let mut state = CoreAppState::default();
    reduce_state_in_place(
        &mut state,
        StateCommand::AddPriceAlert {
            holding_key: "bitcoin:native".into(),
            target_price: "1".into(),
            currency: crate::store::state::FiatCurrency::Usd,
            condition: CorePriceAlertCondition::Above,
        },
    );
    reduce_state_in_place(
        &mut state,
        StateCommand::AddAddressBookEntry {
            name: "Alice".into(),
            chain_id: crate::registry::Chain::Ethereum,
            address: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".into(),
            note: String::new(),
        },
    );
    reduce_state_in_place(
        &mut state,
        StateCommand::SetAppSetting {
            update: crate::store::state::AppSettingUpdate::FiatCurrency {
                value: crate::store::state::FiatCurrency::Chf,
            },
        },
    );
    wallet_db::app_state_save(&crate::wallet_db::WalletDatabase::new(&db), &state).expect("save");
    let back =
        wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db)).expect("load");

    assert_eq!(state.price_alerts.len(), 1);
    assert_eq!(state.address_book.len(), 1);
    assert_eq!(back.price_alerts, state.price_alerts);
    assert_eq!(back.address_book, state.address_book);
    assert_eq!(
        back.settings.fiat_currency,
        crate::store::state::FiatCurrency::Chf,
        "settings not persisted"
    );
}

/// Resetting puts every field back to core's own default.
///
/// Written by mutating *every* field first, so a new field added to
/// `AppSettings` and forgotten in the reducer fails here rather than silently
/// surviving a reset.
#[test]
fn resetting_settings_restores_every_default() {
    use crate::store::state::{AppSettingUpdate as U, StateCommand, reduce_state_in_place};
    let mut state = CoreAppState::default();
    let defaults = state.settings.clone();

    for update in [
        U::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                crate::registry::Chain::Base,
                crate::EndpointApi::EvmJsonRpc,
            ),
            chain_id: crate::registry::Chain::Base,
            api: "evm-json-rpc".into(),
            endpoint: "https://x.example".into(),
        },
        U::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                crate::registry::Chain::Monero,
                crate::EndpointApi::MoneroDaemonRpc,
            ),
            chain_id: crate::registry::Chain::Monero,
            api: "monero-daemon-rpc".into(),
            endpoint: "https://xmr.example".into(),
        },
        U::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                crate::registry::Chain::Bitcoin,
                crate::EndpointApi::Esplora,
            ),
            chain_id: crate::registry::Chain::Bitcoin,
            api: "esplora".into(),
            endpoint: "https://a.example".into(),
        },
        U::BitcoinStopGap { value: 42 },
        U::BackgroundSyncProfile {
            value: crate::store::state::BackgroundSyncProfile::Aggressive,
        },
        U::UsePriceAlerts { value: false },
        U::UseTransactionStatusNotifications { value: false },
        U::UseLargeMovementNotifications { value: false },
        U::LargeMovementAlertPercentThreshold { value: 25.0 },
        U::LargeMovementAlertUsdThreshold { value: 500.0 },
    ] {
        reduce_state_in_place(&mut state, StateCommand::SetAppSetting { update });
    }
    reduce_state_in_place(
        &mut state,
        StateCommand::SetAppSetting {
            update: crate::store::state::AppSettingUpdate::FiatCurrency {
                value: crate::store::state::FiatCurrency::Eur,
            },
        },
    );
    assert_ne!(state.settings, defaults, "nothing was actually changed");

    let events = reduce_state_in_place(&mut state, StateCommand::ResetAppSettings);
    assert_eq!(state.settings, defaults);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, crate::store::state::StateEvent::AppSettingChanged))
    );

    // Resetting what is already default is not a change.
    assert!(reduce_state_in_place(&mut state, StateCommand::ResetAppSettings).is_empty());
}

/// Every settings field survives a save and a reload. Written field by field
/// so a new one that is added to `AppSettings` and forgotten in
/// `apply_app_setting` fails here.
#[test]
fn every_settings_field_round_trips() {
    use crate::store::state::AppSettingUpdate as U;
    let db = tmp_db();
    let mut state = CoreAppState::default();
    let updates = vec![
        U::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                crate::registry::Chain::Ethereum,
                crate::EndpointApi::EvmJsonRpc,
            ),
            chain_id: crate::registry::Chain::Ethereum,
            api: "evm-json-rpc".into(),
            endpoint: "https://rpc.example".into(),
        },
        // The second one is the point: each chain keeps its own override.
        U::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                crate::registry::Chain::Base,
                crate::EndpointApi::EvmJsonRpc,
            ),
            chain_id: crate::registry::Chain::Base,
            api: "evm-json-rpc".into(),
            endpoint: "https://base.example".into(),
        },
        U::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                crate::registry::Chain::Monero,
                crate::EndpointApi::MoneroDaemonRpc,
            ),
            chain_id: crate::registry::Chain::Monero,
            api: "monero-daemon-rpc".into(),
            endpoint: "https://xmr.example".into(),
        },
        U::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                crate::registry::Chain::Bitcoin,
                crate::EndpointApi::Esplora,
            ),
            chain_id: crate::registry::Chain::Bitcoin,
            api: "esplora".into(),
            endpoint: "https://a.example".into(),
        },
        U::BitcoinStopGap { value: 42 },
        U::BackgroundSyncProfile {
            value: crate::store::state::BackgroundSyncProfile::Aggressive,
        },
        U::UsePriceAlerts { value: false },
        U::UseTransactionStatusNotifications { value: false },
        U::UseLargeMovementNotifications { value: false },
        U::LargeMovementAlertPercentThreshold { value: 25.0 },
        U::LargeMovementAlertUsdThreshold { value: 2_500.0 },
    ];
    for update in updates {
        reduce_state_in_place(&mut state, StateCommand::SetAppSetting { update });
    }
    let written = state.settings.clone();
    wallet_db::app_state_save(&crate::wallet_db::WalletDatabase::new(&db), &state).expect("save");
    let back =
        wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db)).expect("load");
    assert_eq!(
        back.settings, written,
        "a settings field did not round trip"
    );
    assert_ne!(
        back.settings,
        crate::store::state::AppSettings::default(),
        "the updates did not change anything"
    );
}

/// A value outside its range is bounded rather than stored. A zero stop gap
/// finds no addresses; a one-minute refresh interval hammers whatever
/// endpoint is configured.
#[test]
fn a_setting_outside_its_range_is_bounded() {
    use crate::store::state::AppSettingUpdate as U;
    let mut state = CoreAppState::default();
    fn set(state: &mut CoreAppState, update: U) {
        reduce_state_in_place(state, StateCommand::SetAppSetting { update });
    }

    set(&mut state, U::BitcoinStopGap { value: 0 });
    set(
        &mut state,
        U::LargeMovementAlertPercentThreshold { value: 0.0 },
    );
    set(
        &mut state,
        U::LargeMovementAlertUsdThreshold { value: 1_000_000.0 },
    );
    assert_eq!(state.settings.bitcoin_stop_gap, 1);
    assert_eq!(state.settings.large_movement_alert_percent_threshold, 1.0);
    assert_eq!(state.settings.large_movement_alert_usd_threshold, 100_000.0);

    set(&mut state, U::BitcoinStopGap { value: 9_999 });
    set(
        &mut state,
        U::LargeMovementAlertPercentThreshold { value: 500.0 },
    );
    assert_eq!(state.settings.bitcoin_stop_gap, 200);
    assert_eq!(state.settings.large_movement_alert_percent_threshold, 90.0);

    // Trimmed, so a pasted URL with a stray newline is the same URL.
    set(
        &mut state,
        U::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                crate::registry::Chain::Monero,
                crate::EndpointApi::MoneroDaemonRpc,
            ),
            chain_id: crate::registry::Chain::Monero,
            api: "monero-daemon-rpc".into(),
            endpoint: "  https://wallet.example\n".into(),
        },
    );
    assert_eq!(
        state.settings.custom_endpoints.last().unwrap().endpoint,
        "https://wallet.example"
    );
}
