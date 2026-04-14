use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDecimalsResolution {
    pub supported: u32,
    pub display: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPreferenceOverride {
    pub chain_name: String,
    pub symbol: String,
    pub decimals: u32,
    pub display_decimals: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDecimalsRequest {
    pub chain_name: String,
    pub symbol: String,
    pub asset_display_decimals: u32,
    pub token_override: Option<TokenPreferenceOverride>,
}

const MAX_DECIMALS: u32 = 30;

const SUPPORTED_DECIMAL_CHAINS: &[(&str, u32)] = &[
    ("Bitcoin", 8),
    ("Bitcoin Cash", 8),
    ("Bitcoin SV", 8),
    ("Litecoin", 8),
    ("Dogecoin", 8),
    ("Aptos", 8),
    ("Ethereum", 18),
    ("Ethereum Classic", 18),
    ("Arbitrum", 18),
    ("Optimism", 18),
    ("BNB Chain", 18),
    ("Avalanche", 18),
    ("Hyperliquid", 18),
    ("Tron", 6),
    ("Cardano", 6),
    ("XRP Ledger", 6),
    ("Solana", 9),
    ("Sui", 9),
    ("TON", 9),
    ("Monero", 12),
    ("NEAR", 24),
    ("Polkadot", 10),
];

pub fn token_preference_lookup_key(chain_name: &str, symbol: &str) -> String {
    let chain_trimmed = chain_name.trim();
    let symbol_trimmed = symbol.trim().to_uppercase();
    format!("{}|{}", chain_trimmed, symbol_trimmed)
}

pub fn native_asset_display_settings_key(chain_name: &str) -> String {
    matches!(chain_name, "Ethereum" | "Arbitrum" | "Optimism")
        .then(|| "Ethereum".to_string())
        .unwrap_or_else(|| chain_name.to_string())
}

pub fn supported_decimal_places(chain_name: &str, override_decimals: Option<u32>) -> u32 {
    if let Some(value) = override_decimals {
        return value;
    }
    SUPPORTED_DECIMAL_CHAINS
        .iter()
        .find(|(name, _)| *name == chain_name)
        .map(|(_, decimals)| *decimals)
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
    SUPPORTED_DECIMAL_CHAINS
        .iter()
        .map(|(name, _)| ((*name).to_string(), normalized))
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
pub fn formatting_resolve_asset_decimals_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request: AssetDecimalsRequest = serde_json::from_str(&request_json)?;
    let result = resolve_asset_decimals(&request);
    Ok(serde_json::to_string(&result)?)
}

#[uniffi::export]
pub fn formatting_token_preference_lookup_key(chain_name: String, symbol: String) -> String {
    token_preference_lookup_key(&chain_name, &symbol)
}

#[uniffi::export]
pub fn formatting_native_asset_display_settings_key(chain_name: String) -> String {
    native_asset_display_settings_key(&chain_name)
}

#[uniffi::export]
pub fn formatting_default_asset_display_decimals_by_chain_json(
    default_value: u32,
) -> Result<String, crate::SpectraBridgeError> {
    let map = default_asset_display_decimals_by_chain(default_value);
    Ok(serde_json::to_string(&map)?)
}

#[uniffi::export]
pub fn formatting_normalized_history_source_tag(
    raw_source: Option<String>,
    unknown_label: String,
) -> String {
    normalized_history_source_tag(raw_source.as_deref(), &unknown_label)
}

#[uniffi::export]
pub fn formatting_normalized_status_rank(status: String) -> u32 {
    normalized_status_rank(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

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
