use crate::registry::{Chain, SendFeeField};

#[test]
fn protocol_fee_fields_and_fallbacks_match_execution_requirements() {
    let cases: &[(&str, SendFeeField, Option<&str>)] = &[
        ("sui", SendFeeField::GasBudget, None),
        ("aptos", SendFeeField::None, None),
        ("ton", SendFeeField::None, None),
        ("xrp", SendFeeField::None, None),
        ("stellar", SendFeeField::None, None),
        ("monero", SendFeeField::None, None),
        ("cardano", SendFeeField::FeeAmount, None),
        ("near", SendFeeField::None, None),
        ("polkadot", SendFeeField::None, None),
        ("bitcoin-cash", SendFeeField::FeeSats, Some("0.00001")),
        ("bitcoin-sv", SendFeeField::FeeSats, Some("0.00001")),
        ("litecoin", SendFeeField::FeeSats, Some("0.0001")),
    ];
    for (name, field, fallback) in cases {
        let chain = Chain::from_str_id(name).expect(name);
        let shape = chain.send_execution_shape();
        assert_eq!(shape.fee_field, *field, "{name} fee_field");
        assert_eq!(shape.fee_fallback, *fallback, "{name} fee_fallback");
    }
}
