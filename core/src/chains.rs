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
    tags: Vec<String>,
    comment: String,
    family: String,
    consensus: String,
    state_model: String,
    total_circulation_model: String,
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
    #[serde(default)]
    note: String,
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
    /// Whether the chain has an RPC that answers "what tokens does this
    /// address hold?" without being told what to look for. False for the EVM
    /// family and NEAR, where a token contract only answers about a holder you
    /// name, so listing holdings needs an indexer rather than a node.
    pub enumerates_holdings: bool,
    pub contract_address_prompt: String,
    pub native_coingecko_id: String,
    pub native_decimals: u32,
    pub native_asset_name: String,
    pub tags: Vec<String>,
    pub comment: String,
    pub family: String,
    pub consensus: String,
    pub state_model: String,
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
        tags: c.tags.clone(),
        comment: c.comment.clone(),
        family: c.family.clone(),
        consensus: c.consensus.clone(),
        state_model: c.state_model.clone(),
        derivation_path: c
            .derivation_path
            .iter()
            .map(|d| ChainDerivationPathEntry {
                tag: d.tag.clone(),
                path: d.path.clone(),
                is_default: d.is_default,
                note: d.note.clone(),
            })
            .collect(),
        total_circulation_model: c.total_circulation_model.clone(),
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
                note: d.note.clone(),
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

        // The editorial block documents a *chain*. The wiki iterates entries
        // and skips those with no `family`, so leaving these empty is what
        // keeps a chain from appearing once per network.
        entry.tags = Vec::new();
        entry.comment = String::new();
        entry.family = String::new();
        entry.consensus = String::new();
        entry.state_model = String::new();
        entry.total_circulation_model = String::new();

        out.push(entry);
    }
    out
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

    /// The wiki documents chains, not networks — it skips rows with no
    /// `family`, and a network has none.
    #[test]
    fn only_chains_carry_the_editorial_block() {
        for chain in Chain::all() {
            let e = entry(chain.str_id());
            if chain.is_testnet() {
                assert!(e.family.is_empty(), "{} would appear in the wiki", e.id);
                assert!(e.comment.is_empty());
                assert!(e.tags.is_empty());
            } else {
                assert!(!e.family.is_empty(), "{} has no wiki entry", e.id);
            }
        }
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
