//! App-facing pure rules: semantic assertions at the exported boundary.
use crate::*;

#[test]
fn derivation_editor_preserves_hardening_and_refuses_malformed_paths() {
    let path = "m/44'/60'/0'/0/7";
    let segments = parse_derivation_path(path.into()).unwrap();
    assert_eq!(format_derivation_path(segments), path);
    for invalid in ["m/2147483648", "m/-1", "m/nope", "m/44'//0"] {
        assert!(parse_derivation_path(invalid.into()).is_none(), "{invalid}");
    }
}

#[test]
fn reset_scopes_do_not_expand_to_unrelated_data() {
    use store::state::ResetScope;
    let wallets = store::reset_dispatch(vec![ResetScope::WalletsAndSecrets]);
    assert!(wallets.reset_wallets_and_secrets && wallets.reset_history_and_cache);
    assert!(!wallets.reset_alerts_and_contacts && !wallets.reset_settings_and_endpoints);
    let settings = store::reset_dispatch(vec![ResetScope::SettingsAndEndpoints]);
    assert!(settings.reset_settings_and_endpoints);
    assert!(!settings.reset_wallets_and_secrets && !settings.reset_history_and_cache);
    assert_eq!(ResetScope::from_raw("typo"), None);
    assert_eq!(
        ResetScope::from_raw("walletsAndSecrets"),
        Some(ResetScope::WalletsAndSecrets)
    );
}

#[test]
fn diagnostics_bundle_redacts_secrets_and_refuses_missing_fields() {
    use diagnostics::export::*;
    let bundle=DiagnosticsBundlePayload {
        schema_version:1,generated_at:123.0,
        environment:serde_json::from_value(serde_json::json!({"appVersion":"test","buildNumber":"1","osVersion":"test","localeIdentifier":"en","timeZoneIdentifier":"UTC","selectedFiatCurrency":"USD","walletCount":1,"transactionCount":0})).unwrap(),
        chain_degraded:std::collections::HashMap::from([("ethereum".into(),crate::service::ChainDegradation::Failed { message: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into() })]),
        chain_diagnostics_json:std::collections::HashMap::new(),
    };
    let encoded = diagnostics_bundle_to_json(bundle).unwrap();
    assert!(!encoded.contains("abandon abandon"));
    assert_eq!(
        diagnostics_bundle_from_json(encoded).unwrap().generated_at,
        123.0
    );
    assert!(diagnostics_bundle_from_json("{}".into()).is_none());
}

/// The preview a front end shows is the reducer's answer: trimmed, bounded,
/// and unchanged when refused.
#[test]
fn applying_a_setting_is_the_reducers_rule() {
    use store::state::{app_settings_applying, app_settings_defaults, AppSettingUpdate};
    let defaults = app_settings_defaults();
    let trimmed = app_settings_applying(
        defaults.clone(),
        AppSettingUpdate::AddCustomEndpoint {
            capabilities: crate::endpoint_capability_options(
                "monero".into(),
                crate::EndpointApi::MoneroDaemonRpc,
            ),
            chain_id: "monero".into(),
            api: "monero-daemon-rpc".into(),
            endpoint: "  https://wallet.example \n".into(),
        },
    );
    assert_eq!(
        trimmed.custom_endpoints[0].endpoint,
        "https://wallet.example"
    );
    let bounded = app_settings_applying(
        defaults.clone(),
        AppSettingUpdate::BitcoinStopGap { value: 0 },
    );
    assert!(bounded.bitcoin_stop_gap >= 1);
    let refused = app_settings_applying(
        defaults.clone(),
        AppSettingUpdate::TorCustomProxyAddress {
            value: "not a proxy".into(),
        },
    );
    assert_eq!(refused, defaults);
}
