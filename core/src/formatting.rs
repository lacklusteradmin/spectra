use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AssetDecimalsResolution {
    pub supported: u32,
    pub display: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TokenPreferenceOverride {
    pub chain_name: String,
    pub symbol: String,
    pub decimals: u32,
    pub display_decimals: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AssetDecimalsRequest {
    pub chain_name: String,
    pub symbol: String,
    pub asset_display_decimals: u32,
    pub token_override: Option<TokenPreferenceOverride>,
}

const MAX_DECIMALS: u32 = 30;


pub fn token_preference_lookup_key(chain_name: &str, symbol: &str) -> String {
    let chain_trimmed = chain_name.trim();
    let symbol_trimmed = symbol.trim().to_uppercase();
    format!("{}|{}", chain_trimmed, symbol_trimmed)
}

/// Which chain's display-decimals setting a chain reads.
///
/// The EVM family shares Ethereum's, because they share a native asset the user
/// sets decimals for once. This named **three** of the twenty-three EVM
/// mainnets — Ethereum, Arbitrum, Optimism — so setting ETH's decimals moved
/// Arbitrum and Optimism and left Base, Polygon, BNB Chain and the other
/// nineteen on their own key. `Chain::is_evm` is the membership.
pub fn native_asset_display_settings_key(chain_name: &str) -> String {
    match crate::registry::Chain::from_display_name(chain_name) {
        Some(chain) if chain.is_evm() => crate::registry::Chain::Ethereum
            .chain_display_name()
            .to_string(),
        _ => chain_name.to_string(),
    }
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

pub fn display_decimal_places(
    _chain_name: &str,
    asset_display_decimals: u32,
    override_decimals: Option<u32>,
    override_display_decimals: Option<u32>,
) -> u32 {
    let normalized_chain_default = asset_display_decimals.min(MAX_DECIMALS);
    if let Some(decimals) = override_decimals {
        let default_display = normalized_chain_default.min(decimals);
        let chosen = override_display_decimals.unwrap_or(default_display);
        return chosen.min(decimals);
    }
    normalized_chain_default
}

pub fn resolve_asset_decimals(request: &AssetDecimalsRequest) -> AssetDecimalsResolution {
    let override_decimals = request.token_override.as_ref().map(|o| o.decimals);
    let override_display_decimals = request
        .token_override
        .as_ref()
        .and_then(|o| o.display_decimals);
    let supported = supported_decimal_places(&request.chain_name, override_decimals);
    let display = display_decimal_places(
        &request.chain_name,
        request.asset_display_decimals,
        override_decimals,
        override_display_decimals,
    )
    .min(supported);
    AssetDecimalsResolution { supported, display }
}

pub fn default_asset_display_decimals_by_chain(default_value: u32) -> HashMap<String, u32> {
    let normalized = default_value.min(MAX_DECIMALS);
    crate::chains::list_all_chains()
        .iter()
        .map(|entry| (entry.name.clone(), normalized))
        .collect()
}

pub fn normalize_asset_display_decimals(value: i64) -> u32 {
    value.clamp(0, MAX_DECIMALS as i64) as u32
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
pub fn formatting_resolve_asset_decimals(request: AssetDecimalsRequest) -> AssetDecimalsResolution {
    resolve_asset_decimals(&request)
}

#[uniffi::export]
pub fn formatting_default_asset_display_decimals_by_chain(
    default_value: u32,
) -> HashMap<String, u32> {
    default_asset_display_decimals_by_chain(default_value)
}

#[uniffi::export]
pub fn formatting_token_preference_lookup_key(chain_name: String, symbol: String) -> String {
    token_preference_lookup_key(&chain_name, &symbol)
}

#[uniffi::export]
pub fn formatting_native_asset_display_settings_key(chain_name: String) -> String {
    native_asset_display_settings_key(&chain_name)
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

const STABLECOIN_USD_SYMBOLS: &[&str] = &["USDC", "USDT", "FDUSD", "TUSD"];

pub fn is_usd_stablecoin(symbol: &str) -> bool {
    STABLECOIN_USD_SYMBOLS
        .iter()
        .any(|s| s.eq_ignore_ascii_case(symbol))
}

pub fn stablecoin_fallback_price_usd(symbol: &str) -> f64 {
    if is_usd_stablecoin(symbol) {
        1.0
    } else {
        0.0
    }
}

#[uniffi::export]
pub fn formatting_stablecoin_fallback_price_usd(symbol: String) -> f64 {
    stablecoin_fallback_price_usd(&symbol)
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
    /// A hand-written table of twenty-two chains stood here and everything else
    /// fell to a literal `6`, so an amount on Base — eighteen decimals — was
    /// formatted to six places, as were Zcash, Dash, Decred, Kaspa, Bitcoin
    /// Gold and Internet Computer at eight, Bittensor at nine, and every EVM L2
    /// outside the original thirteen. Reading the catalog cannot be short.
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

    #[test]
    fn native_settings_key_collapses_evm_l2() {
        assert_eq!(native_asset_display_settings_key("Arbitrum"), "Ethereum");
        assert_eq!(native_asset_display_settings_key("Optimism"), "Ethereum");
        assert_eq!(native_asset_display_settings_key("Bitcoin"), "Bitcoin");
    }

    #[test]
    fn display_decimals_clamps_to_supported() {
        let resolution = resolve_asset_decimals(&AssetDecimalsRequest {
            chain_name: "Ethereum".to_string(),
            symbol: "ETH".to_string(),
            asset_display_decimals: 30,
            token_override: None,
        });
        assert_eq!(resolution.supported, 18);
        assert_eq!(resolution.display, 18);
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
