//! Exact unsigned decimal amounts, as they cross the FFI.
//!
//! An amount is a string of ASCII digits with at most one `.`: no sign, no
//! exponent, no grouping. The canonical spelling has no leading zeros in the
//! whole part (but `0` for zero), no trailing zeros in the fraction and no
//! trailing `.`. Arithmetic is exact at up to 38 significant digits, which is
//! more than any balance a wallet holds; past that it refuses rather than
//! rounds.

use std::cmp::Ordering;

/// Split a valid decimal into its whole and fractional digits.
fn parts(text: &str) -> Option<(&str, &str)> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    ((!whole.is_empty() || !fraction.is_empty()) && digits(whole) && digits(fraction))
        .then_some((whole, fraction))
}

/// The canonical spelling of `text`, or `None` when it is not an unsigned
/// decimal.
pub fn canonical(text: &str) -> Option<String> {
    let (whole, fraction) = parts(text)?;
    let whole = whole.trim_start_matches('0');
    let fraction = fraction.trim_end_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    Some(if fraction.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{fraction}")
    })
}

pub fn is_zero(text: &str) -> bool {
    canonical(text).is_some_and(|c| c == "0")
}

/// `units` smallest units of an asset with `decimals` places, exactly.
pub fn from_units(units: u128, decimals: u32) -> String {
    let digits = units.to_string();
    let decimals = decimals as usize;
    let text = if digits.len() > decimals {
        let (whole, fraction) = digits.split_at(digits.len() - decimals);
        format!("{whole}.{fraction}")
    } else {
        format!("0.{}{digits}", "0".repeat(decimals - digits.len()))
    };
    canonical(&text).unwrap_or_else(|| "0".to_string())
}

/// A value that arrived as a float, in its shortest round-trip spelling.
///
/// For sources that only ever had a float: this is exactly what they said,
/// not more precise than that. Negative and non-finite values are refused.
pub fn from_f64(value: f64) -> Option<String> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    // `{}` never uses an exponent for f64, and prints the shortest string
    // that reads back as the same value.
    canonical(&format!("{value}"))
}

/// A magnitude that arrived as a float, as a stored amount. The sign is the
/// transfer's direction, which records carry separately.
pub fn amount_from_f64(value: f64) -> String {
    from_f64(value.abs()).unwrap_or_else(|| "0".to_string())
}

/// `text` cut to at most `places` fractional digits, never rounded up: a
/// figure shown as spendable must not exceed what is.
pub fn truncate(text: &str, places: u32) -> Option<String> {
    let exact = canonical(text)?;
    let Some((whole, fraction)) = exact.split_once('.') else {
        return Some(exact);
    };
    let kept = &fraction[..fraction.len().min(places as usize)];
    canonical(&format!("{whole}.{kept}"))
}

/// An approximation for arithmetic whose result is itself approximate —
/// multiplying by a price, say.
pub fn to_f64(text: &str) -> f64 {
    canonical(text)
        .and_then(|c| c.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// The two amounts as integers at one scale.
fn aligned(a: &str, b: &str) -> Option<(u128, u128, usize)> {
    let (aw, af) = parts(a)?;
    let (bw, bf) = parts(b)?;
    let scale = af.len().max(bf.len());
    let at = |w: &str, f: &str| -> Option<u128> {
        let digits = format!("{w}{f}{}", "0".repeat(scale - f.len()));
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            Some(0)
        } else {
            digits.parse().ok()
        }
    };
    Some((at(aw, af)?, at(bw, bf)?, scale))
}

/// `a + b`, exactly. `None` when either is not a decimal or the sum overflows.
pub fn add(a: &str, b: &str) -> Option<String> {
    let (a, b, scale) = aligned(a, b)?;
    Some(from_units(a.checked_add(b)?, scale as u32))
}

pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (a, b, _) = aligned(a, b)?;
    Some(a.cmp(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_spelling_is_unique() {
        assert_eq!(canonical("0012.3400").as_deref(), Some("12.34"));
        assert_eq!(canonical(".5").as_deref(), Some("0.5"));
        assert_eq!(canonical("7.").as_deref(), Some("7"));
        assert_eq!(canonical("0.000").as_deref(), Some("0"));
        for bad in ["", ".", "-1", "1e3", "1,5", "1.2.3", "NaN"] {
            assert_eq!(canonical(bad), None, "{bad}");
        }
    }

    #[test]
    fn units_keep_every_digit_a_float_would_lose() {
        assert_eq!(
            from_units(1_234_567_890_123_456_789, 18),
            "1.234567890123456789"
        );
        assert_eq!(from_units(5, 8), "0.00000005");
        assert_eq!(from_units(0, 18), "0");
        assert_eq!(from_units(100, 2), "1");
    }

    #[test]
    fn sums_are_exact() {
        assert_eq!(
            add("0.1", "0.2").as_deref(),
            Some("0.3"),
            "the sum a float gets wrong"
        );
        assert_eq!(
            add("1.000000000000000001", "2").as_deref(),
            Some("3.000000000000000001")
        );
        assert_eq!(compare("1.50", "1.5"), Some(Ordering::Equal));
        assert_eq!(compare("0.00000001", "0"), Some(Ordering::Greater));
    }

    #[test]
    fn truncation_never_rounds_up() {
        assert_eq!(truncate("0.30000000000000004", 8).as_deref(), Some("0.3"));
        assert_eq!(truncate("1.999", 2).as_deref(), Some("1.99"));
        assert_eq!(truncate("5", 2).as_deref(), Some("5"));
    }

    #[test]
    fn floats_arrive_in_their_shortest_spelling() {
        assert_eq!(from_f64(0.1).as_deref(), Some("0.1"));
        assert_eq!(from_f64(1e-7).as_deref(), Some("0.0000001"));
        assert_eq!(from_f64(-1.0), None);
        assert_eq!(from_f64(f64::NAN), None);
    }
}
