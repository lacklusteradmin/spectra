//! Concrete network registry embedded from `chains.toml`.
//! Mainnets and testnets are equal records. Native token metadata in the public
//! projection is joined from `tokens.toml`; it is never stored as a network fact.
//! `chain-ui.toml` supplies presentation by network ID; `chain-wiki.toml` holds prose.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static CHAINS_TOML: &str = include_str!("../data/chains.toml");
static CHAIN_UI_TOML: &str = include_str!("../data/chain-ui.toml");
static CHAIN_WIKI_TOML: &str = include_str!("../data/chain-wiki.toml");

/// A filter the chain picker offers. A display classification only: it never
/// decides a protocol capability.
///
/// Declaration order is the order the picker shows the filters in.
/// `Layer1`, `Evm` and `Testnet` are derived from the registry; the rest are
/// written in `chain-ui.toml`, and parsing them there means a misspelt tag
/// fails when the file loads rather than dropping the chain from a filter.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "kebab-case")]
pub enum ChainTag {
    #[serde(rename = "layer-1")]
    Layer1,
    #[serde(rename = "layer-2")]
    Layer2,
    Evm,
    Utxo,
    Eutxo,
    Move,
    Substrate,
    Pow,
    Privacy,
    Payments,
    Testnet,
}

impl ChainTag {
    pub const ALL: [ChainTag; 11] = [
        ChainTag::Layer1,
        ChainTag::Layer2,
        ChainTag::Evm,
        ChainTag::Utxo,
        ChainTag::Eutxo,
        ChainTag::Move,
        ChainTag::Substrate,
        ChainTag::Pow,
        ChainTag::Privacy,
        ChainTag::Payments,
        ChainTag::Testnet,
    ];

    /// Whether the catalog computes this tag rather than reading it.
    fn is_derived(self) -> bool {
        matches!(self, ChainTag::Layer1 | ChainTag::Evm | ChainTag::Testnet)
    }

    /// The spelling `chain-ui.toml` and the CLI use.
    pub fn as_str(self) -> &'static str {
        match self {
            ChainTag::Layer1 => "layer-1",
            ChainTag::Layer2 => "layer-2",
            ChainTag::Evm => "evm",
            ChainTag::Utxo => "utxo",
            ChainTag::Eutxo => "eutxo",
            ChainTag::Move => "move",
            ChainTag::Substrate => "substrate",
            ChainTag::Pow => "pow",
            ChainTag::Privacy => "privacy",
            ChainTag::Payments => "payments",
            ChainTag::Testnet => "testnet",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tag| tag.as_str() == value)
    }
}

/// A catalog entry's brand colour, from a closed palette. Checked when the
/// file is parsed, so a misspelt name fails there instead of drawing in the
/// wrong colour.
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
pub(crate) struct TomlChain {
    id: String,
    name: String,
    family: String,
    pub(crate) environment: String,
    pub(crate) token_standard: String,
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
    /// The chain's place in the picker. Written on mainnets only.
    #[serde(default)]
    popular_rank: Option<u16>,
    /// The authored tags. Written on mainnets only.
    #[serde(default)]
    tags: Vec<ChainTag>,
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
    /// Where this chain sits in the picker's popular order, 1 first. Every
    /// mainnet has its own; a testnet shares its mainnet's.
    ///
    /// The picker held a short list as an eight-id array in Swift, which is a
    /// per-chain fact in a caller-owned list; every chain is ranked so the
    /// whole list has one order rather than eight ranked rows and the rest.
    pub popular_rank: u16,
    /// The picker filters this chain appears under, in [`ChainTag::ALL`]
    /// order. A testnet carries its mainnet's tags and `Testnet`.
    pub tags: Vec<ChainTag>,
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
/// paths can read it, and so the prose does not cross the FFI with every
/// `list_all_chains()` call.
///
/// There is a row per chain and none per network: the table is the filter.
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

/// `chains.toml` as written, parsed once.
///
/// Read apart from the catalog: building the catalog joins each chain's native
/// token, and the token catalog checks its deployments against these rows, so
/// neither can wait for the other's catalog.
static DECLARED: LazyLock<TomlFile> = LazyLock::new(|| {
    toml::from_str(CHAINS_TOML)
        .expect("chains.toml is embedded at compile time and must be valid TOML")
});

/// A network's row as `chains.toml` declares it.
pub(crate) fn declared(chain: crate::registry::Chain) -> &'static TomlChain {
    DECLARED
        .chains
        .get(chain as usize)
        .expect("enum declaration order is the catalog's order")
}

pub(crate) fn catalog_id(index: usize) -> &'static str {
    &DECLARED
        .chains
        .get(index)
        .expect("enum declaration order is the catalog's order")
        .id
}

static CATALOG: LazyLock<Vec<ChainEntry>> =
    LazyLock::new(|| load_catalog(&DECLARED, CHAIN_UI_TOML));

fn load_catalog(parsed: &TomlFile, presentation: &str) -> Vec<ChainEntry> {
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
    // A testnet's rank and tags are its mainnet's. Read before any row is
    // consumed, so a testnet row can come before its mainnet's.
    let placement: std::collections::HashMap<&str, (u16, Vec<ChainTag>)> = parsed
        .chains
        .iter()
        .filter(|c| c.environment == "mainnet")
        .filter_map(|c| ui_by_id.get(&c.id).map(|ui| (c.id.as_str(), ui)))
        .map(|(id, ui)| {
            let rank = ui
                .popular_rank
                .unwrap_or_else(|| panic!("mainnet {id} has no popular_rank"));
            let mut tags = ui.tags.clone();
            assert!(
                tags.iter().all(|tag| !tag.is_derived()),
                "{id} writes a derived tag"
            );
            tags.sort_unstable();
            tags.dedup();
            assert_eq!(tags.len(), ui.tags.len(), "{id} repeats a tag");
            (id, (rank, tags))
        })
        .collect();

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
                native.is_native() && native.chain_id == chain,
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
            assert!(
                !is_testnet || (ui.popular_rank.is_none() && ui.tags.is_empty()),
                "testnet {} restates its mainnet's rank or tags",
                c.id
            );
            let (popular_rank, mut tags) = placement
                .get(c.family.as_str())
                .cloned()
                .unwrap_or_else(|| panic!("missing UI record for network {}", c.family));
            if !tags.contains(&ChainTag::Layer2) {
                tags.push(ChainTag::Layer1);
            }
            if chain.is_evm() {
                tags.push(ChainTag::Evm);
            }
            if is_testnet {
                tags.push(ChainTag::Testnet);
            }
            tags.sort_unstable();
            ChainEntry {
                id: c.id.clone(),
                name: c.name.clone(),
                family: c.family.clone(),
                is_testnet,
                native_deployment_id: native.deployment_id.clone(),
                address_prefix_hint: ui.address_prefix_hint,
                gas_token_symbol: native.symbol.clone(),
                search_keywords: ui.search_keywords,
                popular_rank,
                tags,
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

/// Every picker filter, in the order the picker shows them.
#[uniffi::export]
pub fn list_chain_tags() -> Vec<ChainTag> {
    ChainTag::ALL.to_vec()
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
pub(crate) fn default_derivation_path_template(
    chain: crate::registry::Chain,
) -> Option<&'static str> {
    default_template_of(chain.entry())
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
        let actual = load_catalog(&DECLARED, &reversed);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(&*CATALOG).unwrap()
        );
    }

    /// `evm` is the registry's fact; the presentation file cannot claim it.
    #[test]
    #[should_panic(expected = "bitcoin writes a derived tag")]
    fn derived_tags_cannot_be_written() {
        let claimed = CHAIN_UI_TOML.replacen(
            "tags = [\"utxo\", \"pow\"]",
            "tags = [\"utxo\", \"pow\", \"evm\"]",
            1,
        );
        load_catalog(&DECLARED, &claimed);
    }

    #[test]
    #[should_panic(expected = "testnet bitcoin-testnet restates its mainnet's rank or tags")]
    fn a_testnet_cannot_restate_its_mainnets_placement() {
        let restated = CHAIN_UI_TOML.replacen(
            "chain_id = \"bitcoin-testnet\"\n",
            "chain_id = \"bitcoin-testnet\"\npopular_rank = 1\n",
            1,
        );
        load_catalog(&DECLARED, &restated);
    }

    #[test]
    #[should_panic(expected = "mainnet bitcoin has no popular_rank")]
    fn every_mainnet_is_ranked() {
        load_catalog(
            &DECLARED,
            &CHAIN_UI_TOML.replacen("popular_rank = 1\n", "", 1),
        );
    }

    #[test]
    fn tag_spellings_agree() {
        for tag in ChainTag::ALL {
            assert_eq!(
                serde_json::to_value(tag).unwrap(),
                serde_json::Value::from(tag.as_str())
            );
            assert_eq!(ChainTag::parse(tag.as_str()), Some(tag));
        }
        let mut sorted = ChainTag::ALL;
        sorted.sort_unstable();
        assert_eq!(sorted, ChainTag::ALL, "ALL is not in declaration order");
    }

    /// The derived tags follow the registry, and the authored ones that the
    /// registry also models agree with it.
    #[test]
    fn tags_agree_with_the_registry() {
        use crate::fetch::transactions::TransactionMergeStrategy as Merge;
        for chain in Chain::all() {
            let e = entry(chain.str_id());
            let has = |tag| e.tags.contains(&tag);
            assert_eq!(has(ChainTag::Evm), chain.is_evm(), "{}", e.id);
            assert_eq!(has(ChainTag::Testnet), chain.is_testnet(), "{}", e.id);
            assert_ne!(has(ChainTag::Layer1), has(ChainTag::Layer2), "{}", e.id);
            assert_eq!(
                has(ChainTag::Utxo),
                matches!(
                    chain.transaction_merge_strategy(),
                    Merge::StandardUtxo | Merge::Dogecoin
                ),
                "{} disagrees about being UTXO",
                e.id
            );
            assert_eq!(
                has(ChainTag::Substrate),
                chain.substrate_balance_bytes().is_some(),
                "{} disagrees about being Substrate",
                e.id
            );
            assert!(
                !has(ChainTag::Layer2) || chain.is_evm(),
                "{} is a non-EVM layer 2",
                e.id
            );
            assert!(e.tags.windows(2).all(|w| w[0] < w[1]), "{}", e.id);
        }
    }

    #[test]
    #[should_panic(expected = "duplicate UI chain_id bitcoin")]
    fn duplicate_presentation_references_are_rejected() {
        let first = CHAIN_UI_TOML.split("[[chains]]").nth(1).unwrap();
        load_catalog(&DECLARED, &format!("{CHAIN_UI_TOML}\n[[chains]]{first}"));
    }

    #[test]
    #[should_panic(expected = "missing UI record for network bitcoin")]
    fn missing_presentation_is_rejected() {
        let without_bitcoin = CHAIN_UI_TOML
            .split("[[chains]]")
            .skip(2)
            .map(|row| format!("[[chains]]{row}"))
            .collect::<String>();
        load_catalog(&DECLARED, &without_bitcoin);
    }

    #[test]
    #[should_panic(expected = "UI records reference unknown networks")]
    fn unknown_presentation_references_are_rejected() {
        let unknown = CHAIN_UI_TOML
            .split("[[chains]]")
            .nth(1)
            .unwrap()
            .replace("chain_id = \"bitcoin\"", "chain_id = \"unknown-network\"");
        load_catalog(&DECLARED, &format!("{CHAIN_UI_TOML}\n[[chains]]{unknown}"));
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
        let parsed = &*DECLARED;
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
            main.popular_rank, net.popular_rank,
            "popular_rank did not carry through to the network"
        );
        assert_eq!(
            [&main.tags[..], &[ChainTag::Testnet]].concat(),
            net.tags,
            "tags did not carry through to the network"
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

    /// A testnet asset has no price, structurally.
    #[test]
    fn a_network_never_inherits_a_price() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            let e = entry(chain.str_id());
            assert!(
                e.native_coingecko_id.is_empty(),
                "{} carries a price id",
                e.id
            );
        }
    }

    /// The picker's popular order ranks every mainnet once, 1..=n without gaps.
    #[test]
    fn every_mainnet_has_its_own_rank() {
        let mut ranks: Vec<u16> = Chain::mainnets()
            .map(|chain| entry(chain.str_id()).popular_rank)
            .collect();
        ranks.sort_unstable();
        let expected: Vec<u16> = (1..=ranks.len() as u16).collect();
        assert_eq!(ranks, expected, "the ranks are not 1..=n without gaps");
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
