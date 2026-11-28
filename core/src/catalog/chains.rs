//! Concrete network registry embedded from `chains.toml`.
//! Mainnets and testnets are equal records. Native token metadata in the public
//! projection is joined from `tokens.toml`; it is never stored as a network fact.
//! Network rules, presentation and what the chain is for a reader share each
//! catalog row; `staking.toml` holds the staking prose.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static CHAINS_TOML: &str = include_str!("../../data/chains.toml");
static STAKING_TOML: &str = include_str!("../../data/staking.toml");

/// A classification for the chain picker. Presentation only:
/// it never decides a protocol capability.
///
/// Declaration order is the order the picker shows the filters in.
/// `Layer1`, `Evm` and `Testnet` are derived from the registry; the rest are
/// written in `chains.toml`, and parsing them there means a misspelt tag
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

    /// The spelling `chains.toml` and the CLI use.
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
    /// The mainnet a test network tests. Written on testnets only.
    #[serde(default)]
    pub(crate) mainnet: Option<String>,
    pub(crate) token_standards: Vec<crate::registry::TokenStandard>,
    derivation_path: Vec<TomlDerivationPathEntry>,
    search_keywords: Vec<String>,
    /// The chain's place in the picker. Written on mainnets only.
    #[serde(default)]
    popular_rank: Option<u16>,
    /// The authored tags. Written on mainnets only.
    #[serde(default)]
    tags: Vec<ChainTag>,
    color: CatalogColor,
    artwork_name: String,
    address_prefix_hint: String,
    /// A test network's faucet page, where its coins are free. Testnets only.
    #[serde(default)]
    pub(crate) faucet: Option<String>,
    /// What the chain is, for a reader. Written on mainnets only.
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    consensus: Option<String>,
    #[serde(default)]
    state_model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TomlDerivationPathEntry {
    tag: DerivationProfile,
    path: String,
    #[serde(default)]
    is_default: bool,
}

#[derive(Debug, Deserialize)]
struct TomlStakingFile {
    chains: Vec<TomlStakingChain>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlStakingChain {
    chain: String,
    short_mechanic: String,
    unbonding_period: String,
    minimum_stake: String,
    explanation: String,
}

/// One token protocol a network supports, with what a token of it is
/// identified by and the places it has when the protocol fixes them.
///
/// A token is added under one of these, named rather than guessed from the
/// identifier: on BNB Smart Chain an ERC-20 and a BEP-20 contract look alike,
/// and Tron's TRC-10 ID and TRC-20 address are asked for differently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TokenStandardEntry {
    pub standard: crate::registry::TokenStandard,
    pub identifier_prompt: String,
    /// The places every token of this protocol has: nothing to choose.
    pub fixed_decimals: Option<u32>,
}

impl TokenStandardEntry {
    pub fn new(standard: crate::registry::TokenStandard) -> Self {
        Self {
            standard,
            identifier_prompt: standard.identifier_prompt().to_string(),
            fixed_decimals: standard.fixed_decimals(),
        }
    }
}

// ── Public serialized shape — exposed to Swift via UniFFI

/// A named way a phrase wallet on a chain derives its account: the script
/// type on the Bitcoin family, an older path some wallets still use, or
/// Hedera's key type.
/// Each chain lists the ones it offers in `chains.toml`; the account index
/// is the `{account}` segment of the profile's template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum DerivationProfile {
    /// The chain's one ordinary BIP-44 (or SLIP-10) account.
    Standard,
    /// BIP-44 P2PKH on the Bitcoin family; an older path elsewhere.
    Legacy,
    /// BIP-49 P2SH-wrapped SegWit.
    NestedSegWit,
    /// BIP-84 native SegWit.
    NativeSegWit,
    /// BIP-86 Taproot.
    Taproot,
    /// Hedera's ECDSA secp256k1 key along its own coin type, as its SDK's
    /// `toStandardECDSAsecp256k1PrivateKey` derives it.
    Ecdsa,
    /// An ECDSA secp256k1 key along Ethereum's path, as MetaMask derives it:
    /// a Hedera account keyed from a phrase an EVM wallet restores.
    Evm,
}

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainDerivationPathEntry {
    pub profile: DerivationProfile,
    /// The path with `{account}` where the account index goes.
    pub path: String,
    pub is_default: bool,
}

/// One network: its identity, the capabilities a front end branches on, and
/// its presentation. The whole catalog crosses the FFI once, from
/// [`list_all_chains`], and a front end reads it locally after that.
#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct ChainEntry {
    pub chain: crate::registry::Chain,
    /// The catalog's `id` — what endpoint tables and stored rows key on.
    pub id: String,
    /// The catalog's `name` — the one spelling of this chain.
    pub name: String,
    /// The mainnet this network tests, or itself.
    pub mainnet: crate::registry::Chain,
    pub is_testnet: bool,
    pub is_evm: bool,
    /// A test network's faucet page, where its coins are free; `None` on
    /// mainnets and on test networks without a working one.
    pub faucet_url: Option<String>,
    /// A wallet on this chain is an account of many addresses
    /// (`Chain::uses_account_utxo`), which a rescan can walk.
    pub uses_account_utxo: bool,
    /// The prefixes a watched account public key starts with here
    /// (`Chain::account_key_prefixes`); empty where none is taken.
    pub account_key_prefixes: Vec<String>,
    /// The send screen has a network card to show for this chain — a fee, a
    /// preview, or both. False only where core routes no send at all.
    pub has_send_preview: bool,
    /// The chain can hold tracked tokens.
    pub hosts_tokens: bool,
    pub native_deployment_id: String,
    /// A terse example of what an address on this network looks like, which
    /// a front end shows wherever an address is typed.
    pub address_prefix_hint: String,
    /// The native coin's symbol.
    pub coin_symbol: String,
    /// The symbol of the asset this network's fees are paid in: the coin's
    /// own, or VTHO on VeChainThor ([`crate::registry::Chain::fee_holding_template`]).
    pub fee_symbol: String,
    pub search_keywords: Vec<String>,
    /// Where this chain sits in the picker's popular order, 1 first. Every
    /// mainnet has its own; a testnet shares its mainnet's.
    ///
    /// The picker held a short list as an eight-id array in Swift, which is a
    /// per-chain fact in a caller-owned list; every chain is ranked so the
    /// whole list has one order rather than eight ranked rows and the rest.
    pub popular_rank: u16,
    /// The picker's tags, in [`ChainTag::ALL`] order.
    /// A testnet carries its mainnet's tags and `Testnet`.
    pub tags: Vec<ChainTag>,
    pub color: CatalogColor,
    pub artwork_name: String,
    /// Every protocol the network supports, in the catalog's order; each
    /// deployment owns its actual standard.
    pub token_standards: Vec<TokenStandardEntry>,
    pub native_coingecko_id: String,
    pub native_decimals: u32,
    pub native_asset_display_name: String,
    pub derivation_path: Vec<ChainDerivationPathEntry>,
    /// What the chain is used for, in a sentence or two. This and the three
    /// below describe the chain for a reader and decide nothing; a testnet
    /// shows its mainnet's.
    pub description: String,
    pub family: String,
    pub consensus: String,
    pub state_model: String,
}

/// What staking on a chain means, for a reader — one row per chain that
/// `Chain::supports_staking`, in catalog order. Protocol information and
/// validator directories; no yield quote or transaction execution.
#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct StakingChainEntry {
    pub chain: crate::registry::Chain,
    pub short_mechanic: String,
    pub unbonding_period: String,
    pub minimum_stake: String,
    pub explanation: String,
}

// ── Static catalog

/// `chains.toml` as written, parsed once.
///
/// Read apart from the catalog: building the catalog joins each chain's native
/// token, and the token catalog checks its deployments against these rows, so
/// neither can wait for the other's catalog.
static DECLARED: LazyLock<TomlFile> = LazyLock::new(|| {
    let parsed: TomlFile = toml::from_str(CHAINS_TOML)
        .expect("chains.toml is embedded at compile time and must be valid TOML");
    for chain in &parsed.chains {
        let mut standards = std::collections::HashSet::new();
        for standard in &chain.token_standards {
            assert!(
                standards.insert(standard),
                "duplicate token protocol on {}",
                chain.id
            );
        }
    }
    parsed
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

static CATALOG: LazyLock<Vec<ChainEntry>> = LazyLock::new(|| load_catalog(&DECLARED));

fn load_catalog(parsed: &TomlFile) -> Vec<ChainEntry> {
    // Chain discriminants already index this catalog. Use that same ordering
    // here; from_str_id/entry would recursively initialize CATALOG.
    assert_eq!(
        parsed.chains.len(),
        crate::registry::Chain::all().count(),
        "network catalog and registry must have the same number of chains"
    );
    let mut ids = std::collections::HashSet::new();
    for c in &parsed.chains {
        assert!(ids.insert(&c.id), "duplicate network id {}", c.id);
        if let Some(mainnet) = &c.mainnet {
            assert!(
                parsed
                    .chains
                    .iter()
                    .any(|n| &n.id == mainnet && n.mainnet.is_none()),
                "{} tests {mainnet}, which is not a mainnet",
                c.id
            );
        }
        assert!(
            c.faucet.is_none()
                || c.mainnet.is_some()
                    && c.faucet
                        .as_deref()
                        .is_some_and(|url| url.starts_with("https://")),
            "{} faucet must be an https page on a testnet",
            c.id
        );
        // A profile is one template with one account segment, and a chain
        // that derives along a path has exactly one default.
        let mut profiles = std::collections::HashSet::new();
        for entry in &c.derivation_path {
            assert!(
                profiles.insert(entry.tag),
                "{} lists {:?} twice",
                c.id,
                entry.tag
            );
            assert_eq!(
                entry.path.matches("{account}").count(),
                1,
                "{} {:?} has no single account segment",
                c.id,
                entry.tag
            );
        }
        assert!(
            c.derivation_path.is_empty()
                || c.derivation_path.iter().filter(|e| e.is_default).count() == 1,
            "{} needs exactly one default derivation profile",
            c.id
        );
    }
    // A testnet's rank, tags and description are its mainnet's. Resolve them
    // independently of the testnet's position in the catalog.
    let placement: std::collections::HashMap<&str, Inherited> = parsed
        .chains
        .iter()
        .filter(|c| c.mainnet.is_none())
        .map(|c| {
            let id = c.id.as_str();
            let rank = c
                .popular_rank
                .unwrap_or_else(|| panic!("mainnet {id} has no popular_rank"));
            let mut tags = c.tags.clone();
            assert!(
                tags.iter().all(|tag| !tag.is_derived()),
                "{id} writes a derived tag"
            );
            tags.sort_unstable();
            tags.dedup();
            assert_eq!(tags.len(), c.tags.len(), "{id} repeats a tag");
            let prose = |field: &Option<String>, name: &str| {
                field
                    .clone()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| panic!("mainnet {id} has no {name}"))
            };
            let inherited = Inherited {
                popular_rank: rank,
                tags,
                description: prose(&c.description, "description"),
                family: prose(&c.family, "family"),
                consensus: prose(&c.consensus, "consensus"),
                state_model: prose(&c.state_model, "state_model"),
            };
            (id, inherited)
        })
        .collect();

    parsed
        .chains
        .iter()
        .zip(crate::registry::Chain::all())
        .map(|(c, chain)| {
            let native = crate::catalog::tokens::deployment(
                &crate::catalog::tokens::deployment_id(chain, None, ""),
            )
            .expect("unknown native token deployment");
            assert!(
                native.is_native() && native.chain_id == chain,
                "native deployment belongs to another network"
            );
            let is_testnet = c.mainnet.is_some();
            assert!(
                !is_testnet || native.coingecko_id.is_empty(),
                "testnet token must be unpriced"
            );
            assert!(
                !is_testnet || (c.popular_rank.is_none() && c.tags.is_empty()),
                "testnet {} restates its mainnet's rank or tags",
                c.id
            );
            assert!(
                !is_testnet
                    || [&c.description, &c.family, &c.consensus, &c.state_model]
                        .iter()
                        .all(|field| field.is_none()),
                "testnet {} restates its mainnet's description",
                c.id
            );
            let Inherited {
                popular_rank,
                mut tags,
                description,
                family,
                consensus,
                state_model,
            } = placement
                .get(c.mainnet.as_deref().unwrap_or(&c.id))
                .cloned()
                .expect("every network has a mainnet placement");
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
                chain,
                id: c.id.clone(),
                name: c.name.clone(),
                mainnet: chain.mainnet_counterpart(),
                is_testnet,
                is_evm: chain.is_evm(),
                faucet_url: c.faucet.clone(),
                uses_account_utxo: chain.uses_account_utxo(),
                account_key_prefixes: chain
                    .account_key_prefixes()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                has_send_preview: chain.has_send_preview(),
                hosts_tokens: chain.hosts_tokens(),
                native_deployment_id: native.deployment_id.clone(),
                address_prefix_hint: c.address_prefix_hint.clone(),
                coin_symbol: native.symbol.clone(),
                fee_symbol: match chain.fee_token_contract() {
                    None => native.symbol.clone(),
                    Some(contract) => crate::catalog::tokens::catalog()
                        .iter()
                        .find(|d| d.chain_id == chain && !d.is_native() && d.contract == contract)
                        .expect("the catalog lists the token a network's fees are paid in")
                        .symbol
                        .clone(),
                },
                search_keywords: c.search_keywords.clone(),
                popular_rank,
                tags,
                color: c.color,
                artwork_name: c.artwork_name.clone(),
                token_standards: c
                    .token_standards
                    .iter()
                    .map(|standard| TokenStandardEntry::new(*standard))
                    .collect(),
                native_coingecko_id: native.coingecko_id.clone(),
                native_decimals: native.decimals,
                native_asset_display_name: native.name.clone(),
                derivation_path: c
                    .derivation_path
                    .iter()
                    .map(|d| ChainDerivationPathEntry {
                        profile: d.tag,
                        path: d.path.clone(),
                        is_default: d.is_default,
                    })
                    .collect(),
                description,
                family,
                consensus,
                state_model,
            }
        })
        .collect()
}

/// What a testnet takes from its mainnet's row.
#[derive(Clone)]
struct Inherited {
    popular_rank: u16,
    tags: Vec<ChainTag>,
    description: String,
    family: String,
    consensus: String,
    state_model: String,
}

static STAKING: LazyLock<Vec<StakingChainEntry>> = LazyLock::new(|| {
    let parsed: TomlStakingFile = toml::from_str(STAKING_TOML)
        .expect("staking.toml is embedded at compile time and must be valid TOML");
    let mut rows: Vec<StakingChainEntry> = parsed
        .chains
        .into_iter()
        .map(|row| {
            let chain = crate::registry::Chain::from_str_id(&row.chain)
                .unwrap_or_else(|| panic!("staking.toml: unknown chain {}", row.chain));
            // A directory description must name a chain whose validators
            // the tab can actually query.
            assert!(
                chain.supports_staking(),
                "staking.toml: {} does not support staking",
                row.chain
            );
            StakingChainEntry {
                chain,
                short_mechanic: row.short_mechanic,
                unbonding_period: row.unbonding_period,
                minimum_stake: row.minimum_stake,
                explanation: row.explanation,
            }
        })
        .collect();
    rows.sort_by_key(|row| row.chain);
    for chain in crate::registry::Chain::all() {
        if chain.supports_staking() {
            assert!(
                rows.iter().filter(|row| row.chain == chain).count() == 1,
                "staking.toml: {} needs exactly one row",
                chain.str_id()
            );
        }
    }
    rows
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

/// Whether `query` names `chain`, ignoring case: its name, its coin's symbol,
/// one of its search keywords or one of its tags. A front end adds what it
/// calls the chain in the reader's language.
#[uniffi::export]
pub fn chain_matches_search(chain: crate::registry::Chain, query: String) -> bool {
    let query = query.trim().to_lowercase();
    let entry = chain.entry();
    query.is_empty()
        || [chain.chain_display_name(), chain.coin_symbol()]
            .into_iter()
            .chain(entry.search_keywords.iter().map(String::as_str))
            .any(|text| text.to_lowercase().contains(&query))
        || entry.tags.iter().any(|tag| tag.as_str().contains(&query))
}

/// What staking means on each chain that supports it, in catalog order.
#[uniffi::export]
pub fn list_staking_chains() -> Vec<StakingChainEntry> {
    STAKING.clone()
}

/// Return a reference to the static catalog slice.
pub(crate) fn catalog() -> &'static [ChainEntry] {
    &CATALOG
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

    fn load_catalog_text(source: &str) -> Vec<ChainEntry> {
        let parsed: TomlFile = toml::from_str(source).unwrap();
        load_catalog(&parsed)
    }

    /// `evm` is the registry's fact; the catalog cannot claim it.
    #[test]
    #[should_panic(expected = "bitcoin writes a derived tag")]
    fn derived_tags_cannot_be_written() {
        let claimed = CHAINS_TOML.replacen(
            "tags = [\"utxo\", \"pow\"]",
            "tags = [\"utxo\", \"pow\", \"evm\"]",
            1,
        );
        load_catalog_text(&claimed);
    }

    #[test]
    #[should_panic(expected = "testnet bitcoin-testnet restates its mainnet's rank or tags")]
    fn a_testnet_cannot_restate_its_mainnets_placement() {
        let restated = CHAINS_TOML.replacen(
            "id = \"bitcoin-testnet\"\n",
            "id = \"bitcoin-testnet\"\npopular_rank = 1\n",
            1,
        );
        load_catalog_text(&restated);
    }

    #[test]
    #[should_panic(expected = "mainnet bitcoin has no popular_rank")]
    fn every_mainnet_is_ranked() {
        load_catalog_text(&CHAINS_TOML.replacen("popular_rank = 1\n", "", 1));
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
        use crate::registry::HistoryFamily as Merge;
        for chain in Chain::all() {
            let e = entry(chain.str_id());
            let has = |tag| e.tags.contains(&tag);
            assert_eq!(has(ChainTag::Evm), chain.is_evm(), "{}", e.id);
            assert_eq!(has(ChainTag::Testnet), chain.is_testnet(), "{}", e.id);
            assert_ne!(has(ChainTag::Layer1), has(ChainTag::Layer2), "{}", e.id);
            assert_eq!(
                has(ChainTag::Utxo),
                matches!(chain.history_family(), Merge::Utxo),
                "{} disagrees about being UTXO",
                e.id
            );
            assert_eq!(
                has(ChainTag::Substrate),
                chain.is_substrate(),
                "{} disagrees about being Substrate",
                e.id
            );
            assert!(e.tags.windows(2).all(|w| w[0] < w[1]), "{}", e.id);
        }
    }

    #[test]
    #[should_panic(expected = "duplicate network id bitcoin")]
    fn duplicate_network_ids_are_rejected() {
        load_catalog_text(&CHAINS_TOML.replacen("id = \"ethereum\"", "id = \"bitcoin\"", 1));
    }

    #[test]
    fn every_network_requires_its_presentation_fields() {
        for field in [
            "search_keywords = [\"Bitcoin\", \"BTC\"]\n",
            "color = \"orange\"\n",
            "artwork_name = \"bitcoin\"\n",
        ] {
            let missing = CHAINS_TOML.replacen(field, "", 1);
            assert!(toml::from_str::<TomlFile>(&missing).is_err(), "{field}");
        }
    }

    #[test]
    #[should_panic(expected = "which is not a mainnet")]
    fn a_testnet_names_a_mainnet() {
        load_catalog_text(&CHAINS_TOML.replacen(
            "mainnet = \"bitcoin\"",
            "mainnet = \"unknown\"",
            1,
        ));
    }

    #[test]
    #[should_panic(expected = "which is not a mainnet")]
    fn a_testnet_does_not_test_a_testnet() {
        load_catalog_text(&CHAINS_TOML.replacen(
            "mainnet = \"bitcoin\"",
            "mainnet = \"bitcoin-signet\"",
            1,
        ));
    }

    /// Every test network's faucet is the one the audit checked live, and
    /// only test networks have one.
    #[test]
    fn faucets_are_the_audited_ones() {
        let audit: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/audits/testnet-faucets-2026-10-07.json"
        ))
        .unwrap();
        let audited: std::collections::HashMap<&str, Option<&str>> = audit["faucets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row["chain"].as_str().unwrap(), row["url"].as_str()))
            .collect();
        for chain in crate::registry::Chain::all() {
            if chain.is_testnet() {
                assert_eq!(
                    chain.faucet_url(),
                    *audited
                        .get(chain.str_id())
                        .unwrap_or_else(|| panic!("{chain} unaudited")),
                    "{chain}"
                );
            } else {
                assert_eq!(chain.faucet_url(), None, "{chain}");
            }
        }
    }

    #[test]
    #[should_panic(expected = "faucet must be an https page on a testnet")]
    fn a_mainnet_has_no_faucet() {
        load_catalog_text(&CHAINS_TOML.replacen(
            "name = \"Bitcoin\"",
            "name = \"Bitcoin\"\nfaucet = \"https://example.com\"",
            1,
        ));
    }

    #[test]
    fn unknown_and_misplaced_fields_are_rejected() {
        let wrong_core = CHAINS_TOML.replacen("[[chains]]", "[[chains]]\ncomment = \"prose\"", 1);
        assert!(toml::from_str::<TomlFile>(&wrong_core).is_err());
        let configured_evm = CHAINS_TOML.replacen("[[chains]]", "[[chains]]\nis_evm = true", 1);
        assert!(toml::from_str::<TomlFile>(&configured_evm).is_err());
    }

    #[test]
    fn mainnets_and_testnets_are_explicit_peers() {
        let parsed = &*DECLARED;
        assert_eq!(CATALOG.len(), parsed.chains.len());
        for n in &parsed.chains {
            let chain = Chain::from_str_id(&n.id).unwrap();
            assert_eq!(chain.is_testnet(), n.mainnet.is_some());
            assert_eq!(
                chain.mainnet_counterpart().str_id(),
                n.mainnet.as_deref().unwrap_or(&n.id)
            );
        }
    }

    /// A network inherits its chain's technical facts.
    ///
    /// They were columns on every testnet row — eight of them restated
    /// verbatim, which is eight chances for one to drift.
    #[test]
    fn networks_share_protocol_facts_but_have_distinct_identity() {
        let (main, net) = (entry("ethereum"), entry("ethereum-sepolia"));
        assert_eq!(
            main.artwork_name, net.artwork_name,
            "artwork_name did not carry through to the network"
        );
        assert_eq!(main.native_asset_display_name, "Ethereum");
        assert_eq!(net.native_asset_display_name, "Test Ethereum");
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
    /// Every network says what its addresses look like: a front end shows
    /// core's hint and keeps none of its own.
    #[test]
    fn every_network_has_an_address_hint() {
        for chain in Chain::all() {
            assert!(
                !entry(chain.str_id()).address_prefix_hint.is_empty(),
                "{}",
                chain.str_id()
            );
        }
    }

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

    /// Every mainnet says what it is; a testnet says it through its mainnet.
    #[test]
    fn every_network_has_its_mainnets_description() {
        for chain in Chain::all() {
            let (e, main) = (
                entry(chain.str_id()),
                entry(chain.mainnet_counterpart().str_id()),
            );
            for field in [&e.description, &e.family, &e.consensus, &e.state_model] {
                assert!(!field.is_empty(), "{}", e.id);
            }
            assert_eq!(
                (&e.description, &e.family, &e.consensus, &e.state_model),
                (
                    &main.description,
                    &main.family,
                    &main.consensus,
                    &main.state_model
                ),
                "{}",
                e.id
            );
        }
    }

    #[test]
    #[should_panic(expected = "mainnet bitcoin has no family")]
    fn every_mainnet_is_described() {
        load_catalog_text(&CHAINS_TOML.replacen(
            "family = \"UTXO / base-layer settlement network\"\n",
            "",
            1,
        ));
    }

    #[test]
    #[should_panic(expected = "testnet bitcoin-testnet restates its mainnet's description")]
    fn a_testnet_cannot_restate_its_mainnets_description() {
        load_catalog_text(&CHAINS_TOML.replacen(
            "id = \"bitcoin-testnet\"\n",
            "id = \"bitcoin-testnet\"\nconsensus = \"Proof of Work\"\n",
            1,
        ));
    }

    /// Each network's standards are the registry's, in its order, and every
    /// one names what its tokens are identified by; EVM membership comes from
    /// the registry.
    #[test]
    fn the_derived_columns_agree_with_what_they_derive_from() {
        for chain in Chain::all() {
            let e = entry(chain.str_id());
            let standards: Vec<crate::registry::TokenStandard> =
                e.token_standards.iter().map(|s| s.standard).collect();
            assert_eq!(standards, chain.token_standards(), "{}", e.id);
            for standard in &e.token_standards {
                assert_ne!(
                    standard.identifier_prompt, "Token Identifier",
                    "{} {}",
                    e.id, standard.standard
                );
            }
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

#[cfg(test)]
mod staking_table_tests {
    use super::*;
    use crate::registry::Chain;

    /// Loading the table checks it against the registry: a row for every
    /// staking chain, none for any other, no unknown field.
    #[test]
    fn the_staking_table_covers_exactly_the_staking_chains() {
        let rows = list_staking_chains();
        let staking: Vec<Chain> = Chain::all().filter(|c| c.supports_staking()).collect();
        assert!(!staking.is_empty());
        assert_eq!(rows.iter().map(|r| r.chain).collect::<Vec<_>>(), staking);
        for row in &rows {
            assert!(
                !row.chain.is_testnet(),
                "{} is a testnet",
                row.chain.str_id()
            );
            for field in [
                &row.short_mechanic,
                &row.unbonding_period,
                &row.minimum_stake,
                &row.explanation,
            ] {
                assert!(
                    !field.trim().is_empty(),
                    "{} has a blank field",
                    row.chain.str_id()
                );
            }
        }
    }
}

#[cfg(test)]
mod search_tests {
    use super::chain_matches_search;
    use crate::registry::Chain;

    /// A search names a chain by its name, symbol, keywords or tags.
    #[test]
    fn a_search_reads_name_symbol_keywords_and_tags() {
        assert!(chain_matches_search(Chain::Ethereum, "ether".into()));
        assert!(chain_matches_search(Chain::Ethereum, "ETH".into()));
        assert!(chain_matches_search(Chain::Arbitrum, "layer-2".into()));
        assert!(chain_matches_search(Chain::Bitcoin, "utxo".into()));
        assert!(chain_matches_search(Chain::Bitcoin, " ".into()));
        assert!(!chain_matches_search(Chain::Bitcoin, "solana".into()));
        for chain in Chain::all() {
            for keyword in &chain.entry().search_keywords {
                assert!(
                    chain_matches_search(chain, keyword.to_uppercase()),
                    "{chain} {keyword}"
                );
            }
        }
    }
}
