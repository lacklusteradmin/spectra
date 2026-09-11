use crate::state::{AppSettings, CoreAppState};

#[test]
fn settings_written_before_a_field_existed_still_load() {
    let legacy = r#"{"fiatCurrencyCode":"EUR"}"#;
    let settings: AppSettings =
        serde_json::from_str(legacy).expect("settings from before the field was added");
    assert_eq!(settings.fiat_currency_code, "EUR");
    assert!(settings.pinned_dashboard_asset_symbols.is_empty());
    // A field's serde default and `AppSettings::default()` are the same
    // function, so an absent field reads as a fresh install would, not as
    // its type's zero value. Notifications off is not the same as unset.
    assert!(settings.use_price_alerts);
    assert_eq!(settings.bitcoin_stop_gap, 10);
}

#[test]
fn state_written_before_token_preferences_existed_still_loads() {
    let legacy = r#"{
            "schemaVersion": 2,
            "wallets": [],
            "selectedWalletId": null,
            "settings": {"fiatCurrencyCode":"USD"},
            "addressBook": []
        }"#;
    let state: CoreAppState = serde_json::from_str(legacy).expect("legacy state");
    assert!(state.token_preferences.is_empty());
}
