//! Built-in token registry.
//!
//! The source of truth is `Core/tokens.toml`, embedded at compile time.
//! Call [`list_tokens_json`] to get a JSON array of token entries for a given
//! chain_id (or all chains when `chain_id == u32::MAX`).

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

// Embedded at compile time — no bundle dependency at runtime.
static TOKENS_TOML: &str = include_str!("../../tokens.toml");

// ----------------------------------------------------------------
// Parsed TOML shape
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TomlFile {
    tokens: Vec<TomlToken>,
}

#[derive(Debug, Deserialize)]
struct TomlToken {
    chain:           String,
    chain_id:        u32,
    name:            String,
    symbol:          String,
    standard:        String,
    contract:        String,
    market_id:       String,
    coingecko_id:    String,
    decimals:        u32,
    display_decimals: Option<u32>,
    category:        String,
    enabled:         bool,
}

// ----------------------------------------------------------------
// Public serialized shape (mirrors ChainTokenRegistryEntry in Swift)
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TokenEntry {
    pub chain:           String,
    pub chain_id:        u32,
    pub name:            String,
    pub symbol:          String,
    pub token_standard:  String,
    pub contract:        String,
    pub market_id:       String,
    pub coingecko_id:    String,
    pub decimals:        u32,
    pub display_decimals: Option<u32>,
    pub category:        String,
    pub enabled:         bool,
}

// ----------------------------------------------------------------
// Static catalog
// ----------------------------------------------------------------

static CATALOG: Lazy<Vec<TokenEntry>> = Lazy::new(|| {
    let parsed: TomlFile = toml::from_str(TOKENS_TOML)
        .expect("tokens.toml is embedded at compile time and must be valid TOML");
    parsed
        .tokens
        .into_iter()
        .map(|t| TokenEntry {
            chain:           t.chain,
            chain_id:        t.chain_id,
            name:            t.name,
            symbol:          t.symbol,
            token_standard:  t.standard,
            contract:        t.contract,
            market_id:       t.market_id,
            coingecko_id:    t.coingecko_id,
            decimals:        t.decimals,
            display_decimals: t.display_decimals,
            category:        t.category,
            enabled:         t.enabled,
        })
        .collect()
});

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Return all token entries as a JSON array.
/// Pass `chain_id = u32::MAX` to get every chain.
pub fn list_tokens_json(chain_id: u32) -> String {
    let entries: Vec<&TokenEntry> = if chain_id == u32::MAX {
        CATALOG.iter().collect()
    } else {
        CATALOG.iter().filter(|t| t.chain_id == chain_id).collect()
    };
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

/// Return a reference to the static catalog slice.
pub fn catalog() -> &'static [TokenEntry] {
    &CATALOG
}
