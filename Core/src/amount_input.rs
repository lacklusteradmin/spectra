// Pure amount-input validation: Swift previously owned one regex-validated
// parser per chain. This module generalizes to any positive decimal amount with
// a max fractional digit count.

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
