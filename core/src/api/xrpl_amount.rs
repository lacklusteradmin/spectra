//! XRP Ledger issued-currency amounts and currency codes, as rippled reads
//! and writes them: a value of at most 16 significant digits between 1e-81
//! and 9999999999999999e80, and a 160-bit currency code that is either a
//! three-character code or 40 hex digits.

use crate::api::error::ApiError;
use std::cmp::Ordering;

/// rippled's `STAmount` for an issued currency, normalized: zero, or a
/// mantissa in `10^15..10^16` and an exponent in `-96..=80`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct IouValue {
    pub negative: bool,
    pub mantissa: u64,
    pub exponent: i32,
}

const MIN_MANTISSA: u64 = 1_000_000_000_000_000;
const MAX_MANTISSA: u64 = 9_999_999_999_999_999;
const MIN_EXPONENT: i32 = -96;
const MAX_EXPONENT: i32 = 80;

impl IouValue {
    pub const ZERO: Self = Self {
        negative: false,
        mantissa: 0,
        exponent: 0,
    };

    pub fn is_zero(&self) -> bool {
        self.mantissa == 0
    }

    /// The value of `digits × 10^exponent`, exactly, or `None` when it needs
    /// more than 16 significant digits or falls outside the ledger's range.
    fn normalized(negative: bool, digits: &str, exponent: i64) -> Option<Self> {
        let trimmed = digits.trim_start_matches('0');
        if trimmed.is_empty() {
            return Some(Self::ZERO);
        }
        let significant = trimmed.trim_end_matches('0');
        if significant.len() > 16 {
            return None;
        }
        let exponent = exponent + (trimmed.len() - significant.len()) as i64;
        // Pad the significant digits to sixteen: the mantissa's own width.
        let pad = 16 - significant.len();
        let mantissa: u64 = format!("{significant}{}", "0".repeat(pad)).parse().ok()?;
        let exponent = i32::try_from(exponent - pad as i64).ok()?;
        (MIN_MANTISSA..=MAX_MANTISSA)
            .contains(&mantissa)
            .then_some(())?;
        (MIN_EXPONENT..=MAX_EXPONENT)
            .contains(&exponent)
            .then_some(Self {
                negative,
                mantissa,
                exponent,
            })
    }

    /// A value as rippled writes it: an optional `-`, digits with at most one
    /// `.`, and an optional exponent (`1234e-20`).
    pub fn parse(text: &str) -> Option<Self> {
        let (negative, unsigned) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (number, exponent) = match unsigned.split_once(['e', 'E']) {
            Some((number, exponent)) => (number, exponent.parse::<i64>().ok()?),
            None => (unsigned, 0),
        };
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        if whole.is_empty() && fraction.is_empty()
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let value = Self::normalized(
            negative,
            &format!("{whole}{fraction}"),
            exponent.checked_sub(fraction.len() as i64)?,
        )?;
        Some(Self {
            negative: negative && !value.is_zero(),
            ..value
        })
    }

    /// A positive exact decimal a person typed, or `None` when the ledger
    /// cannot hold it exactly.
    pub fn from_decimal(text: &str) -> Option<Self> {
        let exact = crate::decimal::canonical(text)?;
        Self::parse(&exact).filter(|value| !value.negative)
    }

    /// The exact decimal, written out in full with no exponent.
    pub fn to_decimal(self) -> String {
        if self.is_zero() {
            return "0".into();
        }
        let digits = self.mantissa.to_string();
        let sign = if self.negative { "-" } else { "" };
        let text = if self.exponent >= 0 {
            format!("{digits}{}", "0".repeat(self.exponent as usize))
        } else {
            let places = (-self.exponent) as usize;
            if places >= digits.len() {
                format!("0.{}{digits}", "0".repeat(places - digits.len()))
            } else {
                let (whole, fraction) = digits.split_at(digits.len() - places);
                format!("{whole}.{fraction}")
            }
        };
        let canonical = crate::decimal::canonical(&text).expect("digits are a decimal");
        format!("{sign}{canonical}")
    }

    /// The eight bytes of an issued-currency amount: not-XRP, sign, the
    /// exponent biased by 97, then the mantissa.
    pub fn to_bytes(self) -> [u8; 8] {
        if self.is_zero() {
            return 0x8000_0000_0000_0000u64.to_be_bytes();
        }
        let sign = if self.negative { 0 } else { 1u64 << 62 };
        let exponent = (self.exponent + 97) as u64;
        ((1u64 << 63) | sign | (exponent << 54) | self.mantissa).to_be_bytes()
    }

    /// A positive value as a count of `places`-decimal units, rounded down:
    /// a balance kept at fewer places than the ledger's never reads as more
    /// than it is. `None` for a negative value, or one too large to count.
    pub fn to_units(self, places: u32) -> Option<u128> {
        if self.negative {
            return None;
        }
        let shift = i64::from(self.exponent) + i64::from(places);
        let mantissa = u128::from(self.mantissa);
        if shift >= 0 {
            mantissa.checked_mul(10u128.checked_pow(u32::try_from(shift).ok()?)?)
        } else {
            Some(
                10u128
                    .checked_pow(u32::try_from(-shift).ok()?)
                    .map_or(0, |divisor| mantissa / divisor),
            )
        }
    }

    /// Whether `amount + balance <= limit`, exactly: a credit fits a trust
    /// line when it does not take the balance past the line's limit.
    pub fn sum_fits(amount: &Self, balance: &Self, limit: &Self) -> bool {
        let base = amount.exponent.min(balance.exponent).min(limit.exponent);
        amount.scaled(base) + balance.scaled(base) <= limit.scaled(base)
    }

    /// The value as an integer count of `10^base` units, `base` at most its
    /// own exponent.
    fn scaled(&self, base: i32) -> num_bigint::BigInt {
        use num_bigint::BigInt;
        let magnitude =
            BigInt::from(self.mantissa) * BigInt::from(10u8).pow((self.exponent - base) as u32);
        if self.negative { -magnitude } else { magnitude }
    }

    /// `self - other`, exactly: whether it is negative, and its magnitude
    /// as a decimal. The change between two ledger values can need more
    /// digits than either holds.
    pub fn minus(&self, other: &Self) -> (bool, String) {
        let base = self.exponent.min(other.exponent);
        let difference = self.scaled(base) - other.scaled(base);
        let negative = difference.sign() == num_bigint::Sign::Minus;
        let digits = difference.magnitude().to_string();
        let decimal = if base >= 0 {
            crate::decimal::canonical(&format!("{digits}{}", "0".repeat(base as usize)))
        } else {
            crate::decimal::from_unit_digits(&digits, base.unsigned_abs())
        };
        (negative, decimal.expect("digits are a decimal"))
    }

    /// `self × rate / 10^9`, rounded up to what the ledger can hold: the
    /// most a sender spends when an issuer's transfer rate takes its share.
    pub fn times_rate_up(&self, rate: u32) -> Option<Self> {
        if self.is_zero() || rate == 1_000_000_000 {
            return Some(*self);
        }
        let product = u128::from(self.mantissa) * u128::from(rate);
        // `product` has at most 26 digits; keep sixteen, rounding up.
        let digits = product.to_string();
        let keep = digits.len().min(16);
        let (head, tail) = digits.split_at(keep);
        let mut mantissa: u128 = head.parse().ok()?;
        if tail.bytes().any(|b| b != b'0') {
            mantissa += 1;
        }
        let exponent = i64::from(self.exponent) + tail.len() as i64 - 9;
        Self::normalized(self.negative, &mantissa.to_string(), exponent)
    }
}

impl PartialOrd for IouValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IouValue {
    fn cmp(&self, other: &Self) -> Ordering {
        let magnitude = |value: &Self| (!value.is_zero(), value.exponent, value.mantissa);
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => magnitude(self).cmp(&magnitude(other)),
            (true, true) => magnitude(other).cmp(&magnitude(self)),
        }
    }
}

/// A 160-bit XRPL currency code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct XrplCurrency(pub [u8; 20]);

/// The characters a three-character currency code may use.
fn standard_code_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"?!@#$%^&*<>(){}[]|".contains(&byte)
}

impl XrplCurrency {
    /// A three-character code, or 40 hex digits. `XRP` is the native coin,
    /// never an issued currency, and a code whose first byte is zero but is
    /// not a standard code is reserved.
    pub fn parse(text: &str) -> Result<Self, ApiError> {
        let invalid = || ApiError::invalid("Invalid XRP Ledger currency code");
        let mut bytes = [0u8; 20];
        if text.len() == 3 {
            if text == "XRP" || !text.bytes().all(standard_code_byte) {
                return Err(invalid());
            }
            bytes[12..15].copy_from_slice(text.as_bytes());
        } else if text.len() == 40 {
            bytes = hex::decode(text)
                .map_err(|_| invalid())?
                .try_into()
                .map_err(|_| invalid())?;
        } else {
            return Err(invalid());
        }
        let currency = Self(bytes);
        let standard = currency.standard_code();
        if bytes == [0; 20] || bytes[0] == 0 && standard.is_none() || standard == Some("XRP") {
            return Err(invalid());
        }
        Ok(currency)
    }

    /// The three-character code, when the bytes are in the standard form.
    fn standard_code(&self) -> Option<&str> {
        let bytes = &self.0;
        (bytes[..12].iter().all(|b| *b == 0)
            && bytes[15..].iter().all(|b| *b == 0)
            && bytes[12..15].iter().all(|b| standard_code_byte(*b)))
        .then(|| std::str::from_utf8(&bytes[12..15]).expect("ASCII"))
    }

    /// The one spelling identity uses: the three characters of a standard
    /// code, otherwise 40 uppercase hex digits.
    pub fn canonical(&self) -> String {
        match self.standard_code() {
            Some(code) => code.to_string(),
            None => hex::encode_upper(self.0),
        }
    }

    /// What a person reads: a standard code, or a nonstandard code's text
    /// when it is printable ASCII padded with zeros (`SOLO`); otherwise the
    /// hex.
    pub fn display(&self) -> String {
        if let Some(code) = self.standard_code() {
            return code.to_string();
        }
        let text = self.0.split(|b| *b == 0).next().unwrap_or_default();
        let padded = self.0[text.len()..].iter().all(|b| *b == 0);
        if !text.is_empty() && padded && text.iter().all(|b| b.is_ascii_graphic()) {
            String::from_utf8(text.to_vec()).expect("ASCII")
        } else {
            self.canonical()
        }
    }
}

/// An issued currency's identity: its code and its issuer, written
/// `CODE.rIssuer`, the code in its canonical spelling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct XrplIssue {
    pub currency: XrplCurrency,
    pub issuer: String,
}

impl XrplIssue {
    pub fn parse(identifier: &str) -> Result<Self, ApiError> {
        let (currency, issuer) = identifier
            .trim()
            .split_once('.')
            .ok_or_else(|| ApiError::invalid("An XRP Ledger token is written CODE.rIssuer"))?;
        crate::derivation::xrp::decode_xrp_address(issuer)
            .map_err(|_| ApiError::invalid("Invalid XRP Ledger token issuer"))?;
        Ok(Self {
            currency: XrplCurrency::parse(currency)?,
            issuer: issuer.to_string(),
        })
    }

    pub fn identifier(&self) -> String {
        format!("{}.{}", self.currency.canonical(), self.issuer)
    }
}

#[cfg(test)]
#[path = "tests/xrpl_amount.rs"]
mod tests;
