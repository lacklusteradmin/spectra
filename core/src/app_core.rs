use crate::store::wallet_domain::CoreSeedDerivationPaths;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const APP_ENDPOINT_DIRECTORY_TOML: &str = include_str!("../data/endpoints.toml");

const ENDPOINT_ROLE_READ: u32 = 1 << 0;
pub(crate) const ENDPOINT_ROLE_BALANCE: u32 = 1 << 1;
const ENDPOINT_ROLE_HISTORY: u32 = 1 << 2;
const ENDPOINT_ROLE_UTXO: u32 = 1 << 3;
const ENDPOINT_ROLE_FEE: u32 = 1 << 4;
const ENDPOINT_ROLE_BROADCAST: u32 = 1 << 5;
const ENDPOINT_ROLE_VERIFICATION: u32 = 1 << 6;
pub(crate) const ENDPOINT_ROLE_RPC: u32 = 1 << 7;
const ENDPOINT_ROLE_EXPLORER: u32 = 1 << 8;
/// An address-indexed API. Its own bit because an indexer is not a `/tx/`
/// link, and the two were sharing one — which is how `explorer_supplemental`
/// briefly picked up every Esplora endpoint Bitcoin has.
const ENDPOINT_ROLE_INDEXER: u32 = 1 << 9;
/// A Monero light-wallet server. Its own bit for the same reason — which the
/// comment above did not stop this one from being written `1 << 9` as well.
/// `catalog_endpoints` asks for `RPC | BALANCE | BACKEND`, so every indexer
/// matched it too, and the sixteen indexer rows that carry no `balance`
/// capability were handed to non-EVM chains as general API bases: Bitcoin
/// Cash's primary list held `/push/transaction` and a `/dashboards/transaction/`
/// URL prefix, which `with_fallback` would try for a balance read.
pub(crate) const ENDPOINT_ROLE_BACKEND: u32 = 1 << 10;

/// Endpoint-table slot for a given chain. Mirrors `crate::registry::EndpointSlot`
/// so the Swift side can ask Rust for the right `chain_id + offset` instead of
/// reimplementing the offset arithmetic.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AppCoreEndpointSlot {
    Primary,
    Secondary,
    Explorer,
}

#[derive(Debug, Clone)]
pub(crate) struct AppCoreCatalog {
    pub(crate) endpoint_records: Vec<AppCoreEndpointRecord>,
    /// Parallel to `endpoint_records`: pre-computed bitmask per record so the
    /// hot-path filter avoids per-call string matching on `roles`.
    pub(crate) endpoint_role_masks: Vec<u32>,
    /// Pre-indexed *network* → record-index list, where a testnet's records
    /// are indexed under the testnet rather than under its mainnet. Backs the
    /// lookups that must return one network's endpoints and no other's.
    pub(crate) endpoint_records_by_chain: std::collections::HashMap<String, Vec<usize>>,
    /// Pre-indexed `chain_name` → record-index list, keeping a chain and its
    /// testnets together. The settings screen wants exactly this: one section
    /// per chain, with the testnets as groups inside it.
    pub(crate) endpoint_records_by_settings_chain: std::collections::HashMap<String, Vec<usize>>,
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
struct TomlEndpoint {
    id: String,
    chain_name: String,
    /// Omitted when it is the chain's own name, which is all but the testnets.
    #[serde(default)]
    group_title: Option<String>,
    provider_id: String,
    endpoint: String,
    kind: String,
    capabilities: Vec<String>,
    #[serde(default)]
    probe_url: Option<String>,
    #[serde(default)]
    settings_visible: bool,
    #[serde(default)]
    supplements_rpc_list: bool,
    #[serde(default)]
    explorer_label: Option<String>,
    #[serde(default)]
    tx_suffix: String,
}

impl From<TomlEndpoint> for AppCoreEndpointRecord {
    fn from(e: TomlEndpoint) -> Self {
        AppCoreEndpointRecord {
            group_title: e.group_title.unwrap_or_else(|| e.chain_name.clone()),
            id: e.id,
            chain_name: e.chain_name,
            provider_id: e.provider_id,
            endpoint: e.endpoint,
            kind: e.kind,
            capabilities: e.capabilities,
            probe_url: e.probe_url,
            settings_visible: e.settings_visible,
            supplements_rpc_list: e.supplements_rpc_list,
            explorer_label: e.explorer_label,
            tx_suffix: e.tx_suffix,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreEndpointRecord {
    pub id: String,
    pub chain_name: String,
    pub group_title: String,
    #[serde(rename = "providerID")]
    pub provider_id: String,
    pub endpoint: String,
    /// What this endpoint *is*, which decides how to talk to it: a JSON-RPC
    /// node, an address-indexed API, a Monero light-wallet backend, or a
    /// `/tx/` link for a person to tap.
    ///
    /// Split out of `roles`, which held this alongside what the endpoint is
    /// *used for*. Nothing forced the two to stay consistent, and both drifted:
    /// ten EVM chains' JSON-RPC nodes were missing the `rpc` marker, so the
    /// diagnostics screen probed them with a GET and called them unreachable.
    pub kind: String,
    /// What this endpoint is used for. A capability is a claim about the
    /// endpoint that has to be true — see `no_evm_node_claims_history`.
    pub capabilities: Vec<String>,
    #[serde(rename = "probeURL")]
    pub probe_url: Option<String>,
    pub settings_visible: bool,
    /// Registered alongside the chain's RPC list rather than instead of it.
    ///
    /// Was inferred from an `explorer` tag that four records carried — three
    /// of them Etherscan V1 endpoints that have since been shut down — while
    /// XRP's `xrpscan` and NEAR's `nearblocks`, which are the same kind of
    /// thing, did not. A tag that describes one member is not describing
    /// anything, so this says it outright.
    #[serde(default)]
    pub supplements_rpc_list: bool,
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
/// invent a kind for an endpoint it knows nothing about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreEndpointTag {
    /// `rpc-node`, `indexer`, `web-link` or `backend`.
    pub kind: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreGroupedSettingsEntry {
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
pub fn app_core_resolve_derivation_path(
    chain: String,
    derivation_path: String,
) -> Result<String, crate::SpectraBridgeError> {
    let default_path = default_path_from_catalog(&chain)?;
    Ok(normalize_derivation_path(&derivation_path, &default_path))
}

#[uniffi::export]
pub fn app_core_derivation_paths_for_preset(
    preset: crate::store::wallet_domain::CoreSeedDerivationPreset,
) -> Result<CoreSeedDerivationPaths, crate::SpectraBridgeError> {
    Ok(seed_derivation_paths_for_account(preset.account_index())?)
}

/// A chain's endpoint records, filtered to a role mask.
///
/// Was also exported with role *names*, for an app wrapper that nothing
/// called; the CLI and core pass the mask constants.
pub fn endpoint_records_for_chain_masked(
    chain_name: String,
    role_mask: u32,
    settings_visible_only: bool,
) -> Result<Vec<AppCoreEndpointRecord>, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    Ok(endpoint_records_for_chain(
        catalog,
        &chain_name,
        role_mask,
        settings_visible_only,
    ))
}

/// Everything the endpoint catalog holds for one chain.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AppCoreChainEndpoints {
    pub chain_id: String,
    pub chain_name: String,
    /// RPC endpoints, for the EVM family.
    pub evm_rpc: Vec<String>,
    /// Explorer endpoints that supplement the RPC list.
    pub explorer_supplemental: Vec<String>,
    /// What the settings screen shows, grouped by network.
    pub grouped_settings: Vec<AppCoreGroupedSettingsEntry>,
    pub transaction_explorer: Option<AppCoreExplorerEntry>,
    /// Esplora bases, for the Bitcoin family. Empty elsewhere.
    pub bitcoin_esplora: Vec<String>,
}

/// The endpoint catalog, one row per chain, in catalog order.
#[uniffi::export]
pub fn app_core_chain_endpoints() -> Result<Vec<AppCoreChainEndpoints>, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    Ok(crate::registry::Chain::all()
        .map(|chain| {
            let name = chain.chain_display_name().to_string();
            let id = chain.str_id().to_string();
            AppCoreChainEndpoints {
                evm_rpc: endpoint_records_for_chain(catalog, &name, ENDPOINT_ROLE_RPC, false)
                    .into_iter()
                    .map(|r| r.endpoint)
                    .collect(),
                // Settings-visible indexers: the address-indexed APIs a user
                // can see and switch, registered alongside the RPC list.
                //
                // This filtered on an `explorer` tag that four records carried
                // and others just as much deserved — XRP's `xrpscan` and
                // NEAR's `nearblocks` are the same kind of thing and were not
                // tagged. Three of the four were Etherscan V1 endpoints that
                // have since been shut down, so the tag was down to one member
                // and was not describing anything. The kind does.
                explorer_supplemental: endpoint_records_for_chain(catalog, &name, 0, false)
                    .into_iter()
                    .filter(|r| r.supplements_rpc_list)
                    .map(|r| r.endpoint)
                    .collect(),
                grouped_settings: grouped_settings_entries(catalog, &name),
                transaction_explorer: transaction_explorer_entry(catalog, &name),
                bitcoin_esplora: bitcoin_esplora_base_urls(catalog, &id).unwrap_or_default(),
                chain_id: id,
                chain_name: name,
            }
        })
        .collect())
}

/// Endpoints for catalog record ids, in the order asked.
///
/// Not exported: the app's only call named three Monero backend ids and three
/// display names beside them. It reads the catalog's settings list for the
/// chain now, which is where those three already were.
pub fn app_core_endpoints_for_ids(
    ids: Vec<String>,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().and_then(|catalog| {
        ids.iter()
            .map(|id| {
                catalog
                    .endpoint_records
                    .iter()
                    .find(|r| &r.id == id)
                    .map(|r| r.endpoint.clone())
                    .ok_or_else(|| format!("Missing endpoint record for id: {id}"))
            })
            .collect::<Result<Vec<_>, _>>()
    })?)
}

// ── Internals ─────────────────────────────────────────────────────────────

pub(crate) fn app_core_catalog() -> Result<&'static AppCoreCatalog, String> {
    match APP_CORE_CATALOG.get_or_init(load_app_core_catalog) {
        Ok(catalog) => Ok(catalog),
        Err(message) => Err(message.clone()),
    }
}

fn load_app_core_catalog() -> Result<AppCoreCatalog, String> {
    let endpoint_records = toml::from_str::<TomlEndpointFile>(APP_ENDPOINT_DIRECTORY_TOML)
        .map_err(|e| e.to_string())?
        .endpoints
        .into_iter()
        .map(AppCoreEndpointRecord::from)
        .collect::<Vec<_>>();
    let endpoint_role_masks: Vec<u32> = endpoint_records
        .iter()
        .map(|r| {
            r.capabilities
                .iter()
                .chain(std::iter::once(&r.kind))
                .fold(0u32, |acc, role| acc | endpoint_role_bit(role))
        })
        .collect();
    // A testnet's records are filed under its *mainnet* `chainName`, with the
    // testnet named only in `groupTitle` — that is how all eight of them are
    // written (Bitcoin Testnet/Testnet4/Signet, Dogecoin Testnet, Ethereum
    // Sepolia/Hoodi). Indexing by `chainName` alone therefore did two wrong
    // things at once: asking for "Ethereum Sepolia" found nothing, and asking
    // for "Ethereum" returned the Sepolia and Hoodi RPCs along with mainnet's.
    //
    // A record belongs to the chain its group names, and to its `chainName`
    // only when the group does not name a different one.
    let mut endpoint_records_by_chain: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    let mut endpoint_records_by_settings_chain: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, record) in endpoint_records.iter().enumerate() {
        let owner = if record.group_title.is_empty() {
            record.chain_name.clone()
        } else {
            record.group_title.clone()
        };
        endpoint_records_by_chain
            .entry(owner)
            .or_default()
            .push(idx);
        endpoint_records_by_settings_chain
            .entry(record.chain_name.clone())
            .or_default()
            .push(idx);
    }
    Ok(AppCoreCatalog {
        endpoint_records,
        endpoint_role_masks,
        endpoint_records_by_chain,
        endpoint_records_by_settings_chain,
    })
}

pub(crate) fn endpoint_role_bit(role: &str) -> u32 {
    match role {
        "read" => ENDPOINT_ROLE_READ,
        "balance" => ENDPOINT_ROLE_BALANCE,
        "history" => ENDPOINT_ROLE_HISTORY,
        "utxo" => ENDPOINT_ROLE_UTXO,
        "fee" => ENDPOINT_ROLE_FEE,
        "broadcast" => ENDPOINT_ROLE_BROADCAST,
        "verification" => ENDPOINT_ROLE_VERIFICATION,
        // Kinds map onto the same mask so a caller can still ask for "the RPC
        // nodes" in one filter. `rpc` is the name the old data used.
        "rpc" | "rpc-node" => ENDPOINT_ROLE_RPC,
        "explorer" | "web-link" => ENDPOINT_ROLE_EXPLORER,
        "indexer" => ENDPOINT_ROLE_INDEXER,
        "backend" => ENDPOINT_ROLE_BACKEND,
        _ => 0,
    }
}

fn endpoint_records_for_chain(
    catalog: &AppCoreCatalog,
    chain_name: &str,
    role_mask: u32,
    settings_visible_only: bool,
) -> Vec<AppCoreEndpointRecord> {
    records_from(
        catalog,
        catalog.endpoint_records_by_chain.get(chain_name),
        role_mask,
        settings_visible_only,
    )
}

fn records_from(
    catalog: &AppCoreCatalog,
    indices: Option<&Vec<usize>>,
    role_mask: u32,
    settings_visible_only: bool,
) -> Vec<AppCoreEndpointRecord> {
    let Some(indices) = indices else {
        return Vec::new();
    };
    indices
        .iter()
        .filter_map(|&idx| {
            let record = &catalog.endpoint_records[idx];
            if settings_visible_only && !record.settings_visible {
                return None;
            }
            if role_mask != 0 && catalog.endpoint_role_masks[idx] & role_mask == 0 {
                return None;
            }
            Some(record.clone())
        })
        .collect()
}

fn grouped_settings_entries(
    catalog: &AppCoreCatalog,
    chain_name: &str,
) -> Vec<AppCoreGroupedSettingsEntry> {
    // Settings shows a chain *and its testnets*, each as its own group, so this
    // reads the by-chain index rather than the per-network one.
    let visible_records = records_from(
        catalog,
        catalog.endpoint_records_by_settings_chain.get(chain_name),
        0,
        true,
    );
    let mut titles = Vec::<String>::new();
    let mut grouped = std::collections::BTreeMap::<String, Vec<String>>::new();
    for record in visible_records {
        if !titles.contains(&record.group_title) {
            titles.push(record.group_title.clone());
        }
        let endpoints = grouped.entry(record.group_title).or_default();
        if !endpoints.contains(&record.endpoint) {
            endpoints.push(record.endpoint);
        }
    }
    titles
        .into_iter()
        .filter_map(|title| {
            grouped
                .get(&title)
                .cloned()
                .filter(|endpoints| !endpoints.is_empty())
                .map(|endpoints| AppCoreGroupedSettingsEntry { title, endpoints })
        })
        .collect()
}

fn transaction_explorer_entry(
    catalog: &AppCoreCatalog,
    chain_name: &str,
) -> Option<AppCoreExplorerEntry> {
    // A `/tx/` link for a person to open, which is exactly `web-link`.
    endpoint_records_for_chain(catalog, chain_name, ENDPOINT_ROLE_EXPLORER, false)
        .into_iter()
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

    /// Every mainnet chain the catalog gives a BIP-32 template for gets a path,
    /// and testnets resolve through their mainnet counterpart rather than
    /// carrying their own entry.
    #[test]
    fn derivation_paths_cover_the_catalog_and_resolve_testnets() {
        use crate::registry::Chain;

        let paths = seed_derivation_paths_for_account(0).expect("paths");
        for chain in Chain::all() {
            let mainnet = chain.mainnet_counterpart();
            let expected =
                crate::chains::default_derivation_path_template_by_id(mainnet.str_id()).is_some();
            assert_eq!(
                paths.path_for(chain).is_some(),
                expected,
                "{} path presence disagrees with the catalog",
                chain.str_id()
            );
            if chain.is_testnet() {
                assert!(
                    !paths.by_chain.contains_key(chain.str_id()),
                    "{} should not have its own entry",
                    chain.str_id()
                );
                assert_eq!(
                    paths.path_for(chain),
                    paths.path_for(mainnet),
                    "{} must resolve to its mainnet path",
                    chain.str_id()
                );
            }
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
pub fn app_core_endpoint_tag(endpoint: String) -> Option<AppCoreEndpointTag> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    load_app_core_catalog()
        .ok()?
        .endpoint_records
        .iter()
        .find(|r| r.endpoint.trim_end_matches('/') == trimmed)
        .map(|r| AppCoreEndpointTag {
            kind: r.kind.clone(),
            capabilities: r.capabilities.clone(),
        })
}

// ── Derivation paths ──────────────────────────────────────────────

pub(crate) fn parse_derivation_path(raw_path: &str) -> Option<Vec<DerivationPathSegment>> {
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
    parse_derivation_path(raw_path)
        .map(|segments| derivation_path_string(&segments))
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn derivation_path_string(segments: &[DerivationPathSegment]) -> String {
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
/// Testnets are skipped: they resolve through `mainnet_counterpart()` at read
/// time. Chains whose catalog entry has no template are skipped rather than
/// failing the whole build — one unconfigured chain should not take out
/// derivation for the other 45.
pub(super) fn seed_derivation_paths_for_account(
    account: u32,
) -> Result<CoreSeedDerivationPaths, String> {
    use crate::registry::Chain;

    let mut by_chain = std::collections::HashMap::new();
    for chain in Chain::all() {
        if chain.is_testnet() {
            continue;
        }
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

    // Testnets carry `derivation_path = []` in the catalog: they derive from
    // their mainnet's path and differ only in address encoding. Resolving
    // through `mainnet_counterpart` states that rule here too — without it,
    // asking for any testnet's path fails, and the iOS caller turns that into
    // a `fatalError`.
    let template = crate::chains::default_derivation_path_template(chain_name).or_else(|| {
        Chain::from_display_name(chain_name).and_then(|chain| {
            crate::chains::default_derivation_path_template_by_id(
                chain.mainnet_counterpart().str_id(),
            )
        })
    });
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

/// The index a UTXO-discovery derivation path encodes, or `None` when the path
/// is not one of this chain's discovery paths on that branch.
///
/// A discovery path is the chain's default path with its last two segments
/// replaced by branch and index, so the test is that everything *before* those
/// two matches and the branch is the one asked about. Ported from Swift's
/// `parseUTXODiscoveryIndex`, which is where the keypool baseline used to be
/// computed.
pub(crate) fn utxo_discovery_index(raw_path: &str, chain_name: &str, branch: u32) -> Option<u32> {
    let default_path = default_path_from_catalog(chain_name).ok()?;
    let path = parse_derivation_path(raw_path)?;
    let mut candidate = parse_derivation_path(&default_path)?;
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
    if derivation_path_string(&candidate[..last]) != derivation_path_string(&path[..last]) {
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
pub fn core_parse_derivation_path(raw_path: String) -> Option<Vec<DerivationPathSegment>> {
    parse_derivation_path(&raw_path)
}

#[uniffi::export]
pub fn core_derivation_path_string(segments: Vec<DerivationPathSegment>) -> String {
    derivation_path_string(&segments)
}

/// A discovery path: the chain's default path with its last two segments
/// replaced by branch and index.
pub(crate) fn core_derivation_path_replacing_last_two(
    raw_path: String,
    branch: u32,
    index: u32,
    fallback: String,
) -> String {
    let normalized = normalize_derivation_path(&raw_path, &fallback);
    let Some(mut segments) = parse_derivation_path(&normalized) else {
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
    derivation_path_string(&segments)
}

// ── Registry-backed catalog lookups ───────────────────────────────

// ── Bitcoin URL groups ────────────────────────────────────────────────────

pub(super) fn bitcoin_esplora_base_urls(
    catalog: &AppCoreCatalog,
    chain_id: &str,
) -> Result<Vec<String>, String> {
    let ids: &[&str] = match chain_id {
        "bitcoin" => &[
            "bitcoin.mainnet.blockstream",
            "bitcoin.mainnet.mempool",
            "bitcoin.mainnet.mempool_emzy",
            "bitcoin.mainnet.maestro",
        ],
        "bitcoin-testnet" => &["bitcoin.testnet.blockstream", "bitcoin.testnet.mempool"],
        "bitcoin-testnet-4" => &["bitcoin.testnet4.mempool"],
        "bitcoin-signet" => &["bitcoin.signet.blockstream", "bitcoin.signet.mempool"],
        _ => return Err(format!("Not a Bitcoin network: {chain_id}")),
    };
    endpoints_for_known_ids(catalog, ids)
}

fn endpoints_for_known_ids(catalog: &AppCoreCatalog, ids: &[&str]) -> Result<Vec<String>, String> {
    ids.iter()
        .map(|id| {
            catalog
                .endpoint_records
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.endpoint.clone())
                .ok_or_else(|| format!("Missing endpoint record for id: {id}"))
        })
        .collect()
}

#[cfg(test)]
mod testnet_derivation_paths {
    use crate::registry::Chain;

    /// Testnets have no catalog template of their own; asking for one used to
    /// fail, and the iOS caller turns a failure here into a `fatalError`.
    #[test]
    fn every_testnet_resolves_to_its_mainnet_path() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            let mainnet = chain.mainnet_counterpart();
            if crate::chains::default_derivation_path_template_by_id(mainnet.str_id()).is_none() {
                continue; // Monero and friends have no BIP-32 path at all.
            }
            let resolved = super::app_core_resolve_derivation_path(
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
    fn a_testnet_resolves_to_the_same_path_as_its_mainnet() {
        let testnet =
            super::app_core_resolve_derivation_path("Bitcoin Testnet4".to_string(), String::new())
                .expect("testnet4");
        let mainnet = super::app_core_resolve_derivation_path("Bitcoin".to_string(), String::new())
            .expect("bitcoin");
        assert_eq!(testnet, mainnet);
    }
}

#[cfg(test)]
mod endpoint_network_index_tests {
    use super::*;

    /// Reads through the one catalog the front ends read, so the index this
    /// asserts about is the index they get.
    fn rpc_endpoints(chain_name: &str) -> Vec<String> {
        app_core_chain_endpoints()
            .expect("catalog")
            .into_iter()
            .find(|entry| entry.chain_name == chain_name)
            .map(|entry| entry.evm_rpc)
            .unwrap_or_default()
    }

    /// A testnet's records are filed under its mainnet's `chainName`, with the
    /// testnet named only in `groupTitle`. Both directions of that were wrong
    /// before: the testnet could not be looked up, and the mainnet's list
    /// included the testnet's endpoints.
    #[test]
    fn a_testnet_resolves_its_own_rpc_endpoints() {
        assert_eq!(
            rpc_endpoints("Ethereum Sepolia"),
            vec!["https://ethereum-sepolia-rpc.publicnode.com".to_string()]
        );
        assert_eq!(
            rpc_endpoints("Ethereum Hoodi"),
            vec!["https://ethereum-hoodi-rpc.publicnode.com".to_string()]
        );
    }

    #[test]
    fn a_mainnet_rpc_list_holds_no_testnet_endpoints() {
        for endpoint in rpc_endpoints("Ethereum") {
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
        let catalog = app_core_catalog().expect("catalog");
        let titles: Vec<String> = grouped_settings_entries(catalog, "Bitcoin")
            .into_iter()
            .map(|entry| entry.title)
            .collect();
        for expected in ["Bitcoin Testnet", "Bitcoin Testnet4", "Bitcoin Signet"] {
            assert!(titles.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn every_testnet_group_is_reachable_by_its_own_name() {
        let catalog = app_core_catalog().expect("catalog");
        for record in &catalog.endpoint_records {
            if record.group_title.is_empty() || record.group_title == record.chain_name {
                continue;
            }
            let found = endpoint_records_for_chain(catalog, &record.group_title, 0, false);
            assert!(
                found.iter().any(|other| other.id == record.id),
                "{} is not reachable as {}",
                record.id,
                record.group_title
            );
        }
    }
}

#[cfg(test)]
mod supplemental_endpoints_are_data {
    use crate::registry::{Chain, EndpointSlot};

    fn supplemental(chain: Chain) -> Vec<String> {
        super::app_core_chain_endpoints()
            .unwrap_or_default()
            .into_iter()
            .find(|c| c.chain_name == chain.chain_display_name())
            .map(|c| c.explorer_supplemental)
            .unwrap_or_default()
    }

    /// Which chains have a supplement is the catalog's answer, not a list.
    ///
    /// The front end held sixteen names. Twelve of them have no supplement at
    /// all, so those entries registered nothing; Hyperliquid had one and was
    /// not named, so its endpoints never reached the service.
    ///
    /// Hyperliquid's supplement was `api.hyperevmscan.io`, which Etherscan has
    /// since shut down along with the rest of its V1 family — the record is
    /// deleted and the chain reads its history through Etherscan V2 now. The
    /// property under test is unchanged: the catalog decides, and a chain with
    /// a supplement gets it without being named anywhere.
    #[test]
    fn a_supplement_comes_from_the_catalog_and_most_chains_have_none() {
        assert_eq!(
            supplemental(Chain::Ethereum),
            vec!["https://api.ethplorer.io".to_string()],
            "Ethereum's supplement is catalog data and was not in the sixteen-name table either"
        );
        assert!(supplemental(Chain::Hyperliquid).is_empty());
        for chain in [
            Chain::Arbitrum,
            Chain::Optimism,
            Chain::Base,
            Chain::Polygon,
            Chain::Linea,
            Chain::Scroll,
            Chain::Blast,
            Chain::Mantle,
            Chain::Avalanche,
            Chain::Near,
            Chain::Tron,
            Chain::EthereumClassic,
        ] {
            assert!(
                supplemental(chain).is_empty(),
                "{} has a supplement after all; the table was not as inert as it looked",
                chain.chain_display_name()
            );
        }
    }

    /// Where a supplement lands is a registry column, and only two chains
    /// differ: Polkadot's and ICP's are a working API the send path queries
    /// (Subscan, the ICP dashboard), so they go in `Secondary` rather than
    /// `Explorer`.
    #[test]
    fn only_polkadot_and_icp_use_the_secondary_slot() {
        for chain in Chain::all() {
            let expected = match chain.mainnet_counterpart() {
                Chain::Polkadot | Chain::Icp => EndpointSlot::Secondary,
                _ => EndpointSlot::Explorer,
            };
            assert_eq!(
                chain.supplemental_endpoint_slot(),
                expected,
                "{} put its supplement in the wrong slot",
                chain.chain_display_name()
            );
        }
    }

    /// Every chain with a supplement is reachable, because the loop walks the
    /// registry rather than a table.
    #[test]
    fn every_chain_with_a_supplement_has_a_slot_to_put_it_in() {
        let mut found = 0;
        for chain in Chain::all() {
            if supplemental(chain).is_empty() {
                continue;
            }
            found += 1;
            let slot_id = chain.endpoint_str_id(chain.supplemental_endpoint_slot());
            assert!(
                slot_id.contains(':'),
                "{} would write its supplement over its own primary endpoints",
                chain.chain_display_name()
            );
        }
        assert!(found > 0, "no chain has a supplemental endpoint at all");
    }
}

#[cfg(test)]
mod an_endpoints_kind_is_not_its_capabilities {
    use crate::registry::Chain;

    fn records() -> Vec<super::AppCoreEndpointRecord> {
        toml::from_str::<super::TomlEndpointFile>(super::APP_ENDPOINT_DIRECTORY_TOML)
            .expect("directory parses")
            .endpoints
            .into_iter()
            .map(super::AppCoreEndpointRecord::from)
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
            let Some(chain) = Chain::from_display_name(&record.chain_name) else {
                continue;
            };
            if !chain.is_evm() || record.kind != "rpc-node" {
                continue;
            }
            assert!(
                !record.capabilities.iter().any(|c| c == "history"),
                "{} {} is an EVM node and cannot serve address history",
                record.chain_name,
                record.endpoint
            );
        }
    }

    /// Every record says what it is, and says it with one of the four names
    /// the readers switch on.
    #[test]
    fn every_record_has_a_kind_the_readers_understand() {
        for record in records() {
            assert!(
                matches!(
                    record.kind.as_str(),
                    "rpc-node" | "indexer" | "web-link" | "backend"
                ),
                "{} {} has kind {:?}",
                record.chain_name,
                record.endpoint,
                record.kind
            );
        }
    }

    /// A kind is never a capability and a capability is never a kind. Holding
    /// both in one array is what let the `rpc` marker go missing on ten chains
    /// while their capabilities looked complete.
    #[test]
    fn the_two_vocabularies_do_not_overlap() {
        const CAPABILITIES: [&str; 7] = [
            "read",
            "balance",
            "history",
            "utxo",
            "fee",
            "broadcast",
            "verification",
        ];
        for record in records() {
            assert!(
                !CAPABILITIES.contains(&record.kind.as_str()),
                "{}",
                record.kind
            );
            for capability in &record.capabilities {
                assert!(
                    CAPABILITIES.contains(&capability.as_str()),
                    "{} {} lists {capability:?} as a capability",
                    record.chain_name,
                    record.endpoint
                );
            }
        }
    }

    /// Each of the eleven role names owns a bit no other name owns.
    ///
    /// Both vocabularies share one `u32` mask, and a repeated shift amount is
    /// invisible: the constants still compile, every filter still returns
    /// endpoints, and the only symptom is a mask quietly matching a kind it
    /// never asked for. It has happened twice — `explorer`/`indexer` first,
    /// then `indexer`/`backend`, where `catalog_endpoints`' `RPC | BALANCE |
    /// BACKEND` collected every indexer as well. Asserting it here costs one
    /// test and removes the third occurrence.
    #[test]
    fn every_role_name_owns_a_distinct_bit() {
        use std::collections::HashMap;

        // The two vocabularies of `data/endpoints.toml`, pinned against the
        // data by `every_record_has_a_kind_the_readers_understand` and
        // `the_two_vocabularies_do_not_overlap` above.
        const ROLES: [&str; 11] = [
            "rpc-node",
            "indexer",
            "web-link",
            "backend",
            "read",
            "balance",
            "history",
            "utxo",
            "fee",
            "broadcast",
            "verification",
        ];

        let mut owner: HashMap<u32, &str> = HashMap::new();
        for role in ROLES {
            let bit = super::endpoint_role_bit(role);
            assert_ne!(bit, 0, "{role:?} maps to no bit");
            assert!(bit.is_power_of_two(), "{role:?} is {bit:#x}, not one bit");
            if let Some(other) = owner.insert(bit, role) {
                panic!("{role:?} and {other:?} share bit {bit:#x}");
            }
        }

        // The aliases are the one place two names may share a bit: they are
        // the older spellings of a role, not roles of their own.
        assert_eq!(
            super::endpoint_role_bit("rpc"),
            super::endpoint_role_bit("rpc-node")
        );
        assert_eq!(
            super::endpoint_role_bit("explorer"),
            super::endpoint_role_bit("web-link")
        );

        // A name the catalog never uses claims nothing, rather than
        // defaulting onto some other role's bit.
        assert_eq!(super::endpoint_role_bit("not-a-role"), 0);
    }

    /// A web link is a URL for a person, so it claims nothing.
    #[test]
    fn a_web_link_has_no_capabilities() {
        for record in records().iter().filter(|r| r.kind == "web-link") {
            assert!(
                record.capabilities.is_empty(),
                "{} {} is a link and claims {:?}",
                record.chain_name,
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
    fn a_catalog_endpoint_reports_its_kind_and_capabilities() {
        let node = super::app_core_endpoint_tag("https://ethereum-rpc.publicnode.com".into())
            .expect("Ethereum's node is in the catalog");
        assert_eq!(node.kind, "rpc-node");
        assert!(node.capabilities.contains(&"balance".to_string()));
        assert!(
            !node.capabilities.contains(&"history".to_string()),
            "an EVM node cannot serve address history"
        );

        let indexer = super::app_core_endpoint_tag("https://eth.blockscout.com".into())
            .expect("Ethereum's indexer is in the catalog");
        assert_eq!(indexer.kind, "indexer");
        assert!(indexer.capabilities.contains(&"history".to_string()));
    }

    /// A trailing slash is not a different endpoint.
    #[test]
    fn the_lookup_ignores_a_trailing_slash() {
        assert_eq!(
            super::app_core_endpoint_tag("https://eth.blockscout.com/".into()),
            super::app_core_endpoint_tag("https://eth.blockscout.com".into()),
        );
    }

    /// An endpoint the user typed has no catalog row, and saying so beats
    /// guessing a kind for it.
    #[test]
    fn an_endpoint_the_catalog_does_not_list_has_no_tag() {
        assert!(super::app_core_endpoint_tag("https://my-own-node.example".into()).is_none());
    }
}
