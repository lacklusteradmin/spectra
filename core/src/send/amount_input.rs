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

#[uniffi::export]
pub fn parse_amount_input(text: String, max_decimals: u32) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut chars = trimmed.chars().peekable();
    let mut saw_digit_int = false;
    let mut saw_dot = false;
    let mut frac_digits: u32 = 0;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            if saw_dot {
                frac_digits += 1;
                if frac_digits > max_decimals {
                    return None;
                }
            } else {
                saw_digit_int = true;
            }
            chars.next();
        } else if c == '.' {
            if saw_dot {
                return None;
            }
            saw_dot = true;
            chars.next();
        } else {
            return None;
        }
    }
    if saw_dot && frac_digits == 0 && !saw_digit_int {
        return None; // lone "."
    }
    if !saw_digit_int && !saw_dot {
        return None;
    }
    let value: f64 = trimmed.parse().ok()?;
    if value > 0.0 {
        Some(value)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_whitespace() {
        assert!(parse_amount_input("".into(), 8).is_none());
        assert!(parse_amount_input("   ".into(), 8).is_none());
    }

    #[test]
    fn rejects_non_numeric() {
        assert!(parse_amount_input("abc".into(), 8).is_none());
        assert!(parse_amount_input("1,5".into(), 8).is_none());
        assert!(parse_amount_input("1e3".into(), 8).is_none());
    }

    #[test]
    fn rejects_zero_and_negative() {
        assert!(parse_amount_input("0".into(), 8).is_none());
        assert!(parse_amount_input("0.0".into(), 8).is_none());
        assert!(parse_amount_input("-1".into(), 8).is_none());
    }

    #[test]
    fn rejects_over_precision() {
        assert!(parse_amount_input("1.123456789".into(), 8).is_none());
        assert!(parse_amount_input("0.12345678901234567890".into(), 18).is_none());
    }

    #[test]
    fn accepts_integer_and_decimal() {
        assert_eq!(parse_amount_input("1".into(), 8), Some(1.0));
        assert_eq!(parse_amount_input("  0.5  ".into(), 8), Some(0.5));
        assert_eq!(parse_amount_input(".25".into(), 8), Some(0.25));
        assert_eq!(parse_amount_input("1.12345678".into(), 8), Some(1.12345678));
    }

    #[test]
    fn rejects_double_dot_and_lone_dot() {
        assert!(parse_amount_input(".".into(), 8).is_none());
        assert!(parse_amount_input("1..2".into(), 8).is_none());
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
