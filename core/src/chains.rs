//! Built-in chain registry.
//!
//! The source of truth is `core/data/chains.toml`, embedded at compile time.
//! Call [`list_all_chains`] to get all chain entries (mainnet + testnet).

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static CHAINS_TOML: &str = include_str!("../data/chains.toml");

// ── Parsed TOML shape

#[derive(Debug, Deserialize)]
struct TomlFile {
    chains: Vec<TomlChain>,
}

#[derive(Debug, Deserialize)]
struct TomlChain {
    id: String,
    name: String,
    symbol: String,
    gas_token_symbol: String,
    search_keywords: Vec<String>,
    category: String,
    is_evm: bool,
    #[serde(default)]
    address_prefix_hint: String,
    color: String,
    asset_name: String,
    token_standard: String,
    contract_address_prompt: String,
    native_coingecko_id: String,
    native_decimals: u32,
    native_asset_name: String,
    tags: Vec<String>,
    comment: String,
    family: String,
    consensus: String,
    state_model: String,
    primary_use: String,
    derivation_path: Vec<TomlDerivationPathEntry>,
    total_circulation_model: String,
}

#[derive(Debug, Deserialize)]
struct TomlDerivationPathEntry {
    tag: String,
    path: String,
    #[serde(default)]
    is_default: bool,
    #[serde(default)]
    note: String,
}

// ── Public serialized shape — exposed to Swift via UniFFI

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainDerivationPathEntry {
    pub tag: String,
    pub path: String,
    pub is_default: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainEntry {
    pub id: String,
    pub name: String,
    pub symbol: String,
    /// A terse example of what an address on this chain looks like, or empty.
    ///
    /// Two Swift tables held this: a fourteen-arm switch of format examples
    /// and an eleven-entry dictionary of sentences built around them. It is a
    /// fact about the chain, so it is a catalog column.
    pub address_prefix_hint: String,
    pub gas_token_symbol: String,
    pub search_keywords: Vec<String>,
    pub category: String,
    pub is_evm: bool,
    pub color: String,
    pub asset_name: String,
    pub token_standard: String,
    pub contract_address_prompt: String,
    pub native_coingecko_id: String,
    pub native_decimals: u32,
    pub native_asset_name: String,
    pub tags: Vec<String>,
    pub comment: String,
    pub family: String,
    pub consensus: String,
    pub state_model: String,
    pub primary_use: String,
    pub derivation_path: Vec<ChainDerivationPathEntry>,
    pub total_circulation_model: String,
}

impl From<TomlDerivationPathEntry> for ChainDerivationPathEntry {
    fn from(value: TomlDerivationPathEntry) -> Self {
        Self {
            tag: value.tag,
            path: value.path,
            is_default: value.is_default,
            note: value.note,
        }
    }
}

// ── Static catalog

static CATALOG: LazyLock<Vec<ChainEntry>> = LazyLock::new(|| {
    let parsed: TomlFile = toml::from_str(CHAINS_TOML)
        .expect("chains.toml is embedded at compile time and must be valid TOML");
    parsed
        .chains
        .into_iter()
        .map(|c| ChainEntry {
            id: c.id,
            name: c.name,
            symbol: c.symbol,
            address_prefix_hint: c.address_prefix_hint,
            gas_token_symbol: c.gas_token_symbol,
            search_keywords: c.search_keywords,
            category: c.category,
            is_evm: c.is_evm,
            color: c.color,
            asset_name: c.asset_name,
            token_standard: c.token_standard,
            contract_address_prompt: c.contract_address_prompt,
            native_coingecko_id: c.native_coingecko_id,
            native_decimals: c.native_decimals,
            native_asset_name: c.native_asset_name,
            tags: c.tags,
            comment: c.comment,
            family: c.family,
            consensus: c.consensus,
            state_model: c.state_model,
            primary_use: c.primary_use,
            derivation_path: c.derivation_path.into_iter().map(Into::into).collect(),
            total_circulation_model: c.total_circulation_model,
        })
        .collect()
});

// ── Public API

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

pub(crate) fn default_derivation_path_template(chain_name: &str) -> Option<&'static str> {
    CATALOG
        .iter()
        .find(|c| c.name == chain_name)
        .and_then(default_template_of)
}

/// Same lookup keyed by the canonical chain id.
///
/// Prefer this over the display-name form: `registry::Chain::chain_display_name`
/// and the catalog's `name` do not always agree (the registry calls Internet
/// Computer `"ICP"`, the catalog calls it `"Internet Computer"`), so a
/// name-keyed lookup silently misses. Ids are frozen and match on both sides.
pub(crate) fn default_derivation_path_template_by_id(id: &str) -> Option<&'static str> {
    chain_by_str_id(id).and_then(default_template_of)
}

fn default_template_of(chain: &'static ChainEntry) -> Option<&'static str> {
    Some(chain)
        .and_then(|chain| {
            chain
                .derivation_path
                .iter()
                .find(|entry| entry.is_default)
                .or_else(|| chain.derivation_path.first())
        })
        .map(|entry| entry.path.as_str())
        .filter(|path| path.starts_with("m/"))
}

pub(crate) fn derivation_paths_for_chain(
    chain_name: &str,
) -> Option<&'static [ChainDerivationPathEntry]> {
    CATALOG
        .iter()
        .find(|c| c.name == chain_name)
        .map(|chain| chain.derivation_path.as_slice())
}
