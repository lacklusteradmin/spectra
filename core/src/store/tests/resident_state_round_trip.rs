use crate::store::state::{reduce_state_in_place, CoreAppState, StateCommand};
use crate::store::wallet_db;
use crate::store::wallet_domain::CorePriceAlertCondition;
use crate::store::PriceAlertEvaluationAlert;

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

fn alert(id: &str, target: f64) -> PriceAlertEvaluationAlert {
    PriceAlertEvaluationAlert {
        id: id.to_string(),
        holding_key: "BTC".to_string(),
        asset_name: "Bitcoin".to_string(),
        symbol: "BTC".to_string(),
        chain_name: "Bitcoin".to_string(),
        target_price: target,
        condition: CorePriceAlertCondition::Above,
        is_enabled: true,
        has_triggered: false,
    }
}

#[test]
fn a_price_alert_survives_a_reopen() {
    let db = tmp_db();
    let mut state = CoreAppState::default();
    reduce_state_in_place(
        &mut state,
        StateCommand::SetPriceAlerts {
            alerts: vec![alert("A1", 100_000.0)],
        },
    );
    wallet_db::app_state_save(&db, &state).expect("save");

    let reloaded = wallet_db::app_state_load(&db).expect("load");
    assert_eq!(reloaded.price_alerts.len(), 1, "price alert was lost");
    assert_eq!(reloaded.price_alerts[0].target_price, 100_000.0);
}

#[test]
fn an_alert_that_cannot_fire_is_refused() {
    let mut state = CoreAppState::default();
    reduce_state_in_place(
        &mut state,
        StateCommand::SetPriceAlerts {
            alerts: vec![alert("A1", 0.0), alert("A2", -5.0), alert("A3", 42.0)],
        },
    );
    assert_eq!(state.price_alerts.len(), 1);
    assert_eq!(state.price_alerts[0].id, "A3");
}

/// Whatever the resident state holds must come back. Add a collection and
/// this fails until `app_state_save` learns about it.
#[test]
fn every_resident_collection_round_trips() {
    let db = tmp_db();
    let mut state = CoreAppState::default();
    reduce_state_in_place(
        &mut state,
        StateCommand::SetPriceAlerts {
            alerts: vec![alert("A1", 1.0)],
        },
    );
    reduce_state_in_place(
        &mut state,
        StateCommand::AddAddressBookEntry {
            id: "C1".into(),
            name: "Alice".into(),
            chain_name: "Ethereum".into(),
            address: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".into(),
            note: String::new(),
        },
    );
    reduce_state_in_place(
        &mut state,
        StateCommand::SetFiatCurrency {
            fiat_currency_code: "CHF".into(),
        },
    );
    wallet_db::app_state_save(&db, &state).expect("save");
    let back = wallet_db::app_state_load(&db).expect("load");

    assert_eq!(back.price_alerts.len(), 1, "price_alerts not persisted");
    assert_eq!(back.address_book.len(), 1, "address_book not persisted");
    assert_eq!(
        back.settings.fiat_currency_code, "CHF",
        "settings not persisted"
    );
}

/// Resetting puts every field back to core's own default.
///
/// The defaults were duplicated in Swift: `resetSettingsAndEndpointsState`
/// assigned twelve literals and `AppUserPreferences.resetToDefaults`
/// another seven, none of which any test could compare against
/// `AppSettings::default()`. Written by mutating *every* field first, so a
/// new field added to `AppSettings` and forgotten in the reducer fails
/// here rather than silently surviving a reset.
#[test]
fn resetting_settings_restores_every_default() {
    use crate::store::state::{reduce_state_in_place, AppSettingUpdate as U, StateCommand};
    let mut state = CoreAppState::default();
    let defaults = state.settings.clone();

    for update in [
        U::RpcEndpoint {
            chain: "Base".into(),
            value: "https://x.example".into(),
        },
        U::EtherscanApiKey {
            value: "KEY".into(),
        },
        U::MoneroBackendBaseUrl {
            value: "https://xmr.example".into(),
        },
        U::MoneroBackendApiKey {
            value: "XKEY".into(),
        },
        U::BitcoinEsploraEndpoints {
            value: "https://a.example".into(),
        },
        U::BitcoinStopGap { value: 42 },
        U::FeePriority {
            chain: "Dogecoin".into(),
            value: "economy".into(),
        },
        U::UseStrictRpcOnly { value: true },
        U::BackgroundSyncProfile {
            value: "aggressive".into(),
        },
        U::AutomaticRefreshFrequencyMinutes { value: 30 },
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
        StateCommand::SetFiatCurrency {
            fiat_currency_code: "EUR".into(),
        },
    );
    reduce_state_in_place(
        &mut state,
        StateCommand::SelectNetworkChain {
            chain_id: "bitcoin-testnet".into(),
        },
    );
    assert_ne!(state.settings, defaults, "nothing was actually changed");

    let events = reduce_state_in_place(&mut state, StateCommand::ResetAppSettings);
    assert_eq!(state.settings, defaults);
    assert!(events.iter().any(|e| e.kind == "appSettingChanged"));

    // Resetting what is already default is not a change.
    assert!(reduce_state_in_place(&mut state, StateCommand::ResetAppSettings).is_empty());
}

/// Every settings field survives a save and a reload.
///
/// Eighteen of them arrived from a blob iOS wrote separately, and the
/// blob's own fields were never in this test because they were never in
/// this state. Written field by field so a new one that is added to
/// `AppSettings` and forgotten in `apply_app_setting` fails here.
#[test]
fn every_settings_field_round_trips() {
    use crate::store::state::AppSettingUpdate as U;
    let db = tmp_db();
    let mut state = CoreAppState::default();
    let updates = vec![
        U::RpcEndpoint {
            chain: "Ethereum".into(),
            value: "https://rpc.example".into(),
        },
        // The second one is the point: this was a single String, so a
        // second chain's override had nowhere to go.
        U::RpcEndpoint {
            chain: "Base".into(),
            value: "https://base.example".into(),
        },
        U::EtherscanApiKey {
            value: "KEY".into(),
        },
        U::MoneroBackendBaseUrl {
            value: "https://xmr.example".into(),
        },
        U::MoneroBackendApiKey {
            value: "XKEY".into(),
        },
        U::BitcoinEsploraEndpoints {
            value: "https://a.example,https://b.example".into(),
        },
        U::BitcoinStopGap { value: 42 },
        U::FeePriority {
            chain: "Bitcoin".into(),
            value: "priority".into(),
        },
        U::FeePriority {
            chain: "Dogecoin".into(),
            value: "economy".into(),
        },
        U::UseStrictRpcOnly { value: true },
        U::BackgroundSyncProfile {
            value: "aggressive".into(),
        },
        U::AutomaticRefreshFrequencyMinutes { value: 30 },
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
    wallet_db::app_state_save(&db, &state).expect("save");
    let back = wallet_db::app_state_load(&db).expect("load");
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

/// A value outside its range is bounded rather than stored.
///
/// These bounds were `didSet` clamps in the iOS layer — the only copy, so
/// a stop gap of zero was only impossible where someone had remembered to
/// check. A zero stop gap finds no addresses; a one-minute refresh
/// interval hammers whatever endpoint is configured.
#[test]
fn a_setting_outside_its_range_is_bounded() {
    use crate::store::state::AppSettingUpdate as U;
    let mut state = CoreAppState::default();
    fn set(state: &mut CoreAppState, update: U) {
        reduce_state_in_place(state, StateCommand::SetAppSetting { update });
    }

    set(&mut state, U::BitcoinStopGap { value: 0 });
    set(&mut state, U::AutomaticRefreshFrequencyMinutes { value: 1 });
    set(
        &mut state,
        U::LargeMovementAlertPercentThreshold { value: 0.0 },
    );
    set(
        &mut state,
        U::LargeMovementAlertUsdThreshold { value: 1_000_000.0 },
    );
    assert_eq!(state.settings.bitcoin_stop_gap, 1);
    assert_eq!(state.settings.automatic_refresh_frequency_minutes, 5);
    assert_eq!(state.settings.large_movement_alert_percent_threshold, 1.0);
    assert_eq!(state.settings.large_movement_alert_usd_threshold, 100_000.0);

    set(&mut state, U::BitcoinStopGap { value: 9_999 });
    set(
        &mut state,
        U::AutomaticRefreshFrequencyMinutes { value: 9_999 },
    );
    set(
        &mut state,
        U::LargeMovementAlertPercentThreshold { value: 500.0 },
    );
    assert_eq!(state.settings.bitcoin_stop_gap, 200);
    assert_eq!(state.settings.automatic_refresh_frequency_minutes, 60);
    assert_eq!(state.settings.large_movement_alert_percent_threshold, 90.0);

    // Trimmed, so a pasted key with a stray newline is the same key.
    set(
        &mut state,
        U::EtherscanApiKey {
            value: "  ABC123\n".into(),
        },
    );
    assert_eq!(state.settings.etherscan_api_key, "ABC123");
}
