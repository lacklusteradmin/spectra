use super::*;

#[test]
fn values_normalize_to_sixteen_digit_mantissas() {
    let value = |text: &str| IouValue::parse(text).unwrap();
    assert_eq!(
        value("1"),
        IouValue {
            negative: false,
            mantissa: 1_000_000_000_000_000,
            exponent: -15
        }
    );
    assert_eq!(value("100"), value("1e2"));
    assert_eq!(value("0.001"), value("1e-3"));
    assert_eq!(value("-1.5").to_decimal(), "-1.5");
    assert_eq!(
        value("1234567890123456e-30").to_decimal(),
        "0.000000000000001234567890123456"
    );
    assert_eq!(value("9999999999999999e80").to_decimal().len(), 96);
    assert_eq!(
        value("1000000000000000e-96").to_decimal(),
        format!("0.{}1", "0".repeat(80))
    );
    assert!(value("0").is_zero() && value("-0").is_zero() && !value("-0").negative);
    for bad in [
        "",
        ".",
        "1.2.3",
        "1e",
        "abc",
        "12345678901234567",
        "1e-82",
        "1e97",
        "+1",
    ] {
        assert_eq!(IouValue::parse(bad), None, "{bad}");
    }
    // A person's decimal: positive, and only what the ledger holds exactly.
    assert_eq!(IouValue::from_decimal("12.5"), Some(value("12.5")));
    assert_eq!(IouValue::from_decimal("1.0000000000000001"), None);
    assert_eq!(IouValue::from_decimal("-1"), None);
}

#[test]
fn values_order_by_magnitude_and_sign() {
    let value = |text: &str| IouValue::parse(text).unwrap();
    assert!(value("2") > value("1.5"));
    assert!(value("0.01") > value("1e-50"));
    assert!(value("0") < value("1e-81"));
    assert!(value("-1") < value("0"));
    assert!(value("-2") < value("-1"));
}

/// rippled's amount bits, checked against ripple-binary-codec's encoding.
#[test]
fn values_encode_as_rippled_amounts() {
    let bytes = |text: &str| hex::encode_upper(IouValue::parse(text).unwrap().to_bytes());
    assert_eq!(bytes("0"), "8000000000000000");
    assert_eq!(bytes("1"), "D4838D7EA4C68000");
    assert_eq!(bytes("-1"), "94838D7EA4C68000");
}

/// A transfer rate rounds the sender's cost up to what the ledger holds.
#[test]
fn transfer_rates_round_the_cost_up() {
    let value = |text: &str| IouValue::parse(text).unwrap();
    assert_eq!(
        value("100").times_rate_up(1_002_000_000).unwrap(),
        value("100.2")
    );
    assert_eq!(value("1").times_rate_up(1_000_000_000).unwrap(), value("1"));
    // A sixteen-digit amount at 0.1% needs a seventeenth digit: rounded up.
    let cost = value("1234567890123456")
        .times_rate_up(1_001_000_000)
        .unwrap();
    assert_eq!(cost.to_decimal(), "1235802458013580");
    // 9999999999999999 × 1.000000001 = 10000000009999998.999999999, up.
    assert_eq!(
        value("9999999999999999")
            .times_rate_up(1_000_000_001)
            .unwrap()
            .to_decimal(),
        "10000000010000000"
    );
}

#[test]
fn currency_codes_have_one_spelling() {
    let usd = XrplCurrency::parse("USD").unwrap();
    assert_eq!(usd.canonical(), "USD");
    assert_eq!(
        XrplCurrency::parse("0000000000000000000000005553440000000000").unwrap(),
        usd
    );
    assert_ne!(
        XrplCurrency::parse("usd").unwrap(),
        usd,
        "codes are case-sensitive"
    );
    let solo = XrplCurrency::parse("534f4c4f00000000000000000000000000000000").unwrap();
    assert_eq!(solo.canonical(), "534F4C4F00000000000000000000000000000000");
    assert_eq!(solo.display(), "SOLO");
    // A legacy demurrage code is nonstandard, shown as its hex.
    let demurrage = XrplCurrency::parse("0158415500000000C1F76FF6ECB0BAC600000000").unwrap();
    assert_eq!(demurrage.display(), demurrage.canonical());
    let reserved = XrplCurrency::parse("0000000000000000000000005553440000000001");
    assert!(reserved.is_err(), "a zero-led nonstandard code is reserved");
    let lp = XrplCurrency::parse("03B2C3E0C8C7E0E3E3D6E1A4D8BA5B6F5F6E0A11").unwrap();
    assert_eq!(lp.display(), lp.canonical());
    for bad in [
        "XRP",
        "0000000000000000000000005852500000000000",
        "US",
        "USDC",
        "U D",
        &"0".repeat(40),
    ] {
        assert!(XrplCurrency::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn issues_are_code_and_issuer() {
    let issue = XrplIssue::parse(
        "0000000000000000000000005553440000000000.rhub8VRN55s94qWKDv6jmDy1pUykJzF3wq",
    )
    .unwrap();
    assert_eq!(issue.identifier(), "USD.rhub8VRN55s94qWKDv6jmDy1pUykJzF3wq");
    for bad in [
        "USD",
        "USD.",
        "USD.rNotAnAddress",
        ".rhub8VRN55s94qWKDv6jmDy1pUykJzF3wq",
    ] {
        assert!(XrplIssue::parse(bad).is_err(), "{bad}");
    }
}

/// Every value ripple-binary-codec encodes, byte for byte.
#[test]
fn amounts_match_ripple_binary_codec() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/issued-assets.json")).unwrap();
    for row in fixture["xrpl"]["amounts"].as_array().unwrap() {
        let value = IouValue::parse(row["value"].as_str().unwrap()).unwrap();
        let hex = row["hex"].as_str().unwrap();
        assert_eq!(hex::encode_upper(value.to_bytes()), hex[..16], "{row}");
        assert_eq!(
            &hex[16..56],
            hex::encode_upper(XrplCurrency::parse("USD").unwrap().0)
        );
    }
}

/// A balance change is exact however far apart the two values' exponents.
#[test]
fn differences_are_exact() {
    let value = |text: &str| IouValue::parse(text).unwrap();
    assert_eq!(value("10").minus(&value("2.5")), (false, "7.5".into()));
    assert_eq!(value("2.5").minus(&value("10")), (true, "7.5".into()));
    assert_eq!(value("0").minus(&value("-3")), (false, "3".into()));
    assert_eq!(
        value("1e20").minus(&value("1e-20")),
        (false, format!("99999999999999999999.{}", "9".repeat(20)))
    );
    assert_eq!(value("5").minus(&value("5")), (false, "0".into()));
}

/// Balances kept at fifteen places round down and refuse what cannot count.
#[test]
fn units_round_down_at_fixed_places() {
    let value = |text: &str| IouValue::parse(text).unwrap();
    assert_eq!(value("1.5").to_units(15), Some(1_500_000_000_000_000));
    assert_eq!(value("1e-20").to_units(15), Some(0));
    assert_eq!(value("1.234567890123456e-14").to_units(15), Some(12));
    assert_eq!(value("0").to_units(15), Some(0));
    assert_eq!(
        value("1e30").to_units(15),
        None,
        "past a u128 at fifteen places"
    );
    assert_eq!(value("-1").to_units(15), None);
}

/// A credit fits a line while the balance stays within its limit, exactly.
#[test]
fn credits_fit_within_limits() {
    let value = |text: &str| IouValue::parse(text).unwrap();
    assert!(IouValue::sum_fits(&value("5"), &value("95"), &value("100")));
    assert!(!IouValue::sum_fits(
        &value("5.0000000000001"),
        &value("95"),
        &value("100")
    ));
    assert!(IouValue::sum_fits(
        &value("1e-80"),
        &value("0"),
        &value("9999999999999999e80")
    ));
    assert!(!IouValue::sum_fits(&value("1"), &value("0"), &value("0")));
    assert!(IouValue::sum_fits(&value("1"), &value("-1"), &value("0")));
}
