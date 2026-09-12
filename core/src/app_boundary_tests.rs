//! App-facing pure rules: semantic assertions at the exported boundary.
use crate::*;

#[test]
fn amount_display_never_renders_positive_dust_as_zero() {
    let dust = formatting::formatting_asset_amount_display(1e-10, 18);
    assert!(dust.below_threshold);
    assert_eq!(dust.threshold, 1e-8);
    assert_eq!(
        formatting::formatting_supported_decimal_places("Ethereum".into(), None),
        18
    );
    assert_eq!(
        formatting::formatting_supported_decimal_places("Ethereum".into(), Some(6)),
        6
    );
    assert_eq!(
        formatting::formatting_fiat_amount_rules("jpy".into()).minimum_visible,
        1.0
    );
    assert_eq!(
        formatting::formatting_fiat_amount_rules("USD".into()).minimum_visible,
        0.01
    );
    assert_eq!(
        formatting::formatting_token_preference_lookup_key(" Ethereum ".into(), " usdc ".into()),
        "Ethereum|USDC"
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
    let wallets = store::core_reset_dispatch(vec!["walletsAndSecrets".into()]);
    assert!(wallets.reset_wallets_and_secrets && wallets.reset_history_and_cache);
    assert!(wallets.clear_network_and_transport_caches);
    assert!(!wallets.reset_alerts_and_contacts && !wallets.reset_settings_and_endpoints);
    let settings = store::core_reset_dispatch(vec!["settingsAndEndpoints".into()]);
    assert!(settings.reset_settings_and_endpoints);
    assert!(!settings.reset_wallets_and_secrets && !settings.reset_history_and_cache);
    let unknown = store::core_reset_dispatch(vec!["typo".into()]);
    assert!(!unknown.reset_wallets_and_secrets && !unknown.clear_network_and_transport_caches);
}

#[test]
fn rpc_error_classification_preserves_retry_decisions() {
    use store::{core_ethereum_send_error_code as classify, EthereumSendErrorCode as E};
    assert_eq!(classify("RPC: NONCE TOO LOW".into()), E::NonceTooLow);
    assert_eq!(
        classify("replacement transaction underpriced".into()),
        E::ReplacementUnderpriced
    );
    assert_eq!(
        classify("insufficient funds for gas".into()),
        E::InsufficientFunds
    );
    assert_ne!(classify("provider unavailable".into()), E::AlreadyKnown);
}

#[test]
fn portfolio_signature_ignores_order_but_not_holding_boundaries() {
    use send::flow::portfolio_composition_signature as signature;
    assert_eq!(
        signature(vec!["b".into(), "a".into()]),
        signature(vec!["a".into(), "b".into()])
    );
    assert_ne!(
        signature(vec!["a|b".into(), "c".into()]),
        signature(vec!["a".into(), "b|c".into()])
    );
    assert_ne!(
        signature(vec!["a".into()]),
        signature(vec!["a".into(), "a".into()])
    );
}

#[test]
fn testnet_pricing_uses_the_selected_family_and_rejects_foreign_networks() {
    let mut settings = store::state::AppSettings::default();
    settings
        .network_chain_by_family
        .insert("ethereum".into(), "ethereum-sepolia".into());
    assert_eq!(
        store::state::core_unpriced_chain_names(settings.clone()),
        vec!["Ethereum"]
    );
    settings
        .network_chain_by_family
        .insert("ethereum".into(), "bitcoin-testnet-4".into());
    assert!(store::state::core_unpriced_chain_names(settings).is_empty());
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
