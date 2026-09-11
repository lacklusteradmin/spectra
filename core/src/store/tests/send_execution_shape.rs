use crate::registry::{Chain, SendFeeField};

#[test]
fn the_shape_matches_what_the_call_sites_carried() {
    let cases: &[(&str, u8, bool, SendFeeField, f64)] = &[
        ("Sui", 6, false, SendFeeField::GasBudget, 0.0),
        ("Aptos", 6, false, SendFeeField::None, 0.0),
        ("TON", 6, false, SendFeeField::None, 0.0),
        ("XRP Ledger", 6, true, SendFeeField::None, 0.0),
        ("Stellar", 7, true, SendFeeField::None, 0.0),
        ("Monero", 6, false, SendFeeField::None, 0.0),
        ("Cardano", 6, false, SendFeeField::FeeAmount, 0.0),
        ("NEAR", 6, false, SendFeeField::None, 0.0),
        ("Polkadot", 6, false, SendFeeField::None, 0.0),
        ("Bitcoin Cash", 8, false, SendFeeField::FeeSats, 0.00001),
        ("Bitcoin SV", 8, false, SendFeeField::FeeSats, 0.00001),
        ("Litecoin", 8, false, SendFeeField::FeeSats, 0.0001),
    ];
    for (name, decimals, private_key, field, fallback) in cases {
        let chain = Chain::from_display_name(name).expect(name);
        let shape = chain.send_execution_shape();
        assert_eq!(shape.fee_decimals, *decimals, "{name} fee_decimals");
        assert_eq!(
            shape.supports_private_key, *private_key,
            "{name} private key"
        );
        assert_eq!(shape.fee_field, *field, "{name} fee_field");
        assert_eq!(shape.fee_fallback, *fallback, "{name} fee_fallback");
    }
}

/// A testnet signs the same way its mainnet does.
#[test]
fn a_testnet_sends_under_its_mainnets_shape() {
    for chain in Chain::all().filter(|c| c.is_testnet()) {
        let mainnet = chain.mainnet_counterpart();
        assert_eq!(
            chain.send_execution_shape().fee_field,
            mainnet.send_execution_shape().fee_field,
            "{} diverges from {}",
            chain.str_id(),
            mainnet.str_id()
        );
    }
}
