use crate::store::wallet_domain::CoreSeedDerivationPaths;
use crate::{EndpointApi, EndpointCapability};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const APP_ENDPOINT_DIRECTORY_TOML: &str = include_str!("../data/endpoints.toml");

#[derive(Debug, Clone)]
pub(crate) struct AppCoreCatalog {
    pub(crate) endpoint_records: Vec<AppCoreEndpointRecord>,
    /// Concrete network ID → record indices, preserving endpoint order.
    endpoint_records_by_chain: std::collections::HashMap<crate::registry::Chain, Vec<usize>>,
}

/// The file's shape, kept separate from the record that crosses the FFI —
/// `chains.rs` splits `TomlChain` from `ChainEntry` for the same reason. The
/// file gets to omit anything empty and to carry comments; the record stays a
/// plain struct with every field present.
#[derive(Debug, Deserialize)]
struct TomlEndpointFile {
    endpoints: Vec<TomlEndpoint>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlEndpoint {
    id: String,
    chain_id: crate::registry::Chain,
    api: EndpointApi,
    endpoint: String,
    capabilities: Vec<EndpointCapability>,
}

impl TryFrom<TomlEndpoint> for AppCoreEndpointRecord {
    type Error = String;

    fn try_from(e: TomlEndpoint) -> Result<Self, Self::Error> {
        if e.capabilities.is_empty() {
            return Err(format!(
                "{}: an endpoint must declare what it is used for",
                e.id
            ));
        }
        let supported = crate::endpoint_capability_options(e.chain_id, e.api);
        if let Some(claim) = e.capabilities.iter().find(|c| !supported.contains(c)) {
            return Err(format!(
                "{}: {} has no {} adapter on this network",
                e.id,
                e.api.as_str(),
                claim.as_str()
            ));
        }
        Ok(AppCoreEndpointRecord {
            id: e.id,
            chain_id: e.chain_id,
            api: e.api,
            endpoint: e.endpoint,
            capabilities: e.capabilities,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreEndpointRecord {
    pub id: String,
    pub api: EndpointApi,
    pub chain_id: crate::registry::Chain,
    pub endpoint: String,
    /// What this endpoint is used for: a claim that has to be true of the
    /// endpoint, and one Spectra's adapter for its API can act on.
    pub capabilities: Vec<EndpointCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreGroupedSettingsEntry {
    pub chain_id: crate::registry::Chain,
    pub title: String,
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
pub struct DerivationPathSegment {
    pub value: u32,
    pub is_hardened: bool,
}

static APP_CORE_CATALOG: OnceLock<Result<AppCoreCatalog, String>> = OnceLock::new();

// ── UniFFI exports ────────────────────────────────────────────────────────

/// The derivation path a wallet on `chain` will use: the caller's, normalized,
/// or the chain's catalog default when the caller named none.
#[uniffi::export]
pub fn resolve_derivation_path(
    chain: crate::registry::Chain,
    derivation_path: String,
) -> Result<String, crate::SpectraBridgeError> {
    let default_path = default_path_from_catalog(chain)?;
    Ok(normalize_derivation_path(&derivation_path, &default_path))
}

#[uniffi::export]
pub fn derivation_paths_for_preset(
    preset: crate::store::wallet_domain::CoreSeedDerivationPreset,
) -> Result<CoreSeedDerivationPaths, crate::SpectraBridgeError> {
    Ok(seed_derivation_paths_for_account(preset.account_index())?)
}

/// A chain's endpoint records that declare any of `any_of`, or all of them
/// when `any_of` is empty.
pub fn filtered_endpoint_records_for_chain(
    chain: crate::registry::Chain,
    any_of: &[EndpointCapability],
) -> Result<Vec<AppCoreEndpointRecord>, crate::SpectraBridgeError> {
    Ok(endpoint_records_for_chain(
        endpoint_catalog()?,
        chain,
        any_of,
    ))
}

/// Everything the endpoint catalog holds for one chain.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AppCoreChainEndpoints {
    pub chain_id: crate::registry::Chain,
    /// What the settings screen shows, grouped by network.
    pub grouped_settings: Vec<AppCoreGroupedSettingsEntry>,
}

/// The endpoint catalog, one row per chain, in catalog order.
#[uniffi::export]
pub fn chain_endpoints() -> Result<Vec<AppCoreChainEndpoints>, crate::SpectraBridgeError> {
    let catalog = endpoint_catalog()?;
    Ok(crate::registry::Chain::all()
        .map(|chain| AppCoreChainEndpoints {
            chain_id: chain,
            grouped_settings: grouped_settings_entries(catalog, chain),
        })
        .collect())
}

// ── Internals ─────────────────────────────────────────────────────────────

pub(crate) fn endpoint_catalog() -> Result<&'static AppCoreCatalog, String> {
    match APP_CORE_CATALOG.get_or_init(load_endpoint_catalog) {
        Ok(catalog) => Ok(catalog),
        Err(message) => Err(message.clone()),
    }
}

fn load_endpoint_catalog() -> Result<AppCoreCatalog, String> {
    let endpoint_records = toml::from_str::<TomlEndpointFile>(APP_ENDPOINT_DIRECTORY_TOML)
        .map_err(|e| e.to_string())?
        .endpoints
        .into_iter()
        .map(AppCoreEndpointRecord::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let mut endpoint_records_by_chain: std::collections::HashMap<
        crate::registry::Chain,
        Vec<usize>,
    > = std::collections::HashMap::new();
    for (idx, record) in endpoint_records.iter().enumerate() {
        endpoint_records_by_chain
            .entry(record.chain_id)
            .or_default()
            .push(idx);
    }
    Ok(AppCoreCatalog {
        endpoint_records,
        endpoint_records_by_chain,
    })
}

fn endpoint_records_for_chain(
    catalog: &AppCoreCatalog,
    chain: crate::registry::Chain,
    any_of: &[EndpointCapability],
) -> Vec<AppCoreEndpointRecord> {
    catalog
        .endpoint_records_by_chain
        .get(&chain)
        .into_iter()
        .flatten()
        .map(|&idx| &catalog.endpoint_records[idx])
        .filter(|record| {
            any_of.is_empty() || any_of.iter().any(|c| record.capabilities.contains(c))
        })
        .cloned()
        .collect()
}

fn grouped_settings_entries(
    catalog: &AppCoreCatalog,
    chain: crate::registry::Chain,
) -> Vec<AppCoreGroupedSettingsEntry> {
    crate::registry::Chain::all()
        .filter(|network| {
            *network == chain || (!chain.is_testnet() && network.mainnet_counterpart() == chain)
        })
        .filter_map(|network| {
            let mut endpoints = Vec::new();
            for record in endpoint_records_for_chain(catalog, network, &[]) {
                if !endpoints.contains(&record.endpoint) {
                    endpoints.push(record.endpoint);
                }
            }
            (!endpoints.is_empty()).then(|| AppCoreGroupedSettingsEntry {
                chain_id: network,
                title: network.chain_display_name().to_string(),
                endpoints,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "This chain has no derivation path" is an answer, not a failure.
    #[test]
    fn a_chain_with_no_catalog_path_derives_without_one() {
        use crate::registry::Chain;

        assert!(!Chain::Monero.uses_derivation_path());
        assert_eq!(
            default_path_for_chain(Chain::Monero).expect("an answer"),
            ""
        );

        // Monero is the only mainnet that says it, so a second one appearing
        // is a catalog edit to notice rather than a silent empty path.
        for chain in Chain::all().filter(|c| !c.is_testnet() && *c != Chain::Monero) {
            assert!(
                chain.uses_derivation_path(),
                "{} has no catalog derivation path",
                chain.str_id()
            );
        }
    }

    #[test]
    fn resolves_bitcoin_taproot_path() {
        let default_path =
            default_path_for_chain(crate::registry::Chain::Bitcoin).expect("default path");
        let normalized = normalize_derivation_path("m/86'/0'/2'/0/0", &default_path);
        assert_eq!(normalized, "m/86'/0'/2'/0/0");
    }

    #[test]
    fn renders_catalog_default_paths_for_preset_accounts() {
        use crate::registry::Chain;

        let paths = seed_derivation_paths_for_account(2).expect("paths");
        assert_eq!(paths.path_for(Chain::BitcoinSV), Some("m/44'/236'/2'/0/0"));
        assert_eq!(paths.path_for(Chain::Ethereum), Some("m/44'/60'/2'/0/0"));
        assert_eq!(paths.path_for(Chain::Solana), Some("m/44'/501'/2'/0'"));
    }

    /// Every concrete network with a catalog template gets its own path.
    #[test]
    fn derivation_paths_cover_the_catalog_and_resolve_testnets() {
        use crate::registry::Chain;

        let paths = seed_derivation_paths_for_account(0).expect("paths");
        for chain in Chain::all() {
            let expected = crate::chains::default_derivation_path_template(chain).is_some();
            assert_eq!(
                paths.path_for(chain).is_some(),
                expected,
                "{} path presence disagrees with the catalog",
                chain.str_id()
            );
            assert_eq!(paths.by_chain.contains_key(chain.str_id()), expected);
        }

        // Monero derives its keys its own way and has `derivation_path = []`
        // in the catalog, so it is deliberately absent.
        assert_eq!(paths.path_for(Chain::Monero), None);

        // BNB Chain has a catalog template, so it gets an entry.
        assert!(paths.path_for(Chain::BnbChain).is_some());
    }
}

// ── FFI surface ─────────────────────────────────────────────────────────────

// ── Derivation paths ──────────────────────────────────────────────

pub(crate) fn parse_derivation_path_str(raw_path: &str) -> Option<Vec<DerivationPathSegment>> {
    let trimmed = raw_path.trim();
    let mut components = trimmed.split('/');
    let head = components.next()?;
    if !head.eq_ignore_ascii_case("m") {
        return None;
    }
    components
        .map(|component| {
            let is_hardened = component.ends_with('\'');
            let value_string = if is_hardened {
                &component[..component.len().saturating_sub(1)]
            } else {
                component
            };
            value_string
                .parse::<u32>()
                .ok()
                .filter(|value| *value < (1 << 31))
                .map(|value| DerivationPathSegment { value, is_hardened })
        })
        .collect()
}

pub(crate) fn normalize_derivation_path(raw_path: &str, fallback: &str) -> String {
    parse_derivation_path_str(raw_path)
        .map(|segments| format_derivation_path_segments(&segments))
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn format_derivation_path_segments(segments: &[DerivationPathSegment]) -> String {
    let suffix = segments
        .iter()
        .map(|segment| {
            format!(
                "{}{}",
                segment.value,
                if segment.is_hardened { "'" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("/");
    if suffix.is_empty() {
        "m".to_string()
    } else {
        format!("m/{suffix}")
    }
}

/// Default derivation paths for every mainnet chain at `account`, driven off
/// `registry::Chain`.
///
/// Paths are keyed by concrete network. A missing template means that the
/// chain derives without a configurable BIP-32 path.
pub(super) fn seed_derivation_paths_for_account(
    account: u32,
) -> Result<CoreSeedDerivationPaths, String> {
    use crate::registry::Chain;

    let mut by_chain = std::collections::HashMap::new();
    for chain in Chain::all() {
        // Keyed by id rather than display name — ids are the stable key, and
        // `every_catalog_name_resolves` guarantees every name resolves back to
        // the id it belongs to.
        if let Some(template) = crate::chains::default_derivation_path_template(chain) {
            by_chain.insert(
                chain.str_id().to_string(),
                render_derivation_path_template(template, account),
            );
        }
    }
    if by_chain.is_empty() {
        return Err("Chain catalog produced no derivation paths.".to_string());
    }
    Ok(CoreSeedDerivationPaths { by_chain })
}

fn render_derivation_path_template(template: &str, account: u32) -> String {
    template.replace("{account}", &account.to_string())
}

pub(super) fn default_path_from_catalog(chain: crate::registry::Chain) -> Result<String, String> {
    default_path_from_catalog_for_account(chain, 0)
}

fn default_path_from_catalog_for_account(
    chain: crate::registry::Chain,
    account: u32,
) -> Result<String, String> {
    let template = crate::chains::default_derivation_path_template(chain);
    if let Some(template) = template {
        return Ok(render_derivation_path_template(template, account));
    }
    // A chain the registry knows and the catalog gives no path for derives
    // without one — that is what `derivation_path = []` says, and Monero is
    // the mainnet that says it. That is an answer, not a broken catalog row,
    // so it is not an error.
    if chain.uses_derivation_path() {
        Err(format!("Missing default derivation path for {chain}."))
    } else {
        Ok(String::new())
    }
}

/// Extract a UTXO discovery index only when the path prefix matches the
/// chain's default path and the penultimate segment is the requested branch.
pub(crate) fn utxo_discovery_index(
    raw_path: &str,
    chain: crate::registry::Chain,
    branch: u32,
) -> Option<u32> {
    let default_path = default_path_from_catalog(chain).ok()?;
    let path = parse_derivation_path_str(raw_path)?;
    let mut candidate = parse_derivation_path_str(&default_path)?;
    if path.len() != candidate.len() || path.len() < 5 {
        return None;
    }
    let last = path.len() - 1;
    candidate[last - 1] = DerivationPathSegment {
        value: branch,
        is_hardened: false,
    };
    candidate[last] = DerivationPathSegment {
        value: path[last].value,
        is_hardened: false,
    };
    if format_derivation_path_segments(&candidate[..last])
        != format_derivation_path_segments(&path[..last])
    {
        return None;
    }
    if path[last - 1].value != branch {
        return None;
    }
    Some(path[last].value)
}

#[cfg(test)]
pub(super) fn default_path_for_chain(chain: crate::registry::Chain) -> Result<String, String> {
    default_path_from_catalog(chain)
}

// ── FFI surface ──────────────────────────────────────────────────────────

#[uniffi::export]
pub fn parse_derivation_path(raw_path: String) -> Option<Vec<DerivationPathSegment>> {
    parse_derivation_path_str(&raw_path)
}

#[uniffi::export]
pub fn format_derivation_path(segments: Vec<DerivationPathSegment>) -> String {
    format_derivation_path_segments(&segments)
}

/// A discovery path: the chain's default path with its last two segments
/// replaced by branch and index.
pub(crate) fn derivation_path_replacing_last_two(
    raw_path: String,
    branch: u32,
    index: u32,
    fallback: String,
) -> String {
    let normalized = normalize_derivation_path(&raw_path, &fallback);
    let Some(mut segments) = parse_derivation_path_str(&normalized) else {
        return fallback;
    };
    if segments.len() < 2 {
        return fallback;
    }
    let len = segments.len();
    segments[len - 2] = DerivationPathSegment {
        value: branch,
        is_hardened: false,
    };
    segments[len - 1] = DerivationPathSegment {
        value: index,
        is_hardened: false,
    };
    format_derivation_path_segments(&segments)
}

// ── Registry-backed catalog lookups ───────────────────────────────

#[cfg(test)]
mod testnet_derivation_paths {
    use crate::registry::Chain;

    /// Every network resolves independently, including pathless derivation.
    #[test]
    fn every_testnet_resolves_its_own_catalog_path() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            let resolved = super::resolve_derivation_path(chain, String::new());
            assert!(
                resolved.is_ok(),
                "{} failed to resolve: {:?}",
                chain.str_id(),
                resolved.err()
            );
        }
    }

    #[test]
    fn bitcoin_testnet_uses_coin_type_one() {
        let testnet = super::resolve_derivation_path(Chain::BitcoinTestnet4, String::new())
            .expect("testnet4");
        let mainnet =
            super::resolve_derivation_path(Chain::Bitcoin, String::new()).expect("bitcoin");
        assert_eq!(testnet, "m/84'/1'/0'/0/0");
        assert_eq!(mainnet, "m/84'/0'/0'/0/0");
    }
}

#[cfg(test)]
mod endpoint_network_index_tests {
    use super::*;

    /// The network index every endpoint consumer reads through.
    fn rpc_endpoints(chain_id: crate::registry::Chain) -> Vec<String> {
        endpoint_records_for_chain(endpoint_catalog().expect("catalog"), chain_id, &[])
            .into_iter()
            .filter(|r| r.api == EndpointApi::EvmJsonRpc)
            .map(|r| r.endpoint)
            .collect()
    }

    /// Network IDs keep testnet lookups independent of mainnet and UI titles.
    #[test]
    fn a_testnet_resolves_its_own_rpc_endpoints() {
        assert_eq!(
            rpc_endpoints(crate::registry::Chain::EthereumSepolia),
            vec![
                "https://ethereum-sepolia-rpc.publicnode.com".to_string(),
                "https://1rpc.io/sepolia".to_string(),
            ]
        );
        assert_eq!(
            rpc_endpoints(crate::registry::Chain::EthereumHoodi),
            vec![
                "https://ethereum-hoodi-rpc.publicnode.com".to_string(),
                "https://1rpc.io/hoodi".to_string(),
            ]
        );
    }

    #[test]
    fn a_mainnet_rpc_list_holds_no_testnet_endpoints() {
        for endpoint in rpc_endpoints(crate::registry::Chain::Ethereum) {
            assert!(
                !endpoint.contains("sepolia") && !endpoint.contains("hoodi"),
                "Ethereum mainnet RPC list contains {endpoint}"
            );
        }
    }

    /// Settings is the one consumer that wants a chain *and* its testnets, as
    /// separate groups inside one section.
    #[test]
    fn settings_keeps_a_chain_and_its_testnets_together() {
        let catalog = endpoint_catalog().expect("catalog");
        let titles: Vec<String> =
            grouped_settings_entries(catalog, crate::registry::Chain::Bitcoin)
                .into_iter()
                .map(|entry| entry.title)
                .collect();
        for expected in ["Bitcoin Testnet", "Bitcoin Testnet4", "Bitcoin Signet"] {
            assert!(titles.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn every_record_belongs_to_exactly_its_network() {
        let catalog = endpoint_catalog().expect("catalog");
        for chain in crate::registry::Chain::all() {
            let rows = endpoint_records_for_chain(catalog, chain, &[]);
            let expected: Vec<_> = catalog
                .endpoint_records
                .iter()
                .filter(|r| r.chain_id == chain)
                .cloned()
                .collect();
            assert_eq!(rows, expected);
            let groups = grouped_settings_entries(catalog, chain);
            for group in groups {
                let network = group.chain_id;
                assert!(
                    network == chain
                        || (!chain.is_testnet() && network.mainnet_counterpart() == chain)
                );
                assert_eq!(group.title, network.chain_display_name());
            }
        }
    }

    #[test]
    fn invalid_network_ids_and_title_based_ownership_are_refused() {
        let valid = r#"id = "test"
chain_id = "ethereum-sepolia"
api = "evm-json-rpc"
endpoint = "https://example.com"
capabilities = ["balance"]"#;
        let row = toml::from_str::<TomlEndpoint>(valid).unwrap();
        assert_eq!(
            AppCoreEndpointRecord::try_from(row).unwrap().chain_id,
            crate::registry::Chain::EthereumSepolia
        );
        assert!(
            toml::from_str::<TomlEndpoint>(&valid.replace("evm-json-rpc", "made-up-api")).is_err()
        );
        assert!(
            toml::from_str::<TomlEndpoint>(&valid.replace("api = \"evm-json-rpc\"\n", "")).is_err()
        );
        let overclaimed = toml::from_str::<TomlEndpoint>(
            &valid.replace("[\"balance\"]", "[\"balance\", \"history\"]"),
        )
        .unwrap();
        assert!(
            AppCoreEndpointRecord::try_from(overclaimed)
                .unwrap_err()
                .contains("no history adapter")
        );
        let unused = toml::from_str::<TomlEndpoint>(&valid.replace("[\"balance\"]", "[]")).unwrap();
        assert!(AppCoreEndpointRecord::try_from(unused).is_err());

        // A row naming no catalog chain does not parse at all.
        for bad in ["Ethereum Sepolia", "unknown-network", ""] {
            assert!(
                toml::from_str::<TomlEndpoint>(&valid.replace("ethereum-sepolia", bad)).is_err()
            );
        }
        for field in ["chain_id", "group_title", "kind", "explorer_label"] {
            assert!(
                toml::from_str::<TomlEndpoint>(&format!("{valid}\n{field} = \"Ethereum\" "))
                    .is_err()
            );
        }
    }
}

#[cfg(test)]
mod endpoint_capabilities {
    use crate::EndpointCapability;

    fn records() -> Vec<super::AppCoreEndpointRecord> {
        toml::from_str::<super::TomlEndpointFile>(super::APP_ENDPOINT_DIRECTORY_TOML)
            .expect("directory parses")
            .endpoints
            .into_iter()
            .map(|row| super::AppCoreEndpointRecord::try_from(row).unwrap())
            .collect()
    }

    /// An EVM node cannot answer "every transaction for this address".
    ///
    /// There is no such JSON-RPC method — `eth_getTransactionsByAddress` does
    /// not exist, because a node stores blocks and state, and transactions are
    /// indexed by block rather than by address. Answering it means scanning
    /// every block ever produced, which is why the job belongs to a separate
    /// indexer.
    ///
    /// Non-EVM chains are a different matter and deliberately not covered
    /// here: Solana's `getSignaturesForAddress` and XRP's `account_tx` are
    /// real node methods, so those nodes genuinely do carry `history`.
    #[test]
    fn no_evm_node_claims_history() {
        for record in records() {
            if !record.chain_id.is_evm() || record.api != crate::EndpointApi::EvmJsonRpc {
                continue;
            }
            assert!(
                !record.capabilities.iter().any(|c| matches!(
                    c,
                    EndpointCapability::History
                        | EndpointCapability::TokenHistory
                        | EndpointCapability::TokenDiscovery
                )),
                "{} {} is an EVM node and cannot serve address history",
                record.chain_id,
                record.endpoint
            );
        }
    }

    #[test]
    fn token_capabilities_select_the_api_that_can_answer() {
        let selected = |chain: &str, capability: &str| {
            super::filtered_endpoint_records_for_chain(
                crate::registry::Chain::parse(chain).unwrap(),
                &[capability.parse().unwrap()],
            )
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>()
        };
        let balances = selected("ethereum", "token-balance");
        assert!(balances.contains(&"ethereum.rpc.publicnode".into()));
        for capability in ["history", "token-history"] {
            let rows = selected("ethereum", capability);
            assert!(rows.contains(&"ethereum.explorer.blockscout".into()));
            assert!(!rows.contains(&"ethereum.rpc.publicnode".into()));
        }
        // The implemented Blockscout adapter reads transfers, not holdings.
        assert!(selected("ethereum", "token-discovery").is_empty());
        assert!(!balances.contains(&"ethereum.explorer.blockscout".into()));
        // Jetton discovery lives on v3, independently of v2's native history.
        // No TON adapter reads jetton transfers, so nothing claims them.
        assert_eq!(selected("ton", "token-discovery"), ["ton.api.v3"]);
        for capability in ["token-discovery", "token-history"] {
            assert!(selected("bitcoin", capability).is_empty());
        }
        assert!(selected("ton", "token-history").is_empty());
        // v2 reads the metadata required to interpret v3 token balances.
        assert_eq!(
            selected("ton", "token-balance"),
            ["ton.api.v2", "ton.api.v3"]
        );
        assert!(selected("ton", "history").contains(&"ton.api.v2".into()));
        assert!(selected("solana", "token-discovery").contains(&"solana.rpc.mainnet".into()));
        assert!(selected("near", "token-discovery").is_empty());
        assert!(selected("near", "token-balance").contains(&"near.rpc.mainnet".into()));
    }
}

#[cfg(test)]
mod catalog_endpoints_carry_their_api {
    use crate::EndpointCapability;

    fn record(endpoint: &str) -> super::AppCoreEndpointRecord {
        super::endpoint_catalog()
            .unwrap()
            .endpoint_records
            .iter()
            .find(|r| r.endpoint == endpoint)
            .cloned()
            .unwrap_or_else(|| panic!("{endpoint} is in the catalog"))
    }

    #[test]
    fn a_node_and_an_indexer_declare_different_capabilities() {
        let node = record("https://ethereum-rpc.publicnode.com");
        assert_eq!(node.api, crate::EndpointApi::EvmJsonRpc);
        assert!(node.capabilities.contains(&EndpointCapability::Balance));
        assert!(
            !node.capabilities.contains(&EndpointCapability::History),
            "an EVM node cannot serve address history"
        );
        let indexer = record("https://eth.blockscout.com");
        assert!(indexer.capabilities.contains(&EndpointCapability::History));
    }
}
