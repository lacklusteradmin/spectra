//! Built-in chain registry.
//!
//! The source of truth is `core/data/chains.toml`, embedded at compile time.
//! Call [`list_all_chains`] to get all chain entries (mainnet + testnet).

use std::sync::LazyLock;
use serde::{Deserialize, Serialize};

static CHAINS_TOML: &str = include_str!("../data/chains.toml");

// ----------------------------------------------------------------
// Parsed TOML shape
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TomlFile {
    chains: Vec<TomlChain>,
}

#[derive(Debug, Deserialize)]
struct TomlChain {
    id:                      String,
    name:                    String,
    symbol:                  String,
    gas_token_symbol:        String,
    search_keywords:         Vec<String>,
    category:                String,
    is_evm:                  bool,
    supports_endpoint_catalog: bool,
    supports_diagnostics:    bool,
    color_name:              String,
    asset_name:              String,
    token_standard:          String,
    contract_address_prompt: String,
    native_coingecko_id:     String,
    native_decimals:         u32,
    native_asset_name:       String,
    tags:                    Vec<String>,
    family:                  String,
    consensus:               String,
    state_model:             String,
    primary_use:             String,
    slip44_coin_type:        String,
    derivation_path:         String,
    alt_derivation_path:     String,
    total_circulation_model: String,
    notable_details:         Vec<String>,
}

// ----------------------------------------------------------------
// Public serialized shape — exposed to Swift via UniFFI
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainEntry {
    pub id:                      String,
    pub name:                    String,
    pub symbol:                  String,
    pub gas_token_symbol:        String,
    pub search_keywords:         Vec<String>,
    pub category:                String,
    pub is_evm:                  bool,
    pub supports_endpoint_catalog: bool,
    pub supports_diagnostics:    bool,
    pub color_name:              String,
    pub asset_name:              String,
    pub token_standard:          String,
    pub contract_address_prompt: String,
    pub native_coingecko_id:     String,
    pub native_decimals:         u32,
    pub native_asset_name:       String,
    pub tags:                    Vec<String>,
    pub family:                  String,
    pub consensus:               String,
    pub state_model:             String,
    pub primary_use:             String,
    pub slip44_coin_type:        String,
    pub derivation_path:         String,
    pub alt_derivation_path:     String,
    pub total_circulation_model: String,
    pub notable_details:         Vec<String>,
}

// ----------------------------------------------------------------
// Static catalog
// ----------------------------------------------------------------

static CATALOG: LazyLock<Vec<ChainEntry>> = LazyLock::new(|| {
    let parsed: TomlFile = toml::from_str(CHAINS_TOML)
        .expect("chains.toml is embedded at compile time and must be valid TOML");
    parsed
        .chains
        .into_iter()
        .map(|c| ChainEntry {
            id:                      c.id,
            name:                    c.name,
            symbol:                  c.symbol,
            gas_token_symbol:        c.gas_token_symbol,
            search_keywords:         c.search_keywords,
            category:                c.category,
            is_evm:                  c.is_evm,
            supports_endpoint_catalog: c.supports_endpoint_catalog,
            supports_diagnostics:    c.supports_diagnostics,
            color_name:              c.color_name,
            asset_name:              c.asset_name,
            token_standard:          c.token_standard,
            contract_address_prompt: c.contract_address_prompt,
            native_coingecko_id:     c.native_coingecko_id,
            native_decimals:         c.native_decimals,
            native_asset_name:       c.native_asset_name,
            tags:                    c.tags,
            family:                  c.family,
            consensus:               c.consensus,
            state_model:             c.state_model,
            primary_use:             c.primary_use,
            slip44_coin_type:        c.slip44_coin_type,
            derivation_path:         c.derivation_path,
            alt_derivation_path:     c.alt_derivation_path,
            total_circulation_model: c.total_circulation_model,
            notable_details:         c.notable_details,
        })
        .collect()
});

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Return all chain entries (mainnet + testnet).
#[uniffi::export]
pub fn list_all_chains() -> Vec<ChainEntry> {
    CATALOG.clone()
}

/// Return a reference to the static catalog slice.
pub(crate) fn catalog() -> &'static [ChainEntry] {
    &CATALOG
}

/// Return the entry for a specific string id, or `None` if not found.
pub fn chain_by_str_id(id: &str) -> Option<&'static ChainEntry> {
    CATALOG.iter().find(|c| c.id == id)
}
