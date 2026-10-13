//! One asset page per registered token ID. Symbols are display text.

use crate::chains::{self, ChainEntry};
use crate::tokens;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static CRYPTO_WIKI_TOML: &str = include_str!("../data/crypto-wiki.toml");

/// The wiki file: one row per coin, keyed by the catalog's token id.
#[derive(Debug, Deserialize)]
struct TomlAssetWikiFile {
    assets: Vec<TomlWikiAsset>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlWikiAsset {
    token_id: String,
    comment: String,
    #[serde(default)]
    total_circulation_model: String,
}

/// The prose, keyed by token id.
static PROSE: LazyLock<Vec<TomlWikiAsset>> = LazyLock::new(|| {
    let parsed: TomlAssetWikiFile = toml::from_str(CRYPTO_WIKI_TOML)
        .expect("crypto-wiki.toml is embedded at compile time and must be valid TOML");
    parsed.assets
});

/// What the file says about a coin: its description and its supply model.
/// Looked up by identity; a ticker is display text and two coins may share one.
fn prose_for(token_id: &str) -> (&'static str, &'static str) {
    PROSE
        .iter()
        .find(|a| a.token_id == token_id)
        .map(|a| (a.comment.as_str(), a.total_circulation_model.as_str()))
        .unwrap_or_default()
}

/// What a coin is. Where it is deployed is Known Tokens' to show.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
pub struct AssetWikiEntry {
    pub token_id: String,
    pub symbol: String,
    pub name: String,
    pub coingecko_id: String,
    pub color: Option<crate::chains::CatalogColor>,
    pub artwork_name: String,
    pub comment: String,
    /// Empty for a token: a supply model is written for the coins that have
    /// one, and nobody has written one for an ERC-20.
    pub total_circulation_model: String,
    pub tags: Vec<crate::tokens::TokenTag>,
}

static ASSETS: LazyLock<Vec<AssetWikiEntry>> = LazyLock::new(build);

fn build() -> Vec<AssetWikiEntry> {
    let mut out: Vec<AssetWikiEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Native coins, in catalog order, so the coin's home chain is the one that
    // names and colours it — the ten chains ETH runs on all say "Ethereum",
    // but Ethereum is the one that gets asked.
    for chain in chains::catalog() {
        if chain.native_coingecko_id.is_empty() || chain.gas_token_symbol.is_empty() {
            continue;
        }
        let token_id = tokens::deployment(&chain.native_deployment_id)
            .unwrap()
            .token_id
            .clone();
        if seen.insert(token_id) {
            out.push(entry_from_chain(chain));
        }
    }

    // Then the deployments. A coin already listed is not listed again: a
    // registered token can have native and contract deployments. Testnet
    // tokens are skipped for the reason testnet coins are: nothing prices them.
    for token in tokens::catalog()
        .iter()
        .filter(|t| !t.is_native() && !t.chain_id.is_testnet())
    {
        if seen.insert(token.token_id.clone()) {
            out.push(entry_from_token(token));
        }
    }

    for entry in out.iter_mut() {
        let (comment, circulation) = prose_for(&entry.token_id);
        entry.comment = comment.to_string();
        entry.total_circulation_model = circulation.to_string();
    }
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out
}

fn entry_from_chain(chain: &ChainEntry) -> AssetWikiEntry {
    let token = tokens::deployment(&chain.native_deployment_id).unwrap();
    AssetWikiEntry {
        token_id: token.token_id.clone(),
        symbol: chain.gas_token_symbol.clone(),
        name: chain.native_asset_display_name.clone(),
        coingecko_id: chain.native_coingecko_id.clone(),
        color: token.color,
        artwork_name: token.artwork_name.clone(),
        comment: String::new(),
        total_circulation_model: String::new(),
        tags: Vec::new(),
    }
}

fn entry_from_token(token: &tokens::TokenDeploymentEntry) -> AssetWikiEntry {
    AssetWikiEntry {
        token_id: token.token_id.clone(),
        symbol: token.symbol.clone(),
        name: token.name.clone(),
        coingecko_id: token.coingecko_id.clone(),
        color: token.color,
        artwork_name: token.artwork_name.clone(),
        comment: String::new(),
        total_circulation_model: String::new(),
        tags: token.tags.clone(),
    }
}

/// Every coin the app can hold, alphabetically by symbol.
#[uniffi::export]
pub fn list_asset_wiki() -> Vec<AssetWikiEntry> {
    ASSETS.clone()
}

#[cfg(test)]
mod the_wiki_is_one_asset_table {
    use super::*;

    fn asset(token_id: &str) -> &'static AssetWikiEntry {
        ASSETS
            .iter()
            .find(|a| a.token_id == token_id)
            .unwrap_or_else(|| panic!("{token_id} has no wiki row"))
    }

    /// Every coin appears once, with prose, and nothing appears twice.
    #[test]
    fn one_row_per_coin() {
        let symbols: std::collections::BTreeSet<&str> =
            ASSETS.iter().map(|a| a.token_id.as_str()).collect();
        assert_eq!(symbols.len(), ASSETS.len(), "a coin has two rows");
        for a in ASSETS.iter() {
            assert!(!a.comment.is_empty(), "{} has no description", a.symbol);
            // Pricing belongs to the registered identity. A verified legacy
            // asset may have no quote; the wiki must neither invent one nor
            // borrow the market identity of its redenominated replacement.
            let identity = tokens::catalog()
                .iter()
                .find(|token| token.token_id == a.token_id && !token.chain_id.is_testnet())
                .expect("wiki asset has a registered mainnet deployment");
            assert_eq!(a.coingecko_id, identity.coingecko_id, "{}", a.token_id);
        }
    }

    #[test]
    fn legacy_bittorrent_does_not_borrow_its_replacements_market_identity() {
        let old = asset("bittorrent-old");
        let replacement = asset("bittorrent");
        assert!(old.coingecko_id.is_empty());
        assert!(!replacement.coingecko_id.is_empty());
        assert_ne!(old.token_id, replacement.token_id);
    }

    /// ETH is one row across Ethereum and its rollups.
    ///
    /// This is the whole point: the chain wiki had ten pages that were really
    /// about the networks, and no page about the coin.
    #[test]
    fn a_coin_native_to_many_chains_is_one_row() {
        assert_eq!(
            ASSETS.iter().filter(|a| a.token_id == "ethereum").count(),
            1
        );
        // Named by its home chain, because the catalog lists Ethereum before
        // its rollups.
        let eth = asset("ethereum");
        assert_eq!(eth.name, "Ethereum");
        assert!(!eth.total_circulation_model.is_empty());
    }

    /// `crypto-wiki.toml` has no row nothing claims.
    ///
    /// The table is built from the catalogs and only looks prose up, so a row
    /// for a coin the app dropped would sit there unread. This is the check
    /// the other direction.
    #[test]
    fn the_file_documents_no_coin_the_app_does_not_have() {
        let documented: std::collections::BTreeSet<&str> =
            PROSE.iter().map(|a| a.token_id.as_str()).collect();
        assert_eq!(
            documented.len(),
            PROSE.len(),
            "a coin has two rows in the file"
        );
        let held: std::collections::BTreeSet<&str> =
            ASSETS.iter().map(|a| a.token_id.as_str()).collect();
        assert_eq!(documented, held, "the file and the catalogs disagree");
    }

    /// The table covers both catalogs and invents nothing.
    #[test]
    fn the_table_is_exactly_the_two_catalogs() {
        let expected: std::collections::BTreeSet<&str> = crate::tokens::catalog()
            .iter()
            .filter(|t| !t.chain_id.is_testnet())
            .map(|t| t.token_id.as_str())
            .collect();
        let got: std::collections::BTreeSet<&str> =
            ASSETS.iter().map(|a| a.token_id.as_str()).collect();
        assert_eq!(expected, got);
    }
}
