//! App-facing pure rules: semantic assertions at the exported boundary.
use crate::*;

#[test]
fn amount_display_never_renders_positive_dust_as_zero() {
    let dust = formatting::formatting_asset_amount_display(1e-10, 18);
    assert!(dust.below_threshold);
    assert_eq!(dust.threshold, 1e-8);
    assert_eq!(
        crate::tokens::token_display_decimals(Some("ethereum:native".into()), None),
        18
    );
    assert_eq!(
        crate::tokens::token_display_decimals(Some("custom:unlisted".into()), Some(6)),
        6
    );
    assert_eq!(
        formatting::formatting_fiat_amount_rules(crate::store::state::FiatCurrency::Jpy)
            .minimum_visible,
        1.0
    );
    assert_eq!(
        formatting::formatting_fiat_amount_rules(crate::store::state::FiatCurrency::Usd)
            .minimum_visible,
        0.01
    );
}

#[test]
fn derivation_editor_preserves_hardening_and_refuses_malformed_paths() {
    let path = "m/44'/60'/0'/0/7";
    let segments = core_parse_derivation_path(path.into()).unwrap();
    assert_eq!(core_derivation_path_string(segments), path);
    for invalid in ["m/2147483648", "m/-1", "m/nope", "m/44'//0"] {
        assert!(
            core_parse_derivation_path(invalid.into()).is_none(),
            "{invalid}"
        );
    }
}

#[test]
fn envelope_rejects_wrong_keys_and_tampering_at_the_app_boundary() {
    use store::seed_envelope::{decrypt_seed_envelope, encrypt_seed_envelope};
    let envelope = encrypt_seed_envelope("public test fixture".into(), vec![7; 32]).unwrap();
    assert_eq!(
        decrypt_seed_envelope(envelope.clone(), vec![7; 32]).unwrap(),
        "public test fixture"
    );
    assert!(decrypt_seed_envelope(envelope.clone(), vec![8; 32]).is_err());
    let mut payload: serde_json::Value = serde_json::from_slice(&envelope).unwrap();
    payload["nonce"] = serde_json::json!("AA==");
    assert!(decrypt_seed_envelope(serde_json::to_vec(&payload).unwrap(), vec![7; 32]).is_err());
    assert!(encrypt_seed_envelope("fixture".into(), vec![7; 31]).is_err());
}

#[test]
fn reset_scopes_do_not_expand_to_unrelated_data() {
    use store::state::ResetScope;
    let wallets = store::core_reset_dispatch(vec![ResetScope::WalletsAndSecrets]);
    assert!(wallets.reset_wallets_and_secrets && wallets.reset_history_and_cache);
    assert!(wallets.clear_network_and_transport_caches);
    assert!(!wallets.reset_alerts_and_contacts && !wallets.reset_settings_and_endpoints);
    let settings = store::core_reset_dispatch(vec![ResetScope::SettingsAndEndpoints]);
    assert!(settings.reset_settings_and_endpoints);
    assert!(!settings.reset_wallets_and_secrets && !settings.reset_history_and_cache);
    assert_eq!(ResetScope::from_raw("typo"), None);
    assert_eq!(
        ResetScope::from_raw("walletsAndSecrets"),
        Some(ResetScope::WalletsAndSecrets)
    );
}

#[test]
fn testnet_pricing_uses_concrete_network_identity() {
    let unpriced = store::state::core_unpriced_chain_names();
    assert!(unpriced.contains(&"Ethereum Sepolia".into()));
    assert!(unpriced.contains(&"ethereum-sepolia".into()));
    assert!(!unpriced.contains(&"Ethereum".into()));
    assert!(!unpriced.contains(&"ethereum".into()));
}

#[test]
fn diagnostics_bundle_redacts_secrets_and_refuses_missing_fields() {
    use diagnostics::export::*;
    let bundle=DiagnosticsBundlePayload {
        schema_version:1,generated_at:123.0,
        environment:serde_json::from_value(serde_json::json!({"appVersion":"test","buildNumber":"1","osVersion":"test","localeIdentifier":"en","timeZoneIdentifier":"UTC","selectedFiatCurrency":"USD","walletCount":1,"transactionCount":0})).unwrap(),
        chain_degraded_messages:std::collections::HashMap::from([("ethereum".into(),"abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into())]),
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
        AppSettingUpdate::EtherscanApiKey {
            value: "  key \n".into(),
        },
    );
    assert_eq!(trimmed.etherscan_api_key, "key");
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
