use serde::{Deserialize, Serialize};

/// How to render one amount of one asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AssetAmountDisplay {
    /// Decimal places to render at. Trailing zeros are trimmed by the caller's
    /// formatter, so this is a maximum, never a minimum.
    pub places: u32,
    /// The amount is smaller than `threshold` and cannot be shown truthfully at
    /// `places`; render `<threshold` instead of a rounded-to-zero number.
    pub below_threshold: bool,
    /// The smallest amount `places` can express — `10^-places`, or 0 when
    /// `places` is 0.
    pub threshold: f64,
}

/// Significant digits kept for an asset amount.
///
/// Six is enough to distinguish holdings a person would act on and short enough
/// to read. It is counted from the first non-zero digit, so a small balance
/// keeps its detail instead of rounding away.
const SIGNIFICANT_DIGITS: u32 = 6;

/// Ceiling on rendered decimal places, whatever the asset supports.
///
/// An 18-decimal token can hold amounts no interface should print in full; past
/// eight places the number is dust, and `below_threshold` says so in one glyph
/// rather than eighteen digits.
const MAX_DISPLAY_PLACES: u32 = 8;


pub fn token_preference_lookup_key(chain_name: &str, symbol: &str) -> String {
    let chain_trimmed = chain_name.trim();
    let symbol_trimmed = symbol.trim().to_uppercase();
    format!("{}|{}", chain_trimmed, symbol_trimmed)
}


/// How many decimal places a chain's native asset actually has.
///
/// Read from `chains.toml`, which carries `native_decimals` on all seventy-eight
/// rows. This used to be `SUPPORTED_DECIMAL_CHAINS`, a hand-written table of
/// **twenty-two** of them beside the catalog, with everything else falling to a
/// literal `6`. The twenty-two agreed with the catalog exactly — it was a
/// correct transcription, and short by fifty-six rows.
pub fn supported_decimal_places(chain_name: &str, override_decimals: Option<u32>) -> u32 {
    if let Some(value) = override_decimals {
        return value;
    }
    crate::chains::list_all_chains()
        .iter()
        .find(|entry| entry.name == chain_name)
        .map(|entry| entry.native_decimals)
        .unwrap_or(6)
}

/// How many decimal places to show for `amount` of an asset with
/// `asset_decimals` of its own.
///
/// Places are chosen per amount rather than per chain: six significant digits
/// counted from the first non-zero digit, capped by what the asset actually
/// supports and by [`MAX_DISPLAY_PLACES`]. A fixed per-chain count cannot do
/// both jobs at once — the count that shows 0.00042 BTC prints six zeros after
/// 1234.5678 ETH, and the count that reads well on the large amount reports the
/// small one as nothing at all.
pub fn asset_amount_display(amount: f64, asset_decimals: u32) -> AssetAmountDisplay {
    let cap = asset_decimals.min(MAX_DISPLAY_PLACES);
    if !amount.is_finite() || amount <= 0.0 {
        return AssetAmountDisplay {
            places: 0,
            below_threshold: false,
            threshold: 0.0,
        };
    }
    let places = if amount >= 1.0 {
        // Digits left of the point already spend the budget.
        let integer_digits = amount.log10().floor() as u32 + 1;
        SIGNIFICANT_DIGITS.saturating_sub(integer_digits).min(cap)
    } else {
        // Zeros between the point and the first significant digit are not
        // digits of the number; they are what a fixed count spends its budget
        // on. Skip them, then keep the same six.
        let leading_zeros = (-amount.log10().floor()) as u32 - 1;
        (leading_zeros + SIGNIFICANT_DIGITS).min(cap)
    };
    let threshold = if places == 0 {
        0.0
    } else {
        10f64.powi(-(places as i32))
    };
    AssetAmountDisplay {
        places,
        below_threshold: places > 0 && amount < threshold,
        threshold,
    }
}


pub fn normalized_history_source_tag(raw_source: Option<&str>, unknown_label: &str) -> String {
    let trimmed = raw_source
        .map(|value| value.trim().to_lowercase())
        .unwrap_or_default();
    if trimmed.is_empty() {
        return unknown_label.to_string();
    }
    match trimmed.as_str() {
        "esplora" => "Esplora".to_string(),
        "litecoinspace" => "LitecoinSpace".to_string(),
        "blockchair" => "Blockchair".to_string(),
        "blockcypher" => "BlockCypher".to_string(),
        "dogecoin.providers" => "DOGE Providers".to_string(),
        "rpc" => "RPC".to_string(),
        "etherscan" => "Etherscan".to_string(),
        "blockscout" => "Blockscout".to_string(),
        "ethplorer" => "Ethplorer".to_string(),
        "none" => unknown_label.to_string(),
        _ => capitalize_words(&trimmed),
    }
}

pub fn normalized_status_rank(status: &str) -> u32 {
    match status {
        "confirmed" => 3,
        "pending" => 2,
        "failed" => 1,
        _ => 0,
    }
}

fn capitalize_words(value: &str) -> String {
    value
        .split(|c: char| !c.is_alphanumeric())
        .map(capitalize_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[uniffi::export]
pub fn formatting_supported_decimal_places(
    chain_name: String,
    override_decimals: Option<u32>,
) -> u32 {
    supported_decimal_places(&chain_name, override_decimals)
}

#[uniffi::export]
pub fn formatting_asset_amount_display(amount: f64, asset_decimals: u32) -> AssetAmountDisplay {
    asset_amount_display(amount, asset_decimals)
}

#[uniffi::export]
pub fn formatting_token_preference_lookup_key(chain_name: String, symbol: String) -> String {
    token_preference_lookup_key(&chain_name, &symbol)
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FiatAmountRules {
    pub decimals: u32,
    pub minimum_visible: f64,
}

pub fn fiat_amount_rules(currency_code: &str) -> FiatAmountRules {
    if currency_code.eq_ignore_ascii_case("JPY") {
        FiatAmountRules {
            decimals: 0,
            minimum_visible: 1.0,
        }
    } else {
        FiatAmountRules {
            decimals: 2,
            minimum_visible: 0.01,
        }
    }
}

#[uniffi::export]
pub fn formatting_fiat_amount_rules(currency_code: String) -> FiatAmountRules {
    fiat_amount_rules(&currency_code)
}

pub fn asset_minimum_visible_amount(visible_decimals: u32) -> f64 {
    if visible_decimals == 0 {
        0.0
    } else {
        10f64.powi(-(visible_decimals as i32))
    }
}

pub fn dashboard_asset_grouping_key(
    chain_identity: &str,
    coin_gecko_id: &str,
    symbol: &str,
) -> String {
    let normalized_cg = coin_gecko_id.trim().to_lowercase();
    let chain_lc = chain_identity.to_lowercase();
    if !normalized_cg.is_empty() {
        format!("chain:{chain_lc}|cg:{normalized_cg}")
    } else {
        format!("chain:{chain_lc}|symbol:{}", symbol.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every chain's native decimals come from the catalog.
    ///
    /// A table here would be short by however many chains it forgot, and the
    /// forgotten ones would silently format at whatever the fallback is.
    #[test]
    fn native_decimals_come_from_the_catalog_for_every_chain() {
        for entry in crate::chains::list_all_chains() {
            assert_eq!(
                supported_decimal_places(&entry.name, None),
                entry.native_decimals,
                "{} formats at the wrong precision",
                entry.name
            );
        }
        // The override still wins, which is what the argument is for.
        assert_eq!(supported_decimal_places("Base", Some(2)), 2);
        // And a chain the catalog does not know keeps the old fallback.
        assert_eq!(supported_decimal_places("Not A Chain", None), 6);
    }

    #[test]
    fn lookup_key_normalizes_inputs() {
        assert_eq!(
            token_preference_lookup_key("  Bitcoin  ", "  btc  "),
            "Bitcoin|BTC"
        );
    }

    #[test]
    fn supported_defaults_match_chain_table() {
        assert_eq!(supported_decimal_places("Bitcoin", None), 8);
        assert_eq!(supported_decimal_places("Ethereum", None), 18);
        assert_eq!(supported_decimal_places("Unknown", None), 6);
        assert_eq!(supported_decimal_places("Ethereum", Some(6)), 6);
    }

    /// The rule reads the amount, not a per-chain setting. Each row is a case a
    /// fixed count gets wrong in one direction or the other.
    #[test]
    fn places_follow_the_amount_not_the_chain() {
        // (amount, asset decimals, expected places)
        let cases = [
            // A small balance keeps its significant digits instead of rounding
            // to nothing — at the old fixed three places this read "<0.001".
            (0.00042_f64, 8_u32, 8_u32),
            (0.000015, 18, 8),
            // A large one spends its budget left of the point.
            (1234.5678, 18, 2),
            (12.5, 6, 4),
            (0.5, 18, 6),
            // Six significant digits, counted from the first non-zero.
            (0.123456789, 18, 6),
            // Never more places than the asset has.
            (0.5, 2, 2),
            (0.5, 0, 0),
            // Millions need none.
            (1_234_567.0, 18, 0),
        ];
        for (amount, decimals, expected) in cases {
            assert_eq!(
                asset_amount_display(amount, decimals).places,
                expected,
                "{amount} on a {decimals}-decimal asset"
            );
        }
    }

    #[test]
    fn dust_below_the_ceiling_is_marked_rather_than_rounded_to_zero() {
        // 1 wei. Eighteen places would be truthful and unreadable.
        let d = asset_amount_display(1e-18, 18);
        assert_eq!(d.places, 8);
        assert!(d.below_threshold);
        assert!((d.threshold - 1e-8).abs() < 1e-16);

        // A balance the ceiling can express is never marked.
        assert!(!asset_amount_display(0.00042, 8).below_threshold);
        assert!(!asset_amount_display(1e-8, 18).below_threshold);
    }

    #[test]
    fn zero_and_nonsense_amounts_ask_for_nothing() {
        for amount in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let d = asset_amount_display(amount, 18);
            assert_eq!(d.places, 0);
            assert!(!d.below_threshold);
        }
    }

    #[test]
    fn fiat_amount_rules_jpy_vs_others() {
        let jpy = fiat_amount_rules("JPY");
        assert_eq!(jpy.decimals, 0);
        assert_eq!(jpy.minimum_visible, 1.0);
        let usd = fiat_amount_rules("USD");
        assert_eq!(usd.decimals, 2);
        assert!((usd.minimum_visible - 0.01).abs() < 1e-9);
    }

    #[test]
    fn asset_minimum_visible_zero_decimals() {
        assert_eq!(asset_minimum_visible_amount(0), 0.0);
        assert!((asset_minimum_visible_amount(2) - 0.01).abs() < 1e-12);
        assert!((asset_minimum_visible_amount(8) - 1e-8).abs() < 1e-16);
    }

    #[test]
    fn history_source_tag_handles_known_and_unknown() {
        assert_eq!(
            normalized_history_source_tag(Some("esplora"), "Unknown"),
            "Esplora"
        );
        assert_eq!(
            normalized_history_source_tag(Some(""), "Unknown"),
            "Unknown"
        );
        assert_eq!(normalized_history_source_tag(None, "Unknown"), "Unknown");
        assert_eq!(
            normalized_history_source_tag(Some("custom"), "Unknown"),
            "Custom"
        );
    }
}
