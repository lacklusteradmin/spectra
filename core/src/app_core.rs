use crate::store::wallet_domain::CoreSeedDerivationPaths;
use crate::EndpointApi;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const APP_ENDPOINT_DIRECTORY_TOML: &str = include_str!("../data/endpoints.toml");

pub const ENDPOINT_CAPABILITY_BALANCE: u32 = 1 << 1;
pub const ENDPOINT_CAPABILITY_HISTORY: u32 = 1 << 2;
pub const ENDPOINT_CAPABILITY_UTXO: u32 = 1 << 3;
pub const ENDPOINT_CAPABILITY_FEE: u32 = 1 << 4;
pub const ENDPOINT_CAPABILITY_BROADCAST: u32 = 1 << 5;
const ENDPOINT_CAPABILITY_VERIFICATION: u32 = 1 << 6;
const ENDPOINT_CAPABILITY_TOKEN_HISTORY: u32 = 1 << 11;
const ENDPOINT_CAPABILITY_TOKEN_DISCOVERY: u32 = 1 << 12;
const ENDPOINT_CAPABILITY_TOKEN_BALANCE: u32 = 1 << 13;

pub(crate) const ENDPOINT_CAPABILITIES: [&str; 10] = [
    "balance",
    "history",
    "token-history",
    "token-discovery",
    "token-balance",
    "utxo",
    "fee",
    "broadcast",
    "verification",
    "staking",
];

#[derive(Debug, Clone)]
pub(crate) struct AppCoreCatalog {
    pub(crate) endpoint_records: Vec<AppCoreEndpointRecord>,
    /// Parallel to `endpoint_records`: pre-computed bitmask per record so the
    /// hot-path filter avoids per-call string matching on capabilities.
    pub(crate) endpoint_filter_masks: Vec<u32>,
    /// Concrete network ID → record indices, preserving endpoint order.
    endpoint_records_by_chain: std::collections::HashMap<String, Vec<usize>>,
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
    chain_id: String,
    api: Option<EndpointApi>,
    endpoint: String,
    capabilities: Vec<String>,
    #[serde(default)]
    probe_url: Option<String>,
    #[serde(default)]
    explorer_label: Option<String>,
    #[serde(default)]
    tx_suffix: String,
}

impl TryFrom<TomlEndpoint> for AppCoreEndpointRecord {
    type Error = String;

    fn try_from(e: TomlEndpoint) -> Result<Self, Self::Error> {
        crate::registry::Chain::from_str_id(&e.chain_id)
            .ok_or_else(|| format!("{}: unknown endpoint chain_id {:?}", e.id, e.chain_id))?;
        if e.api.is_none() && !e.capabilities.is_empty() {
            return Err(format!(
                "{}: web links cannot declare API capabilities",
                e.id
            ));
        }
        if e.api.is_some() && (e.explorer_label.is_some() || !e.tx_suffix.is_empty()) {
            return Err(format!(
                "{}: APIs cannot declare explorer link fields",
                e.id
            ));
        }
        Ok(AppCoreEndpointRecord {
            id: e.id,
            chain_id: e.chain_id,
            api: e.api,
            endpoint: e.endpoint,
            capabilities: e.capabilities,
            probe_url: e.probe_url,
            explorer_label: e.explorer_label,
            tx_suffix: e.tx_suffix,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreEndpointRecord {
    pub id: String,
    pub api: Option<EndpointApi>,
    pub chain_id: String,
    pub endpoint: String,
    /// What this endpoint is used for. A capability is a claim about the
    /// endpoint that has to be true — see `no_evm_node_claims_history`.
    pub capabilities: Vec<String>,
    #[serde(rename = "probeURL")]
    pub probe_url: Option<String>,
    pub explorer_label: Option<String>,
    /// Appended after the transaction hash, for an explorer whose URL needs
    /// more than a prefix. Aptos wants `?network=mainnet`; nothing else does.
    #[serde(default)]
    pub tx_suffix: String,
}

/// What one endpoint is and what it is used for, looked up by URL.
///
/// A lookup rather than a field on `AppCoreGroupedSettingsEntry`, because the
/// settings screen assembles some of its groups itself — Bitcoin's Esplora
/// bases, and whatever RPC the user typed in. Those have no catalog row, and
/// asking by URL lets them come back `None` instead of forcing the caller to
/// invent an API for an endpoint it knows nothing about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreEndpointTag {
    pub api: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreGroupedSettingsEntry {
    pub chain_id: String,
    pub title: String,
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreExplorerEntry {
    pub endpoint: String,
    pub label: String,
    /// Appended after the transaction hash. Empty for every explorer but
    /// Aptos's, which was a `chain_name == "Aptos"` branch inside
    /// `core_transaction_explorer_url` — the one thing that export did that a
    /// caller holding this record could not.
    pub tx_suffix: String,
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
///
/// Returned a four-field record before. `chain` was the argument handed back,
/// and `account_index` and `flavor` were derived from the normalized path for
/// nobody — Swift stored the flavor in a struct with no reader, and the CLI and
/// both in-crate callers took `normalized_path` and dropped the rest.
#[uniffi::export]
pub fn resolve_derivation_path(
    chain: String,
    derivation_path: String,
) -> Result<String, crate::SpectraBridgeError> {
    let default_path = default_path_from_catalog(&chain)?;
    Ok(normalize_derivation_path(&derivation_path, &default_path))
}

#[uniffi::export]
pub fn derivation_paths_for_preset(
    preset: crate::store::wallet_domain::CoreSeedDerivationPreset,
) -> Result<CoreSeedDerivationPaths, crate::SpectraBridgeError> {
    Ok(seed_derivation_paths_for_account(preset.account_index())?)
}

/// A chain's endpoint records, filtered by any requested capability.
///
/// Was also exported with role *names*, for an app wrapper that nothing
/// called; the CLI and core pass the mask constants.
pub fn filtered_endpoint_records_for_chain(
    chain_id: String,
    filter_mask: u32,
) -> Result<Vec<AppCoreEndpointRecord>, crate::SpectraBridgeError> {
    crate::registry::Chain::from_str_id(&chain_id)
        .ok_or_else(|| format!("Unknown endpoint chain_id: {chain_id}"))?;
    let catalog = endpoint_catalog()?;
    Ok(endpoint_records_for_chain(catalog, &chain_id, filter_mask))
}

/// Everything the endpoint catalog holds for one chain.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AppCoreChainEndpoints {
    pub chain_id: String,
    /// RPC endpoints, for the EVM family.
    pub evm_rpc: Vec<String>,
    /// Machine-facing services for this concrete network, excluding web links.
    pub service_endpoints: Vec<String>,
    /// Light-wallet backends eligible for the backend selector.
    pub backends: Vec<String>,
    /// What the settings screen shows, grouped by network.
    pub grouped_settings: Vec<AppCoreGroupedSettingsEntry>,
    pub transaction_explorer: Option<AppCoreExplorerEntry>,
    /// Esplora bases, for the Bitcoin family. Empty elsewhere.
    pub bitcoin_esplora: Vec<String>,
}

/// The endpoint catalog, one row per chain, in catalog order.
#[uniffi::export]
pub fn chain_endpoints() -> Result<Vec<AppCoreChainEndpoints>, crate::SpectraBridgeError> {
    let catalog = endpoint_catalog()?;
    Ok(crate::registry::Chain::all()
        .map(|chain| {
            let id = chain.str_id().to_string();
            AppCoreChainEndpoints {
                service_endpoints: endpoint_records_for_chain(catalog, &id, 0)
                    .into_iter()
                    .filter(|r| r.api.is_some())
                    .map(|r| r.endpoint)
                    .collect(),
                backends: endpoint_records_for_chain(catalog, &id, 0)
                    .into_iter()
                    .filter(|r| r.api == Some(EndpointApi::MoneroDaemonRpc))
                    .map(|r| r.endpoint)
                    .collect(),
                evm_rpc: endpoint_records_for_chain(catalog, &id, 0)
                    .into_iter()
                    .filter(|r| r.api == Some(EndpointApi::EvmJsonRpc))
                    .map(|r| r.endpoint)
                    .collect(),
                grouped_settings: grouped_settings_entries(catalog, chain),
                transaction_explorer: transaction_explorer_entry(catalog, &id),
                bitcoin_esplora: bitcoin_esplora_base_urls(catalog, &id).unwrap_or_default(),
                chain_id: id,
            }
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
    for record in &endpoint_records {
        for capability in &record.capabilities {
            if !ENDPOINT_CAPABILITIES.contains(&capability.as_str()) {
                return Err(format!(
                    "{}: unknown endpoint capability {capability:?}",
                    record.id
                ));
            }
        }
    }
    let endpoint_filter_masks: Vec<u32> = endpoint_records
        .iter()
        .map(|r| {
            r.capabilities
                .iter()
                .fold(0u32, |acc, role| acc | endpoint_filter_bit(role))
        })
        .collect();
    let mut endpoint_records_by_chain: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, record) in endpoint_records.iter().enumerate() {
        endpoint_records_by_chain
            .entry(record.chain_id.clone())
            .or_default()
            .push(idx);
    }
    Ok(AppCoreCatalog {
        endpoint_records,
        endpoint_filter_masks,
        endpoint_records_by_chain,
    })
}

pub(crate) fn endpoint_filter_bit(role: &str) -> u32 {
    match role {
        "balance" => ENDPOINT_CAPABILITY_BALANCE,
        "history" => ENDPOINT_CAPABILITY_HISTORY,
        "token-history" => ENDPOINT_CAPABILITY_TOKEN_HISTORY,
        "token-discovery" => ENDPOINT_CAPABILITY_TOKEN_DISCOVERY,
        "token-balance" => ENDPOINT_CAPABILITY_TOKEN_BALANCE,
        "utxo" => ENDPOINT_CAPABILITY_UTXO,
        "fee" => ENDPOINT_CAPABILITY_FEE,
        "broadcast" => ENDPOINT_CAPABILITY_BROADCAST,
        "verification" => ENDPOINT_CAPABILITY_VERIFICATION,
        "staking" => 1 << 14,
        _ => 0,
    }
}

fn endpoint_records_for_chain(
    catalog: &AppCoreCatalog,
    chain_id: &str,
    filter_mask: u32,
) -> Vec<AppCoreEndpointRecord> {
    records_from(
        catalog,
        catalog.endpoint_records_by_chain.get(chain_id),
        filter_mask,
    )
}

fn records_from(
    catalog: &AppCoreCatalog,
    indices: Option<&Vec<usize>>,
    filter_mask: u32,
) -> Vec<AppCoreEndpointRecord> {
    let Some(indices) = indices else {
        return Vec::new();
    };
    indices
        .iter()
        .filter_map(|&idx| {
            let record = &catalog.endpoint_records[idx];
            if filter_mask != 0 && catalog.endpoint_filter_masks[idx] & filter_mask == 0 {
                return None;
            }
            Some(record.clone())
        })
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
            for record in endpoint_records_for_chain(catalog, network.str_id(), 0) {
                if !endpoints.contains(&record.endpoint) {
                    endpoints.push(record.endpoint);
                }
            }
            (!endpoints.is_empty()).then(|| AppCoreGroupedSettingsEntry {
                chain_id: network.str_id().to_string(),
                title: network.chain_display_name().to_string(),
                endpoints,
            })
        })
        .collect()
}

fn transaction_explorer_entry(
    catalog: &AppCoreCatalog,
    chain_id: &str,
) -> Option<AppCoreExplorerEntry> {
    endpoint_records_for_chain(catalog, chain_id, 0)
        .into_iter()
        .filter(|record| record.api.is_none())
        .find_map(|record| {
            record.explorer_label.map(|label| AppCoreExplorerEntry {
                endpoint: record.endpoint,
                label,
                tx_suffix: record.tx_suffix,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "This chain has no derivation path" is an answer, not a failure.
    #[test]
    fn a_chain_with_no_catalog_path_derives_without_one() {
        use crate::registry::Chain;

        assert!(!Chain::Monero.uses_derivation_path());
        assert_eq!(default_path_for_chain("Monero").expect("an answer"), "");

        // Monero is the only mainnet that says it, so a second one appearing
        // is a catalog edit to notice rather than a silent empty path.
        for chain in Chain::all().filter(|c| !c.is_testnet() && *c != Chain::Monero) {
            assert!(
                chain.uses_derivation_path(),
                "{} has no catalog derivation path",
                chain.chain_display_name()
            );
        }

        // A chain the registry does not know is still an error: the fallback
        // is for rows that say "none", not for names that say nothing.
        assert!(default_path_for_chain("Not A Chain").is_err());
    }

    #[test]
    fn resolves_bitcoin_taproot_path() {
        let default_path = default_path_for_chain("Bitcoin").expect("default path");
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
            let expected =
                crate::chains::default_derivation_path_template_by_id(chain.str_id()).is_some();
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

/// What the catalog knows about one endpoint URL. `None` for anything it does
/// not list — a user's own RPC, or a runtime-assembled Esplora base.
#[uniffi::export]
pub fn endpoint_tag(endpoint: String) -> Option<AppCoreEndpointTag> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    load_endpoint_catalog()
        .ok()?
        .endpoint_records
        .iter()
        .find(|r| r.endpoint.trim_end_matches('/') == trimmed)
        .map(|r| AppCoreEndpointTag {
            api: r.api.map(|api| api.as_str().to_string()),
            capabilities: r.capabilities.clone(),
        })
}

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

/// Default derivation paths for every mainnet chain at `account`.
///
/// Driven off `registry::Chain` rather than a hand-written list. The list this
/// replaced named 44 chains and had to be edited alongside `chains.toml`, the
/// Rust record and the Swift enum every time a chain was added.
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
        if let Some(template) =
            crate::chains::default_derivation_path_template_by_id(chain.str_id())
        {
            by_chain.insert(
                chain.str_id().to_string(),
                render_derivation_path_template(template, account),
            );
        }
    }
    if by_chain.is_empty() {
        return Err("Chain catalog produced no derivation paths.".to_string());
    }
    Ok(CoreSeedDerivationPaths {
        is_custom_enabled: false,
        by_chain,
    })
}

fn render_derivation_path_template(template: &str, account: u32) -> String {
    template.replace("{account}", &account.to_string())
}

pub(super) fn default_path_from_catalog(chain_name: &str) -> Result<String, String> {
    default_path_from_catalog_for_account(chain_name, 0)
}

fn default_path_from_catalog_for_account(chain_name: &str, account: u32) -> Result<String, String> {
    use crate::registry::Chain;

    let template = crate::chains::default_derivation_path_template(chain_name);
    if let Some(template) = template {
        return Ok(render_derivation_path_template(template, account));
    }
    // A chain the registry knows and the catalog gives no path for derives
    // without one — that is what `derivation_path = []` says, and Monero is
    // the mainnet that says it. Erroring here made "the answer is none"
    // indistinguishable from "the catalog row is broken", and every caller in
    // the import pipeline treated it as the second: the CLI refused the
    // import, and iOS dropped the chain out of the batch it was deriving.
    match Chain::from_display_name(chain_name) {
        Some(chain) if !chain.uses_derivation_path() => Ok(String::new()),
        _ => Err(format!("Missing default derivation path for {chain_name}.")),
    }
}

/// Extract a UTXO discovery index only when the path prefix matches the
/// chain's default path and the penultimate segment is the requested branch.
pub(crate) fn utxo_discovery_index(raw_path: &str, chain_name: &str, branch: u32) -> Option<u32> {
    let default_path = default_path_from_catalog(chain_name).ok()?;
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
pub(super) fn default_path_for_chain(chain_name: &str) -> Result<String, String> {
    default_path_from_catalog(chain_name)
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

// ── Bitcoin URL groups ────────────────────────────────────────────────────

pub(super) fn bitcoin_esplora_base_urls(
    catalog: &AppCoreCatalog,
    chain_id: &str,
) -> Result<Vec<String>, String> {
    let chain = crate::registry::Chain::from_str_id(chain_id)
        .ok_or_else(|| format!("Unknown network: {chain_id}"))?;
    if chain.mainnet_counterpart() != crate::registry::Chain::Bitcoin {
        return Err(format!("Not a Bitcoin network: {chain_id}"));
    }
    Ok(endpoint_records_for_chain(catalog, chain_id, 0)
        .into_iter()
        .filter(|record| record.api == Some(EndpointApi::Esplora))
        .map(|record| record.endpoint)
        .collect())
}

#[cfg(test)]
mod testnet_derivation_paths {
    use crate::registry::Chain;

    /// Every network resolves independently, including pathless derivation.
    #[test]
    fn every_testnet_resolves_its_own_catalog_path() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            let resolved = super::resolve_derivation_path(
                chain.chain_display_name().to_string(),
                String::new(),
            );
            assert!(
                resolved.is_ok(),
                "{} failed to resolve: {:?}",
                chain.chain_display_name(),
                resolved.err()
            );
        }
    }

    #[test]
    fn bitcoin_testnet_uses_coin_type_one() {
        let testnet = super::resolve_derivation_path("Bitcoin Testnet4".to_string(), String::new())
            .expect("testnet4");
        let mainnet =
            super::resolve_derivation_path("Bitcoin".to_string(), String::new()).expect("bitcoin");
        assert_eq!(testnet, "m/84'/1'/0'/0/0");
        assert_eq!(mainnet, "m/84'/0'/0'/0/0");
    }
}

#[cfg(test)]
mod endpoint_network_index_tests {
    use super::*;

    /// Reads through the one catalog the front ends read, so the index this
    /// asserts about is the index they get.
    fn rpc_endpoints(chain_id: &str) -> Vec<String> {
        chain_endpoints()
            .expect("catalog")
            .into_iter()
            .find(|entry| entry.chain_id == chain_id)
            .map(|entry| entry.evm_rpc)
            .unwrap_or_default()
    }

    /// Network IDs keep testnet lookups independent of mainnet and UI titles.
    #[test]
    fn a_testnet_resolves_its_own_rpc_endpoints() {
        assert_eq!(
            rpc_endpoints("ethereum-sepolia"),
            vec!["https://ethereum-sepolia-rpc.publicnode.com".to_string()]
        );
        assert_eq!(
            rpc_endpoints("ethereum-hoodi"),
            vec!["https://ethereum-hoodi-rpc.publicnode.com".to_string()]
        );
    }

    #[test]
    fn a_mainnet_rpc_list_holds_no_testnet_endpoints() {
        for endpoint in rpc_endpoints("ethereum") {
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
    fn showing_explorer_links_does_not_make_them_backend_choices() {
        let monero = chain_endpoints()
            .unwrap()
            .into_iter()
            .find(|row| row.chain_id == "monero")
            .unwrap();
        let explorer = monero.transaction_explorer.unwrap().endpoint;
        assert!(monero.grouped_settings[0].endpoints.contains(&explorer));
        assert!(!monero.backends.is_empty());
        assert!(!monero.backends.contains(&explorer));
        assert!(!monero.service_endpoints.contains(&explorer));
    }

    #[test]
    fn every_record_belongs_to_exactly_its_network() {
        let catalog = endpoint_catalog().expect("catalog");
        for chain in crate::registry::Chain::all() {
            let rows = endpoint_records_for_chain(catalog, chain.str_id(), 0);
            let expected: Vec<_> = catalog
                .endpoint_records
                .iter()
                .filter(|r| r.chain_id == chain.str_id())
                .cloned()
                .collect();
            assert_eq!(rows, expected);
            let groups = grouped_settings_entries(catalog, chain);
            for group in groups {
                let network = crate::registry::Chain::from_str_id(&group.chain_id).unwrap();
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
capabilities = []"#;
        let row = toml::from_str::<TomlEndpoint>(valid).unwrap();
        assert_eq!(
            AppCoreEndpointRecord::try_from(row).unwrap().chain_id,
            "ethereum-sepolia"
        );
        assert!(
            toml::from_str::<TomlEndpoint>(&valid.replace("evm-json-rpc", "made-up-api")).is_err()
        );
        let missing =
            toml::from_str::<TomlEndpoint>(&valid.replace("api = \"evm-json-rpc\"\n", "")).unwrap();
        assert!(AppCoreEndpointRecord::try_from(missing).is_ok());
        let link_with_api =
            toml::from_str::<TomlEndpoint>(&format!("{valid}\nexplorer_label = \"Explorer\""))
                .unwrap();
        assert!(AppCoreEndpointRecord::try_from(link_with_api).is_err());
        let link = format!(
            "{}\nexplorer_label = \"Explorer\"",
            valid.replace("api = \"evm-json-rpc\"\n", "")
        );
        assert!(
            AppCoreEndpointRecord::try_from(toml::from_str::<TomlEndpoint>(&link).unwrap()).is_ok()
        );
        let link_with_capability =
            link.replace("capabilities = []", "capabilities = [\"balance\"]");
        assert!(AppCoreEndpointRecord::try_from(
            toml::from_str::<TomlEndpoint>(&link_with_capability).unwrap()
        )
        .is_err());

        for bad in ["Ethereum Sepolia", "unknown-network", ""] {
            let row =
                toml::from_str::<TomlEndpoint>(&valid.replace("ethereum-sepolia", bad)).unwrap();
            assert!(AppCoreEndpointRecord::try_from(row).is_err());
        }
        for field in ["chain_name", "group_title", "kind"] {
            assert!(
                toml::from_str::<TomlEndpoint>(&format!("{valid}\n{field} = \"Ethereum\" "))
                    .is_err()
            );
        }
        assert!(filtered_endpoint_records_for_chain("Ethereum".into(), 0).is_err());
    }
}

#[cfg(test)]
mod endpoint_capabilities {
    use crate::registry::Chain;

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
    /// Forty-six EVM RPC records claimed the `history` capability anyway. It
    /// went unnoticed because nothing reads the field to decide where EVM
    /// history comes from — `Chain::evm_history_source` does — but it was a
    /// false statement in the data, and the next thing to trust the field
    /// would have inherited it.
    ///
    /// Non-EVM chains are a different matter and deliberately not covered
    /// here: Solana's `getSignaturesForAddress` and XRP's `account_tx` are
    /// real node methods, so those nodes genuinely do carry `history`.
    #[test]
    fn no_evm_node_claims_history() {
        for record in records() {
            let Some(chain) = Chain::from_str_id(&record.chain_id) else {
                continue;
            };
            if !chain.is_evm() || record.api != Some(crate::EndpointApi::EvmJsonRpc) {
                continue;
            }
            assert!(
                !record
                    .capabilities
                    .iter()
                    .any(|c| matches!(c.as_str(), "history" | "token-history" | "token-discovery")),
                "{} {} is an EVM node and cannot serve address history",
                record.chain_id,
                record.endpoint
            );
        }
    }

    #[test]
    fn every_capability_owns_a_distinct_bit() {
        let mut bits = std::collections::HashSet::new();
        for capability in super::ENDPOINT_CAPABILITIES {
            let bit = super::endpoint_filter_bit(capability);
            assert!(bit.is_power_of_two());
            assert!(bits.insert(bit));
        }
    }

    #[test]
    fn token_capabilities_select_the_api_that_can_answer() {
        let selected = |chain: &str, capability: &str| {
            super::filtered_endpoint_records_for_chain(
                chain.into(),
                super::endpoint_filter_bit(capability),
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
        // Jetton indexing lives on v3, independently of v2's native history.
        for capability in ["token-discovery", "token-history"] {
            assert_eq!(selected("ton", capability), ["ton.api.v3"]);
            assert!(selected("bitcoin", capability).is_empty());
        }
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

    /// A web link is a URL for a person, so it claims nothing.
    #[test]
    fn a_web_link_has_no_capabilities() {
        for record in records().iter().filter(|r| r.api.is_none()) {
            assert!(
                record.capabilities.is_empty(),
                "{} {} is a link and claims {:?}",
                record.chain_id,
                record.endpoint,
                record.capabilities
            );
        }
    }
}

#[cfg(test)]
mod an_endpoint_can_be_asked_what_it_is {
    /// The settings screen shows a URL per row and nothing else. The catalog
    /// knows what each one is; this is how the row asks.
    #[test]
    fn a_catalog_endpoint_reports_its_api_and_capabilities() {
        let node = super::endpoint_tag("https://ethereum-rpc.publicnode.com".into())
            .expect("Ethereum's node is in the catalog");
        assert_eq!(node.api.as_deref(), Some("evm-json-rpc"));
        assert!(node.capabilities.contains(&"balance".to_string()));
        assert!(
            !node.capabilities.contains(&"history".to_string()),
            "an EVM node cannot serve address history"
        );

        let indexer = super::endpoint_tag("https://eth.blockscout.com".into())
            .expect("Ethereum's indexer is in the catalog");
        assert_eq!(indexer.api.as_deref(), Some("blockscout"));
        assert!(indexer.capabilities.contains(&"history".to_string()));
    }

    /// A trailing slash is not a different endpoint.
    #[test]
    fn the_lookup_ignores_a_trailing_slash() {
        assert_eq!(
            super::endpoint_tag("https://eth.blockscout.com/".into()),
            super::endpoint_tag("https://eth.blockscout.com".into()),
        );
    }

    /// An endpoint the user typed has no catalog row, and saying so beats
    /// guessing an API for it.
    #[test]
    fn an_endpoint_the_catalog_does_not_list_has_no_tag() {
        assert!(super::endpoint_tag("https://my-own-node.example".into()).is_none());
    }
}
