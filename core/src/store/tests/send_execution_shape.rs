use crate::registry::{Chain, SendFeeField};

#[test]
fn protocol_fee_fields_and_fallbacks_match_execution_requirements() {
    let cases: &[(&str, SendFeeField, f64)] = &[
        ("sui", SendFeeField::GasBudget, 0.0),
        ("aptos", SendFeeField::None, 0.0),
        ("ton", SendFeeField::None, 0.0),
        ("xrp", SendFeeField::None, 0.0),
        ("stellar", SendFeeField::None, 0.0),
        ("monero", SendFeeField::None, 0.0),
        ("cardano", SendFeeField::FeeAmount, 0.0),
        ("near", SendFeeField::None, 0.0),
        ("polkadot", SendFeeField::None, 0.0),
        ("bitcoin-cash", SendFeeField::FeeSats, 0.00001),
        ("bitcoin-sv", SendFeeField::FeeSats, 0.00001),
        ("litecoin", SendFeeField::FeeSats, 0.0001),
    ];
    for (name, field, fallback) in cases {
        let chain = Chain::from_str_id(name).expect(name);
        let shape = chain.send_execution_shape();
        assert_eq!(shape.fee_field, *field, "{name} fee_field");
        assert_eq!(shape.fee_fallback, *fallback, "{name} fee_fallback");
    }
}
