//! The asset wiki: what a coin is, and everywhere the app can hold it.
//!
//! One table joined from three files. `crypto-wiki.toml` says what a coin is,
//! `chains.toml` says which chains run it natively, and `tokens.toml` says
//! which chains host it as a contract. A coin can be both — CRO is native to
//! Cronos and an ERC-20 on Ethereum, and that is one row with two places.
//!
//! The app is organised by asset everywhere else: a dashboard row is a coin
//! however many chains it sits on, the pin list is by symbol, holdings are
//! coins. The wiki was the one screen organised by chain, so a holder of USDC
//! could read about Base and not about USDC.
//!
//! Keyed on the coin's own symbol. For a native coin that is
//! `gas_token_symbol`, never the chain's `symbol`: eleven chains disagree, and
//! Arbitrum is ARB but runs on ETH.

use crate::chains::{self, ChainEntry};
use crate::tokens;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static CRYPTO_WIKI_TOML: &str = include_str!("../data/crypto-wiki.toml");

/// The wiki file: one row per coin, keyed by the coin's own symbol.
#[derive(Debug, Deserialize)]
struct TomlAssetWikiFile {
    assets: Vec<TomlWikiAsset>,
}

#[derive(Debug, Deserialize)]
struct TomlWikiAsset {
    asset: String,
    comment: String,
    #[serde(default)]
    total_circulation_model: String,
}

/// The prose, keyed by coin symbol. It used to be loaded in `chains.rs`, so
/// this module had to call back into the chain module for its own data.
static PROSE: LazyLock<Vec<TomlWikiAsset>> = LazyLock::new(|| {
    let parsed: TomlAssetWikiFile = toml::from_str(CRYPTO_WIKI_TOML)
        .expect("crypto-wiki.toml is embedded at compile time and must be valid TOML");
    parsed.assets
});

/// What the file says about a coin: its description and its supply model.
fn prose_for(symbol: &str) -> (&'static str, &'static str) {
    PROSE
        .iter()
        .find(|a| a.asset == symbol)
        .map(|a| (a.comment.as_str(), a.total_circulation_model.as_str()))
        .unwrap_or_default()
}

/// One place a coin exists: a chain, and either a contract or nothing.
///
/// A native coin has an empty `contract` on purpose. It is the honest answer —
/// there is no contract to show — and it is what tells the two apart without a
/// second flag that could disagree.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
pub struct AssetWikiPlace {
    pub chain_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract: String,
    pub decimals: u32,
    pub is_native: bool,
}

/// What a coin is, and everywhere it lives.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
pub struct AssetWikiEntry {
    pub symbol: String,
    pub name: String,
    pub coin_gecko_id: String,
    pub color: String,
    pub asset_name: String,
    pub comment: String,
    /// Empty for a token: a supply model is written for the coins that have
    /// one, and nobody has written one for an ERC-20.
    pub total_circulation_model: String,
    pub tags: Vec<String>,
    /// Native places first, then contracts by chain name. The first is where
    /// the coin is from, which is what a page should lead with.
    pub lives_on: Vec<AssetWikiPlace>,
}

static ASSETS: LazyLock<Vec<AssetWikiEntry>> = LazyLock::new(build);

fn build() -> Vec<AssetWikiEntry> {
    let mut out: Vec<AssetWikiEntry> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // Native coins, in catalog order, so the coin's home chain is the one that
    // names and colours it — the ten chains ETH runs on all say "Ethereum",
    // but Ethereum is the one that gets asked.
    for chain in chains::catalog() {
        if chain.native_coingecko_id.is_empty() || chain.gas_token_symbol.is_empty() {
            continue;
        }
        let slot = *index
            .entry(chain.gas_token_symbol.clone())
            .or_insert_with(|| {
                out.push(entry_from_chain(chain));
                out.len() - 1
            });
        out[slot].lives_on.push(AssetWikiPlace {
            chain_id: chain.id.clone(),
            chain_name: chain.name.clone(),
            token_standard: "Native".to_string(),
            contract: String::new(),
            decimals: chain.native_decimals,
            is_native: true,
        });
    }

    // Then the deployments. A coin already listed gains places rather than a
    // second row: CRO is native to Cronos and a contract on Ethereum.
    for token in tokens::catalog() {
        let chain_name = chains::chain_by_str_id(&token.chain)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| token.chain.clone());
        let slot = *index.entry(token.symbol.clone()).or_insert_with(|| {
            out.push(entry_from_token(token));
            out.len() - 1
        });
        out[slot].lives_on.push(AssetWikiPlace {
            chain_id: token.chain.clone(),
            chain_name,
            token_standard: token.token_standard.clone(),
            contract: token.contract.clone(),
            decimals: token.decimals,
            is_native: false,
        });
    }

    for entry in out.iter_mut() {
        // Native places first and in catalog order, so ETH leads with
        // Ethereum rather than with Arbitrum; contracts after them by name.
        // The sort is stable, which is what keeps the catalog order.
        entry.lives_on.sort_by(|a, b| {
            b.is_native.cmp(&a.is_native).then_with(|| {
                if a.is_native {
                    std::cmp::Ordering::Equal
                } else {
                    a.chain_name.cmp(&b.chain_name)
                }
            })
        });
        let (comment, circulation) = prose_for(&entry.symbol);
        entry.comment = comment.to_string();
        entry.total_circulation_model = circulation.to_string();
    }
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out
}

fn entry_from_chain(chain: &ChainEntry) -> AssetWikiEntry {
    AssetWikiEntry {
        symbol: chain.gas_token_symbol.clone(),
        name: chain.native_asset_name.clone(),
        coin_gecko_id: chain.native_coingecko_id.clone(),
        color: chain.color.clone(),
        asset_name: chain.asset_name.clone(),
        comment: String::new(),
        total_circulation_model: String::new(),
        tags: Vec::new(),
        lives_on: Vec::new(),
    }
}

fn entry_from_token(token: &tokens::TokenEntry) -> AssetWikiEntry {
    AssetWikiEntry {
        symbol: token.symbol.clone(),
        name: token.name.clone(),
        coin_gecko_id: token.coingecko_id.clone(),
        color: token.color.clone(),
        asset_name: token.asset_name.clone(),
        comment: String::new(),
        total_circulation_model: String::new(),
        tags: token.tags.clone(),
        lives_on: Vec::new(),
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

    fn asset(symbol: &str) -> &'static AssetWikiEntry {
        ASSETS
            .iter()
            .find(|a| a.symbol == symbol)
            .unwrap_or_else(|| panic!("{symbol} has no wiki row"))
    }

    /// Every coin appears once, with prose, and nothing appears twice.
    #[test]
    fn one_row_per_coin() {
        let symbols: std::collections::BTreeSet<&str> =
            ASSETS.iter().map(|a| a.symbol.as_str()).collect();
        assert_eq!(symbols.len(), ASSETS.len(), "a coin has two rows");
        assert_eq!(ASSETS.len(), 66);
        for a in ASSETS.iter() {
            assert!(!a.comment.is_empty(), "{} has no description", a.symbol);
            assert!(!a.lives_on.is_empty(), "{} lives nowhere", a.symbol);
            // A coin enabled anywhere must be priceable. USD1 and WLFI once
            // had no id at all; both are disabled, so nothing showed an
            // unpriced balance, but enabling one would have.
            assert!(!a.coin_gecko_id.is_empty(), "{} has no market id", a.symbol);
        }
    }

    /// ETH is one row over ten chains, not ten rows.
    ///
    /// This is the whole point: the chain wiki had ten pages that were really
    /// about the networks, and no page about the coin.
    #[test]
    fn a_coin_native_to_ten_chains_is_one_row() {
        let eth = asset("ETH");
        assert_eq!(eth.lives_on.len(), 10);
        assert!(eth.lives_on.iter().all(|p| p.is_native));
        assert!(eth.lives_on.iter().all(|p| p.contract.is_empty()));
        // Presented as its home chain, because native places sort first and
        // the catalog lists Ethereum before its rollups.
        assert_eq!(eth.lives_on[0].chain_name, "Ethereum");
        assert_eq!(eth.name, "Ethereum");
        assert!(!eth.total_circulation_model.is_empty());
    }

    /// A coin that is native on one chain and a contract on another is one
    /// row with both kinds of place.
    ///
    /// CRO is the only one, and it only works because the two catalogs were
    /// made to agree about its market-data id — they named it `crypto-com-chain`
    /// and `cronos`, and priced it five hundred times apart.
    #[test]
    fn a_coin_can_be_native_here_and_a_contract_there() {
        let cro = asset("CRO");
        assert_eq!(cro.lives_on.len(), 2);
        assert!(cro.lives_on[0].is_native);
        assert_eq!(cro.lives_on[0].chain_name, "Cronos");
        assert!(!cro.lives_on[1].is_native);
        assert_eq!(cro.lives_on[1].chain_name, "Ethereum");
        assert_eq!(cro.lives_on[1].token_standard, "ERC-20");
        assert!(!cro.lives_on[1].contract.is_empty());
    }

    /// A token's places are its deployments, with the per-chain facts intact.
    #[test]
    fn a_token_lists_a_contract_per_chain() {
        let usdc = asset("USDC");
        assert_eq!(usdc.lives_on.len(), 13);
        assert!(usdc.lives_on.iter().all(|p| !p.is_native));
        assert!(usdc.lives_on.iter().all(|p| !p.contract.is_empty()));
        // Decimals stay per place — the field the token split refused to fold.
        let widths: std::collections::BTreeSet<u32> =
            usdc.lives_on.iter().map(|p| p.decimals).collect();
        assert!(!widths.is_empty());
        // Chain names are resolved, not left as ids.
        assert!(usdc.lives_on.iter().any(|p| p.chain_name == "Ethereum"));
    }

    /// `crypto-wiki.toml` has no row nothing claims.
    ///
    /// The table is built from the catalogs and only looks prose up, so a row
    /// for a coin the app dropped would sit there unread. This is the check
    /// the other direction.
    #[test]
    fn the_file_documents_no_coin_the_app_does_not_have() {
        let documented: std::collections::BTreeSet<&str> =
            PROSE.iter().map(|a| a.asset.as_str()).collect();
        assert_eq!(documented.len(), PROSE.len(), "a coin has two rows in the file");
        let held: std::collections::BTreeSet<&str> =
            ASSETS.iter().map(|a| a.symbol.as_str()).collect();
        assert_eq!(documented, held, "the file and the catalogs disagree");
    }

    /// The table covers both catalogs and invents nothing.
    #[test]
    fn the_table_is_exactly_the_two_catalogs() {
        let mut expected: std::collections::BTreeSet<&str> = crate::chains::catalog()
            .iter()
            .filter(|c| !c.native_coingecko_id.is_empty())
            .map(|c| c.gas_token_symbol.as_str())
            .collect();
        expected.extend(crate::tokens::catalog().iter().map(|t| t.symbol.as_str()));
        let got: std::collections::BTreeSet<&str> =
            ASSETS.iter().map(|a| a.symbol.as_str()).collect();
        assert_eq!(expected, got);
    }
}
