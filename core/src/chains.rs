//! Built-in chain registry.
//!
//! Two files, embedded at compile time. `core/data/chains.toml` holds what the
//! app *computes* with — derivation paths, decimals, address formats, token
//! standards. `core/data/chain-wiki.toml` holds what a reader reads, and
//! nothing computes anything from it.
//!
//! They are separate because a wrong value in the first is a wrong address or
//! a wrong balance, and a wrong value in the second is a wrong sentence on a
//! page. The boundary is a type, not a convention: the editorial fields exist
//! only on [`ChainWikiEntry`], so no code outside the wiki can reach them.
//!
//! Call [`list_all_chains`] for all chain entries (mainnet + testnet), and
//! [`list_chain_wiki`] for the wiki rows (chains only — a testnet is not a
//! different chain).

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static CHAINS_TOML: &str = include_str!("../data/chains.toml");
static CHAIN_WIKI_TOML: &str = include_str!("../data/chain-wiki.toml");

// ── Parsed TOML shape

#[derive(Debug, Deserialize)]
struct TomlFile {
    chains: Vec<TomlChain>,
    networks: Vec<TomlNetwork>,
}

/// What a chain is — one row however many networks it runs.
#[derive(Debug, Deserialize)]
struct TomlChain {
    id: String,
    name: String,
    symbol: String,
    gas_token_symbol: String,
    search_keywords: Vec<String>,
    category: String,
    color: String,
    asset_name: String,
    #[serde(default)]
    address_prefix_hint: String,
    token_standard: String,
    native_coingecko_id: String,
    native_decimals: u32,
    native_asset_name: String,
    #[serde(default)]
    enumerates_holdings: bool,
    derivation_path: Vec<TomlDerivationPathEntry>,
}

/// One network of a chain — a testnet. Inherits everything it does not state.
#[derive(Debug, Deserialize)]
struct TomlNetwork {
    chain: String,
    id: String,
    name: String,
    search_keywords: Vec<String>,
    #[serde(default)]
    address_prefix_hint: Option<String>,
    derivation_path: Vec<TomlDerivationPathEntry>,
}

#[derive(Debug, Deserialize)]
struct TomlDerivationPathEntry {
    tag: String,
    path: String,
    #[serde(default)]
    is_default: bool,
}

/// The wiki file: one row per chain, joined to `chains.toml` by `chain`.
#[derive(Debug, Deserialize)]
struct TomlWikiFile {
    chains: Vec<TomlWikiChain>,
}

#[derive(Debug, Deserialize)]
struct TomlWikiChain {
    chain: String,
    tags: Vec<String>,
    comment: String,
    family: String,
    consensus: String,
    state_model: String,
}

/// The prompt shown above a contract-address field, from the standard the
/// chain hosts.
///
/// Was a column: seventy-eight rows carrying one of seven strings, computable
/// from the `token_standard` beside it.
fn contract_address_prompt_for(token_standard: &str) -> String {
    match token_standard {
        "" => "",
        "AIP-21" => "Fungible Asset Metadata or Package Address",
        "NEP-141" => "Contract Account ID",
        "SPL" => "Mint Address",
        "Sui Coin" => "Coin Standard Type",
        "TEP-74" => "Jetton Master Address",
        // ARC-20, BEP-20, ERC-20, TRC-20 — the contract-address families.
        _ => "Contract Address",
    }
    .to_string()
}

/// Whether the chain is EVM-compatible, from the family it belongs to.
///
/// Was a column. It could not be derived while `category` doubled as a
/// network-kind flag — every testnet's category was `"testnet"`, whatever
/// family it actually belonged to.
fn is_evm_for(category: &str) -> bool {
    matches!(category, "evm-l1" | "evm-l2")
}

// ── Public serialized shape — exposed to Swift via UniFFI

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainDerivationPathEntry {
    pub tag: String,
    pub path: String,
    pub is_default: bool,
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
    /// Whether the chain has an RPC that answers "what tokens does this
    /// address hold?" without being told what to look for. False for the EVM
    /// family and NEAR, where a token contract only answers about a holder you
    /// name, so listing holdings needs an indexer rather than a node.
    pub enumerates_holdings: bool,
    pub contract_address_prompt: String,
    pub native_coingecko_id: String,
    pub native_decimals: u32,
    pub native_asset_name: String,
    pub derivation_path: Vec<ChainDerivationPathEntry>,
}

/// What a *chain* is — the facts that have no coin to belong to.
///
/// Ten chains share ETH, so "Base is an optimistic rollup" cannot live on an
/// asset page; that is what this is for. What a *coin* is lives on
/// [`crate::wiki::AssetWikiEntry`], which is the wiki's index — a holder thinks
/// in coins, and this is one level down from there.
///
/// Kept out of [`ChainEntry`] so that nothing in the send, derive or fetch
/// paths can read it — sixty-one percent of the catalog's bytes used to travel
/// the FFI on every `list_all_chains()` call to serve one screen.
///
/// There is a row per chain and none per network, so the wiki no longer filters
/// networks out by testing `family` for emptiness. The table is the filter.
#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainWikiEntry {
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub tags: Vec<String>,
    pub comment: String,
    pub family: String,
    pub consensus: String,
    pub state_model: String,
    pub derivation_path: Vec<ChainDerivationPathEntry>,
}

impl From<TomlDerivationPathEntry> for ChainDerivationPathEntry {
    fn from(value: TomlDerivationPathEntry) -> Self {
        Self {
            tag: value.tag,
            path: value.path,
            is_default: value.is_default,
        }
    }
}

// ── Static catalog

static CATALOG: LazyLock<Vec<ChainEntry>> = LazyLock::new(|| {
    let parsed: TomlFile = toml::from_str(CHAINS_TOML)
        .expect("chains.toml is embedded at compile time and must be valid TOML");

    let entry_of = |c: &TomlChain| ChainEntry {
        id: c.id.clone(),
        name: c.name.clone(),
        symbol: c.symbol.clone(),
        address_prefix_hint: c.address_prefix_hint.clone(),
        gas_token_symbol: c.gas_token_symbol.clone(),
        search_keywords: c.search_keywords.clone(),
        category: c.category.clone(),
        is_evm: is_evm_for(&c.category),
        color: c.color.clone(),
        asset_name: c.asset_name.clone(),
        token_standard: c.token_standard.clone(),
        enumerates_holdings: c.enumerates_holdings,
        contract_address_prompt: contract_address_prompt_for(&c.token_standard),
        native_coingecko_id: c.native_coingecko_id.clone(),
        native_decimals: c.native_decimals,
        native_asset_name: c.native_asset_name.clone(),
        derivation_path: c
            .derivation_path
            .iter()
            .map(|d| ChainDerivationPathEntry {
                tag: d.tag.clone(),
                path: d.path.clone(),
                is_default: d.is_default,
            })
            .collect(),
    };

    let by_id: std::collections::HashMap<&str, &TomlChain> = parsed
        .chains
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();

    let mut out: Vec<ChainEntry> = parsed.chains.iter().map(entry_of).collect();

    for n in &parsed.networks {
        // A network naming a chain the file does not define is a build-time
        // mistake, not a row to skip: the entry would carry an id and nothing
        // that says what it is.
        let chain = by_id.get(n.chain.as_str()).unwrap_or_else(|| {
            panic!("chains.toml: network {} names unknown chain {}", n.id, n.chain)
        });
        let mut entry = entry_of(chain);
        entry.id = n.id.clone();
        entry.name = n.name.clone();
        entry.search_keywords = n.search_keywords.clone();
        entry.derivation_path = n
            .derivation_path
            .iter()
            .map(|d| ChainDerivationPathEntry {
                tag: d.tag.clone(),
                path: d.path.clone(),
                is_default: d.is_default,
            })
            .collect();

        // What a network does *not* inherit.
        //
        // A testnet hosts no tokens, and a testnet asset has no price. The
        // coingecko id used to be copied from the mainnet and then overridden
        // elsewhere — a field that could only ever be wrong.
        entry.token_standard = String::new();
        entry.contract_address_prompt = String::new();
        entry.native_coingecko_id = String::new();

        // An address hint describes a network's address format, and a
        // testnet's differs — Bitcoin's is `tb1…`, not `bc1q…`. A network
        // states its own or has none.
        entry.address_prefix_hint = n.address_prefix_hint.clone().unwrap_or_default();

        out.push(entry);
    }
    out
});

static WIKI: LazyLock<Vec<ChainWikiEntry>> = LazyLock::new(|| {
    let parsed: TomlWikiFile = toml::from_str(CHAIN_WIKI_TOML)
        .expect("chain-wiki.toml is embedded at compile time and must be valid TOML");

    parsed
        .chains
        .into_iter()
        .map(|w| {
            // A wiki row naming a chain the catalog does not define is a
            // build-time mistake, not a row to skip: the page would have prose
            // and no name to put it under.
            let chain = chain_by_str_id(&w.chain)
                .unwrap_or_else(|| panic!("chain-wiki.toml: unknown chain {}", w.chain));
            ChainWikiEntry {
                id: chain.id.clone(),
                name: chain.name.clone(),
                symbol: chain.symbol.clone(),
                tags: w.tags,
                comment: w.comment,
                family: w.family,
                consensus: w.consensus,
                state_model: w.state_model,
                derivation_path: chain.derivation_path.clone(),
            }
        })
        .collect()
});

// ── Public API

/// Return all chain entries (mainnet + testnet).
#[uniffi::export]
pub fn list_all_chains() -> Vec<ChainEntry> {
    CATALOG.clone()
}

/// Return the chain wiki rows — one per chain, never one per network.
#[uniffi::export]
pub fn list_chain_wiki() -> Vec<ChainWikiEntry> {
    WIKI.clone()
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


#[cfg(test)]
mod the_catalog_is_two_tables {
    use super::*;
    use crate::registry::Chain;

    fn entry(id: &str) -> &'static ChainEntry {
        CATALOG.iter().find(|c| c.id == id).expect("a catalog row")
    }

    /// Every network resolves to a chain, and every chain runs at least the
    /// network it is.
    #[test]
    fn the_two_tables_cover_each_other() {
        let parsed: TomlFile = toml::from_str(CHAINS_TOML).expect("valid TOML");
        let ids: std::collections::HashSet<&str> =
            parsed.chains.iter().map(|c| c.id.as_str()).collect();
        for n in &parsed.networks {
            assert!(
                ids.contains(n.chain.as_str()),
                "{} names unknown chain {}",
                n.id,
                n.chain
            );
        }
        assert_eq!(CATALOG.len(), parsed.chains.len() + parsed.networks.len());
        // And the registry agrees about which is which.
        for n in &parsed.networks {
            let chain = Chain::from_str_id(&n.id).expect("the registry knows it");
            assert!(chain.is_testnet(), "{} is a network row and not a testnet", n.id);
            assert_eq!(chain.mainnet_counterpart().str_id(), n.chain);
        }
    }

    /// A network inherits its chain's technical facts.
    ///
    /// They were columns on every testnet row — eight of them restated
    /// verbatim, which is eight chances for one to drift.
    #[test]
    fn a_network_inherits_what_it_does_not_state() {
        let (main, net) = (entry("ethereum"), entry("ethereum-sepolia"));
        for (field, a, b) in [
            ("symbol", &main.symbol, &net.symbol),
            ("gas_token_symbol", &main.gas_token_symbol, &net.gas_token_symbol),
            ("color", &main.color, &net.color),
            ("asset_name", &main.asset_name, &net.asset_name),
            ("native_asset_name", &main.native_asset_name, &net.native_asset_name),
            ("category", &main.category, &net.category),
        ] {
            assert_eq!(a, b, "{field} did not carry through to the network");
        }
        assert_eq!(main.native_decimals, net.native_decimals);
        assert_eq!(main.is_evm, net.is_evm);
        // And it states its own name and derivation path — a testnet derives
        // down a different coin type.
        assert_ne!(main.name, net.name);
        let paths = |e: &ChainEntry| -> Vec<String> {
            e.derivation_path.iter().map(|d| d.path.clone()).collect()
        };
        assert_ne!(
            paths(main),
            paths(net),
            "the network inherited its chain's derivation path"
        );
    }

    /// A testnet asset has no price and hosts no tokens, structurally.
    ///
    /// The coingecko id used to be copied from the mainnet — Sepolia's said
    /// `"ethereum"` — and something else had to override it. A field that can
    /// only ever be wrong.
    #[test]
    fn a_network_never_inherits_a_price_or_a_token_standard() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            let e = entry(chain.str_id());
            assert!(
                e.native_coingecko_id.is_empty(),
                "{} carries a price id",
                e.id
            );
            assert!(e.token_standard.is_empty(), "{} claims to host tokens", e.id);
            assert!(e.contract_address_prompt.is_empty());
        }
    }

    /// An address hint describes a network's format, so it is never inherited:
    /// Bitcoin's is `bc1q…` and its testnet's is not.
    #[test]
    fn an_address_hint_is_never_inherited() {
        assert_eq!(entry("bitcoin").address_prefix_hint, "bc1q…");
        assert_ne!(
            entry("bitcoin-testnet").address_prefix_hint,
            entry("bitcoin").address_prefix_hint,
            "a testnet showed its mainnet's address format"
        );
    }

    /// The wiki documents chains, not networks, and it says so by having a
    /// table rather than by testing a field for emptiness.
    ///
    /// The editorial block used to be six columns on the chain row, which
    /// meant the network loop had to remember to blank all six — and when it
    /// did not, thirty-two testnets appeared in the wiki as duplicate chains.
    /// That is not a bug you can have when networks have no wiki row to
    /// inherit.
    #[test]
    fn the_wiki_covers_every_chain_and_no_network() {
        let ids: std::collections::HashSet<&str> = WIKI.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids.len(), WIKI.len(), "a chain has two wiki rows");
        for chain in Chain::all() {
            let documented = ids.contains(chain.str_id());
            if chain.is_testnet() {
                assert!(!documented, "{} is a network and has a wiki row", chain.str_id());
            } else {
                assert!(documented, "{} has no wiki row", chain.str_id());
            }
        }
        assert_eq!(WIKI.len(), Chain::all().filter(|c| !c.is_testnet()).count());
    }

    /// The wiki joins to the catalog rather than restating it.
    #[test]
    fn a_wiki_row_takes_its_name_from_the_catalog() {
        let dot = WIKI.iter().find(|w| w.id == "polkadot").expect("a wiki row");
        let catalog = entry("polkadot");
        assert_eq!(dot.name, catalog.name);
        assert_eq!(dot.symbol, catalog.symbol);
        assert_eq!(dot.derivation_path.len(), catalog.derivation_path.len());
        assert!(!dot.family.is_empty());
    }

    /// `is_evm` and the contract prompt are computed, not stored.
    ///
    /// `is_evm` could not be derived while `category` doubled as a
    /// network-kind flag: every testnet's category was `"testnet"`, whatever
    /// family it belonged to.
    #[test]
    fn the_derived_columns_agree_with_what_they_derive_from() {
        for e in CATALOG.iter() {
            assert_eq!(e.is_evm, is_evm_for(&e.category), "{}", e.id);
            assert_eq!(
                e.contract_address_prompt,
                if e.token_standard.is_empty() {
                    String::new()
                } else {
                    contract_address_prompt_for(&e.token_standard)
                },
                "{}",
                e.id
            );
        }
        // The EVM family is exactly the two EVM categories.
        for chain in Chain::all() {
            assert_eq!(
                chain.is_evm(),
                matches!(entry(chain.str_id()).category.as_str(), "evm-l1" | "evm-l2"),
                "{} disagrees about being EVM",
                chain.str_id()
            );
        }
    }
}
