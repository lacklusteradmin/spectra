//! Concrete network registry embedded from `chains.toml`.
//! Mainnets and testnets are equal records. Native token metadata in the public
//! projection is joined from `tokens.toml`; it is never stored as a network fact.
//! `chain-ui.toml` supplies presentation by network ID; `chain-wiki.toml` holds prose.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static CHAINS_TOML: &str = include_str!("../data/chains.toml");
static CHAIN_UI_TOML: &str = include_str!("../data/chain-ui.toml");
static CHAIN_WIKI_TOML: &str = include_str!("../data/chain-wiki.toml");

/// A catalog entry's brand colour, from a closed palette.
///
/// Was a free `String` on chains, tokens and the wiki, rendered by a Swift
/// switch whose `default` was the app's accent colour — so a misspelt or new
/// name in a TOML file drew in the wrong colour and nothing failed. Per-chain
/// presentation facts belong to the catalog; this makes the catalog's the only
/// spelling, checked when the file is parsed, and the app's switch exhaustive.
/// The setup picker's section for a chain. A display grouping only: it never
/// decides a protocol capability (`is_evm` is the registry's). Parsed from the
/// catalog, so a misspelt section fails when the file loads rather than
/// dropping the chain from the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "kebab-case")]
pub enum ChainCategory {
    BitcoinFamily,
    EvmL1,
    EvmL2,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum CatalogColor {
    Blue,
    Cyan,
    Gray,
    Green,
    Indigo,
    Mint,
    Orange,
    Pink,
    Purple,
    Red,
    Teal,
    Yellow,
}

// ── Parsed TOML shape

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlFile {
    chains: Vec<TomlChain>,
}

/// One concrete network. Mainnets and testnets have the same required fields.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlChain {
    id: String,
    name: String,
    family: String,
    environment: String,
    token_standard: String,
    #[serde(default)]
    enumerates_holdings: bool,
    derivation_path: Vec<TomlDerivationPathEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlUiFile {
    chains: Vec<TomlChainUi>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlChainUi {
    chain_id: String,
    search_keywords: Vec<String>,
    category: ChainCategory,
    /// Position in the setup picker's short list, or absent.
    #[serde(default)]
    popular_rank: Option<u8>,
    color: CatalogColor,
    artwork_name: String,
    #[serde(default)]
    address_prefix_hint: String,
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

// ── Public serialized shape — exposed to Swift via UniFFI

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainDerivationPathEntry {
    pub tag: String,
    pub path: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainEntry {
    pub family: String,
    pub is_testnet: bool,
    pub id: String,
    pub name: String,
    pub native_deployment_id: String,
    /// A terse example of what an address on this chain looks like, or empty.
    ///
    /// Two Swift tables held this: a fourteen-arm switch of format examples
    /// and an eleven-entry dictionary of sentences built around them. It is a
    /// fact about the chain, so it is a catalog column.
    pub address_prefix_hint: String,
    pub gas_token_symbol: String,
    pub search_keywords: Vec<String>,
    pub category: ChainCategory,
    /// Where this chain sits in the setup picker's short list, or `None` for
    /// the chains that reach it only through "browse all".
    ///
    /// The picker held this as an eight-id array in Swift, which is a per-chain
    /// fact in a caller-owned list: adding a chain to the catalog could not put
    /// it there, and removing one left an id the filter silently dropped.
    pub popular_rank: Option<u8>,
    pub is_evm: bool,
    pub color: CatalogColor,
    pub artwork_name: String,
    pub token_standard: String,
    /// Whether the chain has an RPC that answers "what tokens does this
    /// address hold?" without being told what to look for. False for the EVM
    /// family and NEAR, where a token contract only answers about a holder you
    /// name, so listing holdings needs an indexer rather than a node.
    pub enumerates_holdings: bool,
    pub contract_address_prompt: String,
    pub native_coingecko_id: String,
    pub native_decimals: u32,
    pub native_asset_display_name: String,
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
    pub native_deployment_id: String,
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

static CATALOG: LazyLock<Vec<ChainEntry>> =
    LazyLock::new(|| load_catalog(CHAINS_TOML, CHAIN_UI_TOML));

fn load_catalog(chains: &str, presentation: &str) -> Vec<ChainEntry> {
    let parsed: TomlFile = toml::from_str(chains)
        .expect("chains.toml is embedded at compile time and must be valid TOML");

    let ui: TomlUiFile = toml::from_str(presentation)
        .expect("chain-ui.toml must contain valid network presentation records");
    let mut ui_by_id = std::collections::HashMap::new();
    for row in ui.chains {
        let id = row.chain_id.clone();
        assert!(
            ui_by_id.insert(id.clone(), row).is_none(),
            "duplicate UI chain_id {id}"
        );
    }

    // Chain discriminants already index this catalog. Use that same ordering
    // here; from_str_id/entry would recursively initialize CATALOG.
    assert_eq!(
        parsed.chains.len(),
        crate::registry::Chain::all().count(),
        "network catalog and registry must have the same number of chains"
    );
    let mut ids = std::collections::HashSet::new();
    let catalog = parsed
        .chains
        .iter()
        .zip(crate::registry::Chain::all())
        .map(|(c, chain)| {
            assert!(ids.insert(&c.id), "duplicate network id {}", c.id);
            assert!(
                matches!(c.environment.as_str(), "mainnet" | "testnet"),
                "invalid environment"
            );
            let native = crate::tokens::deployment(&format!("{}:native", c.id))
                .expect("unknown native token deployment");
            assert!(
                native.is_native() && native.chain_id == c.id,
                "native deployment belongs to another network"
            );
            let is_testnet = c.environment == "testnet";
            assert!(
                !is_testnet || native.coingecko_id.is_empty(),
                "testnet token must be unpriced"
            );
            assert!(
                parsed
                    .chains
                    .iter()
                    .any(|n| n.id == c.family && n.environment == "mainnet"),
                "unknown network family"
            );
            let ui = ui_by_id
                .remove(&c.id)
                .unwrap_or_else(|| panic!("missing UI record for network {}", c.id));
            ChainEntry {
                id: c.id.clone(),
                name: c.name.clone(),
                family: c.family.clone(),
                is_testnet,
                native_deployment_id: native.deployment_id.clone(),
                address_prefix_hint: ui.address_prefix_hint,
                gas_token_symbol: native.symbol.clone(),
                search_keywords: ui.search_keywords,
                category: ui.category,
                popular_rank: ui.popular_rank,
                is_evm: chain.is_evm(),
                color: ui.color,
                artwork_name: ui.artwork_name,
                token_standard: c.token_standard.clone(),
                enumerates_holdings: c.enumerates_holdings,
                contract_address_prompt: contract_address_prompt_for(&c.token_standard),
                native_coingecko_id: native.coingecko_id.clone(),
                native_decimals: native.decimals,
                native_asset_display_name: native.name.clone(),
                derivation_path: c
                    .derivation_path
                    .iter()
                    .map(|d| ChainDerivationPathEntry {
                        tag: d.tag.clone(),
                        path: d.path.clone(),
                        is_default: d.is_default,
                    })
                    .collect(),
            }
        })
        .collect();
    assert!(ui_by_id.is_empty(), "UI records reference unknown networks");
    catalog
}

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
                native_deployment_id: chain.native_deployment_id.clone(),
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

/// The catalog's default derivation path template for a chain id.
pub(crate) fn default_derivation_path_template(chain_id: &str) -> Option<&'static str> {
    chain_by_str_id(chain_id).and_then(default_template_of)
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

#[cfg(test)]
mod explicit_network_catalog {
    use super::*;
    use crate::registry::Chain;

    fn entry(id: &str) -> &'static ChainEntry {
        CATALOG.iter().find(|c| c.id == id).expect("a catalog row")
    }

    #[test]
    fn presentation_joins_by_id_independent_of_row_order() {
        let reversed = CHAIN_UI_TOML
            .split("[[chains]]")
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|row| format!("[[chains]]{row}"))
            .collect::<String>();
        let actual = load_catalog(CHAINS_TOML, &reversed);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(&*CATALOG).unwrap()
        );
    }

    #[test]
    fn display_categories_cannot_change_evm_membership() {
        let changed = CHAIN_UI_TOML
            .replace("category = \"evm-l1\"", "category = \"other\"")
            .replace("category = \"evm-l2\"", "category = \"other\"")
            .replace("category = \"bitcoin-family\"", "category = \"evm-l1\"");
        let catalog = load_catalog(CHAINS_TOML, &changed);
        for (actual, expected) in catalog.iter().zip(CATALOG.iter()) {
            assert_eq!(actual.is_evm, expected.is_evm, "{}", actual.id);
        }
        assert!(catalog
            .iter()
            .any(|c| c.is_evm && c.category == ChainCategory::Other));
        assert!(catalog
            .iter()
            .any(|c| !c.is_evm && c.category == ChainCategory::EvmL1));
    }

    #[test]
    #[should_panic(expected = "duplicate UI chain_id bitcoin")]
    fn duplicate_presentation_references_are_rejected() {
        let first = CHAIN_UI_TOML.split("[[chains]]").nth(1).unwrap();
        load_catalog(CHAINS_TOML, &format!("{CHAIN_UI_TOML}\n[[chains]]{first}"));
    }

    #[test]
    #[should_panic(expected = "missing UI record for network bitcoin")]
    fn missing_presentation_is_rejected() {
        let without_bitcoin = CHAIN_UI_TOML
            .split("[[chains]]")
            .skip(2)
            .map(|row| format!("[[chains]]{row}"))
            .collect::<String>();
        load_catalog(CHAINS_TOML, &without_bitcoin);
    }

    #[test]
    #[should_panic(expected = "UI records reference unknown networks")]
    fn unknown_presentation_references_are_rejected() {
        let unknown = CHAIN_UI_TOML
            .split("[[chains]]")
            .nth(1)
            .unwrap()
            .replace("chain_id = \"bitcoin\"", "chain_id = \"unknown-network\"");
        load_catalog(
            CHAINS_TOML,
            &format!("{CHAIN_UI_TOML}\n[[chains]]{unknown}"),
        );
    }

    #[test]
    fn fields_in_the_wrong_catalog_are_rejected() {
        let wrong_core = CHAINS_TOML.replacen("[[chains]]", "[[chains]]\ncolor = \"orange\"", 1);
        assert!(toml::from_str::<TomlFile>(&wrong_core).is_err());
        let configured_evm = CHAINS_TOML.replacen("[[chains]]", "[[chains]]\nis_evm = true", 1);
        assert!(toml::from_str::<TomlFile>(&configured_evm).is_err());
        let wrong_ui = CHAIN_UI_TOML.replacen("[[chains]]", "[[chains]]\nis_evm = false", 1);
        assert!(toml::from_str::<TomlUiFile>(&wrong_ui).is_err());
    }

    #[test]
    fn mainnets_and_testnets_are_explicit_peers() {
        let parsed: TomlFile = toml::from_str(CHAINS_TOML).unwrap();
        assert_eq!(CATALOG.len(), parsed.chains.len());
        for n in &parsed.chains {
            let chain = Chain::from_str_id(&n.id).unwrap();
            assert_eq!(chain.is_testnet(), n.environment == "testnet");
            assert_eq!(chain.mainnet_counterpart().str_id(), n.family);
        }
    }

    /// A network inherits its chain's technical facts.
    ///
    /// They were columns on every testnet row — eight of them restated
    /// verbatim, which is eight chances for one to drift.
    #[test]
    fn networks_share_protocol_facts_but_have_distinct_identity() {
        let (main, net) = (entry("ethereum"), entry("ethereum-sepolia"));
        for (field, a, b) in [
            ("artwork_name", &main.artwork_name, &net.artwork_name),
            (
                "native_asset_display_name",
                &main.native_asset_display_name,
                &net.native_asset_display_name,
            ),
        ] {
            assert_eq!(a, b, "{field} did not carry through to the network");
        }
        assert_eq!(
            main.category, net.category,
            "category did not carry through to the network"
        );
        assert_eq!(
            main.color, net.color,
            "color did not carry through to the network"
        );
        assert_eq!(main.native_decimals, net.native_decimals);
        assert_eq!(main.is_evm, net.is_evm);
        assert_ne!(main.name, net.name);
        // EVM accounts use the same path across networks, while Bitcoin's
        // test networks use coin type 1. Neither rule is inferred from names.
        assert_eq!(main.derivation_path[0].path, net.derivation_path[0].path);
        assert_ne!(
            entry("bitcoin").derivation_path[0].path,
            entry("bitcoin-testnet-4").derivation_path[0].path
        );
    }

    #[test]
    fn testnet_native_symbols_are_visibly_distinct_from_mainnet() {
        for chain in Chain::all().filter(|chain| chain.is_testnet()) {
            let native = chain.native_holding_template();
            assert_eq!(
                native.symbol,
                format!("t{}", chain.mainnet_counterpart().coin_symbol())
            );
            assert_eq!(chain.coin_symbol(), native.symbol);
            assert!(native.coingecko_id.is_empty());
        }
        assert_eq!(Chain::Bitcoin.coin_symbol(), "BTC");
        assert_eq!(Chain::Ethereum.coin_symbol(), "ETH");
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
            assert!(
                e.token_standard.is_empty(),
                "{} claims to host tokens",
                e.id
            );
            assert!(e.contract_address_prompt.is_empty());
        }
    }

    /// The setup picker's short list is a rank per chain, so it cannot hold a
    /// duplicate position, a gap, or a testnet.
    #[test]
    fn the_popular_short_list_is_a_ranking() {
        let mut ranks: Vec<u8> = Vec::new();
        for chain in Chain::all() {
            let e = entry(chain.str_id());
            let Some(rank) = e.popular_rank else { continue };
            assert!(!e.is_testnet, "{} is a testnet on the short list", e.id);
            ranks.push(rank);
        }
        ranks.sort_unstable();
        let expected: Vec<u8> = (1..=ranks.len() as u8).collect();
        assert_eq!(ranks, expected, "the short list is not 1..=n without gaps");
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
                assert!(
                    !documented,
                    "{} is a network and has a wiki row",
                    chain.str_id()
                );
            } else {
                assert!(documented, "{} has no wiki row", chain.str_id());
            }
        }
        assert_eq!(WIKI.len(), Chain::all().filter(|c| !c.is_testnet()).count());
    }

    /// The wiki joins to the catalog rather than restating it.
    #[test]
    fn a_wiki_row_takes_its_name_from_the_catalog() {
        let dot = WIKI
            .iter()
            .find(|w| w.id == "polkadot")
            .expect("a wiki row");
        let catalog = entry("polkadot");
        assert_eq!(dot.name, catalog.name);
        assert_eq!(dot.native_deployment_id, catalog.native_deployment_id);
        assert_eq!(dot.derivation_path.len(), catalog.derivation_path.len());
        assert!(!dot.family.is_empty());
    }

    /// Contract prompts follow token standards; EVM membership comes from the registry.
    #[test]
    fn the_derived_columns_agree_with_what_they_derive_from() {
        for e in CATALOG.iter() {
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
        for chain in Chain::all() {
            assert_eq!(
                chain.is_evm(),
                entry(chain.str_id()).is_evm,
                "{} disagrees about being EVM",
                chain.str_id()
            );
        }
    }
}
