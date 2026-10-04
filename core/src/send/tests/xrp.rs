use super::*;

#[test]
fn signed_payment_matches_official_xrpl_codec_and_signature() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/xrp-payment.json")).unwrap();
    let tx = &fixture["transaction"];
    let mut key = [0; 32];
    key[31] = 1;
    let signed = build_signed_payment(
        tx["Account"].as_str().unwrap(),
        tx["Destination"].as_str().unwrap(),
        tx["Amount"].as_str().unwrap().parse().unwrap(),
        tx["Fee"].as_str().unwrap().parse().unwrap(),
        tx["Sequence"].as_u64().unwrap().try_into().unwrap(),
        &key,
        tx["SigningPubKey"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(signed, fixture["signed_hex"]);
}

#[test]
fn refuses_native_amounts_that_would_corrupt_asset_flags() {
    for drops in [0, 100_000_000_000_000_001, u128::from(u64::MAX), u128::MAX] {
        assert!(validate_drops(drops).is_err(), "{drops}");
    }
    assert!(validate_drops(1).is_ok());
    assert!(validate_drops(100_000_000_000_000_000).is_ok());
}
