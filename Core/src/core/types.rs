use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreCapabilities {
    pub schema_version: u32,
    pub supports_derivation: bool,
    pub supports_fetch_contracts: bool,
    pub supports_send_contracts: bool,
    pub supports_store_contracts: bool,
    pub supports_localization_catalogs: bool,
    pub supports_state_reducer: bool,
    pub supported_locales: Vec<String>,
    pub localization_tables: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainSummary {
    pub chain_name: String,
    pub curve: String,
    pub default_network: Option<String>,
    pub default_derivation_path: Option<String>,
    pub endpoint_count: u64,
    pub settings_visible_endpoint_count: u64,
    pub explorer_endpoint_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct LocalizationSummary {
    pub supported_locales: Vec<String>,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreBootstrap {
    pub capabilities: CoreCapabilities,
    pub chains: Vec<ChainSummary>,
    pub localization: LocalizationSummary,
    pub live_chain_names: Vec<String>,
}
