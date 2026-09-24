use crate::SpectraBridgeError;

/// Parse decimal input exactly. Never round, truncate, saturate, or allocate
/// according to untrusted decimal counts. Zero is allowed here; each send
/// protocol decides whether it permits a zero-value transaction.
pub fn parse_raw_amount(text: &str, decimals: u32) -> Result<u128, SpectraBridgeError> {
    let invalid = || SpectraBridgeError::InvalidInput {
        message: "amount must be an unsigned decimal within the asset precision and integer range"
            .into(),
    };
    if decimals > 38 {
        return Err(invalid());
    }
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|c| c.is_ascii_digit())
        || fraction.len() > decimals as usize
    {
        return Err(invalid());
    }
    let mut raw = 0u128;
    for digit in whole.bytes().chain(fraction.bytes()) {
        raw = raw
            .checked_mul(10)
            .and_then(|v| v.checked_add(u128::from(digit - b'0')))
            .ok_or_else(invalid)?;
    }
    for _ in fraction.len()..decimals as usize {
        raw = raw.checked_mul(10).ok_or_else(invalid)?;
    }
    Ok(raw)
}

// Amount-input validation: parses any positive decimal amount, bounded by a
// maximum fractional digit count.

/// Whether `text` is a positive amount within `max_decimals` places. The
/// answer only enables a button; the send paths parse exactly again.
#[uniffi::export]
pub fn is_valid_amount_input(text: String, max_decimals: u32) -> bool {
    parse_raw_amount(&text, max_decimals).is_ok_and(|raw| raw > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_whitespace() {
        assert!(!is_valid_amount_input("".into(), 8));
        assert!(!is_valid_amount_input("   ".into(), 8));
    }

    #[test]
    fn rejects_non_numeric() {
        assert!(!is_valid_amount_input("abc".into(), 8));
        assert!(!is_valid_amount_input("1,5".into(), 8));
        assert!(!is_valid_amount_input("1e3".into(), 8));
    }

    #[test]
    fn rejects_zero_and_negative() {
        assert!(!is_valid_amount_input("0".into(), 8));
        assert!(!is_valid_amount_input("0.0".into(), 8));
        assert!(!is_valid_amount_input("-1".into(), 8));
    }

    #[test]
    fn rejects_over_precision() {
        assert!(!is_valid_amount_input("1.123456789".into(), 8));
        assert!(!is_valid_amount_input("0.12345678901234567890".into(), 18));
    }

    #[test]
    fn accepts_integer_and_decimal() {
        assert!(is_valid_amount_input("1".into(), 8));
        assert!(is_valid_amount_input("  0.5  ".into(), 8));
        assert!(is_valid_amount_input(".25".into(), 8));
        assert!(is_valid_amount_input("1.12345678".into(), 8));
    }

    #[test]
    fn rejects_double_dot_and_lone_dot() {
        assert!(!is_valid_amount_input(".".into(), 8));
        assert!(!is_valid_amount_input("1..2".into(), 8));
    }
}

#[cfg(test)]
mod exact_tests {
    use super::parse_raw_amount;
    #[test]
    fn preserves_units_beyond_float_precision() {
        assert_eq!(
            parse_raw_amount("9007199.254740993", 9).unwrap(),
            9_007_199_254_740_993
        );
        assert_eq!(
            parse_raw_amount("1.234567890123456789", 18).unwrap(),
            1_234_567_890_123_456_789
        );
        assert_eq!(parse_raw_amount(".25", 6).unwrap(), 250_000);
        assert_eq!(
            parse_raw_amount(&u128::MAX.to_string(), 0).unwrap(),
            u128::MAX
        );
    }
    #[test]
    fn refuses_invalid_precision_unicode_and_overflow() {
        for (input, decimals) in [
            ("", 8),
            ("-1", 8),
            ("NaN", 8),
            ("inf", 8),
            ("1e3", 8),
            ("1.001", 2),
            ("1.000", 2),
            ("1.é", 1),
            ("1..0", 8),
            ("1", u32::MAX),
            ("340282366920938463463374607431768211456", 0),
        ] {
            assert!(
                parse_raw_amount(input, decimals).is_err(),
                "{input}/{decimals}"
            );
        }
    }
}

/// A percentage of an exact balance as editable decimal input, floored at the
/// asset's precision and again at the percentage: a shortcut never rounds up
/// past what it was taken from.
#[uniffi::export]
pub fn send_amount_shortcut(maximum: String, decimals: u32, percentage: u32) -> Option<String> {
    let truncated = crate::decimal::truncate(&maximum, decimals)?;
    shortcut_of_units(
        parse_raw_amount(&truncated, decimals).ok()?,
        decimals,
        percentage,
    )
}

/// The same over a fee-adjusted estimate a preview decoded as `f64`. Takes
/// the preceding representable value: the float may have rounded the
/// provider's integer balance upwards, and reserving one ULP keeps that
/// uncertainty on the safe side instead of manufacturing spendable units.
pub(crate) fn estimate_shortcut(maximum: f64, decimals: u32, percentage: u32) -> Option<String> {
    if !maximum.is_finite() || maximum <= 0.0 || decimals > 38 {
        return None;
    }
    let bits = maximum.to_bits().checked_sub(1)?;
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    // Split powers of ten so the intermediate fits for 24-decimal chains.
    let scaled = u128::from(mantissa).checked_mul(5u128.checked_pow(decimals)?)?;
    let shift = exponent + decimals as i32;
    let units = if shift >= 0 {
        scaled.checked_mul(1u128.checked_shl(shift as u32)?)?
    } else {
        scaled.checked_shr((-shift) as u32).unwrap_or(0)
    };
    shortcut_of_units(units, decimals, percentage)
}

fn shortcut_of_units(units: u128, decimals: u32, percentage: u32) -> Option<String> {
    if decimals > 38 || !(1..=100).contains(&percentage) {
        return None;
    }
    let percent = u128::from(percentage);
    let units = (units / 100).checked_mul(percent)? + (units % 100) * percent / 100;
    (units > 0).then(|| crate::decimal::from_units(units, decimals))
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    #[test]
    fn shortcuts_never_exceed_the_fee_adjusted_quote() {
        assert_eq!(
            estimate_shortcut(0.99999, 8, 100).as_deref(),
            Some("0.99998999")
        );
        assert_eq!(estimate_shortcut(1.0, 8, 50).as_deref(), Some("0.49999999"));
        let near = estimate_shortcut(1.0, 24, 100).unwrap();
        assert!(parse_raw_amount(&near, 24).unwrap() < 10u128.pow(24));
        assert!(!near.contains('e'));
        for maximum in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1e-30] {
            assert!(estimate_shortcut(maximum, 8, 100).is_none());
        }
        assert!(estimate_shortcut(1.0, 8, 0).is_none());
        assert!(estimate_shortcut(1.0, 8, 101).is_none());
        assert!(estimate_shortcut(1.0, 39, 100).is_none());
    }
    #[test]
    fn exact_shortcuts_floor_without_a_float() {
        assert_eq!(
            send_amount_shortcut("1".into(), 8, 100).as_deref(),
            Some("1")
        );
        assert_eq!(
            send_amount_shortcut("1".into(), 8, 50).as_deref(),
            Some("0.5")
        );
        assert_eq!(
            send_amount_shortcut("0.123456789".into(), 8, 10).as_deref(),
            Some("0.01234567")
        );
        assert_eq!(
            send_amount_shortcut("9007199254740993".into(), 0, 100).as_deref(),
            Some("9007199254740993")
        );
        for maximum in ["", "-1", "0", "0.000000001", "NaN"] {
            assert!(
                send_amount_shortcut(maximum.into(), 8, 100).is_none(),
                "{maximum}"
            );
        }
    }
}
