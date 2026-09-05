use crate::store::wallet_domain::CoreSeedDerivationPaths;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const APP_ENDPOINT_DIRECTORY_TOML: &str = include_str!("../data/endpoints.toml");

const ENDPOINT_ROLE_READ: u32 = 1 << 0;
const ENDPOINT_ROLE_BALANCE: u32 = 1 << 1;
const ENDPOINT_ROLE_HISTORY: u32 = 1 << 2;
const ENDPOINT_ROLE_UTXO: u32 = 1 << 3;
const ENDPOINT_ROLE_FEE: u32 = 1 << 4;
const ENDPOINT_ROLE_BROADCAST: u32 = 1 << 5;
const ENDPOINT_ROLE_VERIFICATION: u32 = 1 << 6;
const ENDPOINT_ROLE_RPC: u32 = 1 << 7;
const ENDPOINT_ROLE_EXPLORER: u32 = 1 << 8;
/// An address-indexed API. Its own bit because an indexer is not a `/tx/`
/// link, and the two were sharing one — which is how `explorer_supplemental`
/// briefly picked up every Esplora endpoint Bitcoin has.
const ENDPOINT_ROLE_INDEXER: u32 = 1 << 9;
const ENDPOINT_ROLE_BACKEND: u32 = 1 << 9;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreDerivationPathResolution {
    pub chain: String,
    pub normalized_path: String,
    pub account_index: u32,
    pub flavor: String,
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
pub struct AppCoreDiagnosticsCheck {
    pub endpoint: String,
    #[serde(rename = "probeURL")]
    pub probe_url: String,
    /// Set when this endpoint carries the catalog's `rpc` role and its chain
    /// has a health method — probe it with this JSON-RPC call rather than a
    /// GET against `probe_url`.
    pub rpc_probe_method: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreBroadcastProviderOption {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
pub struct DerivationPathSegment {
    pub value: u32,
    pub is_hardened: bool,
}

static APP_CORE_CATALOG: OnceLock<Result<AppCoreCatalog, String>> = OnceLock::new();

// ── UniFFI exports ────────────────────────────────────────────────────────

#[uniffi::export]
pub fn app_core_resolve_derivation_path(
    chain: String,
    derivation_path: String,
) -> Result<AppCoreDerivationPathResolution, crate::SpectraBridgeError> {
    let default_path = default_path_from_catalog(&chain)?;
    let normalized_path = normalize_derivation_path(&derivation_path, &default_path);
    Ok(AppCoreDerivationPathResolution {
        chain: chain.clone(),
        normalized_path: normalized_path.clone(),
        account_index: resolved_account_index(&chain, &normalized_path),
        flavor: resolved_flavor(&chain, &normalized_path),
    })
}

#[uniffi::export]
pub fn app_core_derivation_paths_for_preset(
    account_index: u32,
) -> Result<CoreSeedDerivationPaths, crate::SpectraBridgeError> {
    Ok(seed_derivation_paths_for_account(account_index)?)
}

/// A chain's endpoint records, filtered to the roles asked for.
///
/// Takes role *names*. It used to take the bit mask, and the only way to get
/// one was `core_endpoint_role_mask` — so a caller made a round trip to turn
/// `["rpc", "balance"]` into a `u32` and handed the `u32` straight back.
/// Rust callers keep the mask constants; the boundary does not need them.
#[uniffi::export]
pub fn app_core_endpoint_records_for_chain(
    chain_name: String,
    roles: Vec<String>,
    settings_visible_only: bool,
) -> Result<Vec<AppCoreEndpointRecord>, crate::SpectraBridgeError> {
    endpoint_records_for_chain_masked(
        chain_name,
        core_endpoint_role_mask(roles),
        settings_visible_only,
    )
}

/// The same, for Rust callers that already hold the mask constants.
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
    pub diagnostics_checks: Vec<AppCoreDiagnosticsCheck>,
    pub transaction_explorer: Option<AppCoreExplorerEntry>,
    pub broadcast_providers: Vec<AppCoreBroadcastProviderOption>,
    /// Esplora bases, for the Bitcoin family. Empty elsewhere.
    pub bitcoin_esplora: Vec<String>,
    /// Wallet-store defaults, for the Bitcoin family. Empty elsewhere.
    pub bitcoin_wallet_store: Vec<String>,
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
                diagnostics_checks: diagnostics_checks(catalog, &name),
                transaction_explorer: transaction_explorer_entry(catalog, &name),
                broadcast_providers: broadcast_provider_options(&name),
                bitcoin_esplora: bitcoin_esplora_base_urls(catalog, &id).unwrap_or_default(),
                bitcoin_wallet_store: bitcoin_wallet_store_default_base_urls(catalog, &id)
                    .unwrap_or_default(),
                chain_id: id,
                chain_name: name,
            }
        })
        .collect())
}

#[uniffi::export]
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
    let endpoint_records =
        toml::from_str::<TomlEndpointFile>(APP_ENDPOINT_DIRECTORY_TOML)
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

/// Every endpoint of a chain that can be checked, and how to check it.
///
/// A record qualifies two ways: it carries a `probe_url` to GET, or it is an
/// RPC endpoint on a chain with a health method, in which case the JSON-RPC
/// call goes to the endpoint itself and no probe URL is needed.
///
/// That second clause is what merged the two endpoint-diagnostics paths. This
/// used to require a `probe_url`, and no EVM RPC record has one — so twelve
/// mainnets had an empty list here and were probed instead by
/// `evmEndpointChecks` in Swift, reading `evm_rpc` off the same catalog and
/// running its own `probeEthereumRPC`. One catalog, two slices, two probes.
fn diagnostics_checks(catalog: &AppCoreCatalog, chain_name: &str) -> Vec<AppCoreDiagnosticsCheck> {
    let health_method = crate::registry::Chain::from_display_name(chain_name)
        .and_then(crate::registry::Chain::rpc_health_method);
    endpoint_records_for_chain(catalog, chain_name, 0, false)
        .into_iter()
        .filter_map(|record| {
            let is_rpc = record.kind == "rpc-node";
            let rpc_probe_method = is_rpc.then_some(health_method).flatten().map(str::to_string);
            // An RPC endpoint is probed by POSTing to itself, so `probe_url`
            // stands in as the endpoint and the RPC branch ignores it.
            let probe_url = record
                .probe_url
                .or_else(|| rpc_probe_method.is_some().then(|| record.endpoint.clone()))?;
            Some(AppCoreDiagnosticsCheck {
                endpoint: record.endpoint,
                probe_url,
                rpc_probe_method,
            })
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
        assert_eq!(resolved_account_index("Bitcoin", &normalized), 2);
        assert_eq!(resolved_flavor("Bitcoin", &normalized), "taproot");
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

/// Not exported: the boundary takes role names, and this is how they become a
/// mask on this side.
pub fn core_endpoint_role_mask(roles: Vec<String>) -> u32 {
    roles
        .iter()
        .fold(0u32, |mask, role| mask | endpoint_role_bit(role))
}

// ── Merged from app_core_derivation_paths.rs ──────────────────────

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

pub(crate) fn derivation_path_segment_value(path: &str, index: usize) -> Option<u32> {
    parse_derivation_path(path).and_then(|segments| segments.get(index).map(|s| s.value))
}

pub(super) fn resolved_account_index(chain_name: &str, normalized_path: &str) -> u32 {
    match chain_name {
        "Bitcoin" if normalized_path == "m/0'/0" || normalized_path == "m/0'/0/0" => 0,
        "Bitcoin Cash" | "Bitcoin SV" if normalized_path == "m/0" => 0,
        _ => derivation_path_segment_value(normalized_path, 2).unwrap_or(0),
    }
}

pub(super) fn resolved_flavor(chain_name: &str, normalized_path: &str) -> String {
    let account = resolved_account_index(chain_name, normalized_path);
    crate::chains::derivation_paths_for_chain(chain_name)
        .and_then(|entries| {
            entries.iter().find_map(|entry| {
                let rendered = render_derivation_path_template(&entry.path, account);
                if normalize_derivation_path(&rendered, "") == normalized_path {
                    Some(entry.tag.clone())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "standard".to_string())
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


/// No longer on the FFI: its only Swift caller was
/// `AppState.utxoDiscoveryDerivationPath`, and address derivation moved into
/// `WalletService::discover_utxo_addresses`. Core still builds paths with it.
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

// ── Merged from app_core_registry_data.rs ─────────────────────────


pub(super) fn broadcast_provider_options(chain_name: &str) -> Vec<AppCoreBroadcastProviderOption> {
    let resolved = crate::registry::Chain::from_display_name(chain_name)
        .map(|c| c.mainnet_counterpart().chain_display_name())
        .unwrap_or(chain_name);
    let pairs: &[(&str, &str)] = match resolved {
        "Bitcoin" => &[
            ("esplora", "Esplora"),
            ("maestro-esplora", "Maestro Esplora"),
        ],
        "Bitcoin Cash" => &[
            ("blockchair", "Blockchair"),
            ("actorforth", "ActorForth REST"),
        ],
        "Bitcoin SV" => &[
            ("whatsonchain", "WhatsOnChain"),
            ("blockchair", "Blockchair"),
        ],
        "Litecoin" => &[
            ("litecoinspace", "LitecoinSpace"),
            ("blockcypher", "BlockCypher"),
        ],
        "Dogecoin" => &[("blockcypher", "BlockCypher")],
        "Tron" => &[
            ("trongrid-io", "TronGrid"),
            ("trongrid-pro", "TronGrid Pro"),
            ("trongrid-network", "TronGrid Network"),
        ],
        "Solana" => &[
            ("solana-mainnet-beta", "Solana Mainnet RPC"),
            ("solana-ankr", "Ankr Solana RPC"),
        ],
        "Cardano" => &[
            ("koios", "Koios"),
            ("xray-koios", "Xray Koios"),
            ("happystaking-koios", "HappyStake Koios"),
        ],
        "XRP Ledger" => &[
            ("ripple-s1", "Ripple RPC S1"),
            ("ripple-s2", "Ripple RPC S2"),
            ("xrplcluster", "XRPL Cluster"),
        ],
        "Stellar" => &[
            ("stellar-horizon", "Stellar Horizon"),
            ("lobstr-horizon", "LOBSTR Horizon"),
        ],
        "Monero" => &[
            ("edge-lws-1", "Edge Monero LWS 1"),
            ("edge-lws-2", "Edge Monero LWS 2"),
            ("edge-lws-3", "Edge Monero LWS 3"),
        ],
        "Sui" => &[
            ("sui-mainnet", "Sui Mainnet"),
            ("sui-publicnode", "PublicNode Sui"),
            ("sui-blockvision", "BlockVision Sui"),
            ("sui-blockpi", "BlockPI Sui"),
            ("sui-suiscan", "SuiScan RPC"),
        ],
        "Aptos" => &[
            ("aptoslabs-api", "Aptos Labs API"),
            ("blastapi-aptos", "BlastAPI Aptos"),
            ("aptoslabs-mainnet", "Aptos Mainnet"),
        ],
        "TON" => &[("ton-api-v2", "TON API v2")],
        "Internet Computer" => &[("rosetta", "Rosetta")],
        "NEAR" => &[
            ("near-mainnet-rpc", "NEAR Mainnet RPC"),
            ("fastnear-rpc", "FastNEAR RPC"),
            ("lava-near-rpc", "Lava NEAR RPC"),
        ],
        "Polkadot" => &[("sidecar", "Sidecar")],
        "Zcash" => &[("trezor-blockbook", "Trezor Blockbook")],
        "Bitcoin Gold" => &[("trezor-blockbook", "Trezor Blockbook")],
        "Decred" => &[("dcrdata", "dcrdata Insight")],
        "Kaspa" => &[("kaspaorg", "api.kaspa.org")],
        "Dash" => &[("trezor-blockbook", "Trezor Blockbook")],
        "Bittensor" => &[("opentensor", "OpenTensor RPC")],
        // Every EVM chain broadcasts through its RPC. This was two arms of
        // thirteen and ten names — the twenty-three mainnets split by whichever
        // half the author was looking at — and a twenty-fourth would have
        // reached `_ => &[]`: no broadcast provider at all.
        name if crate::registry::Chain::from_display_name(name).is_some_and(|c| c.is_evm()) => {
            &[("rpc", "RPC Broadcast")]
        }
        _ => &[],
    };
    pairs
        .iter()
        .map(|(id, title)| AppCoreBroadcastProviderOption {
            id: (*id).to_string(),
            title: (*title).to_string(),
        })
        .collect()
}

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

pub(super) fn bitcoin_wallet_store_default_base_urls(
    catalog: &AppCoreCatalog,
    chain_id: &str,
) -> Result<Vec<String>, String> {
    let ids: &[&str] = match chain_id {
        "bitcoin" => &[
            "bitcoin.mainnet.blockstream",
            "bitcoin.mainnet.mempool",
            "bitcoin.mainnet.maestro",
        ],
        "bitcoin-testnet" => &["bitcoin.testnet.blockstream", "bitcoin.testnet.mempool"],
        "bitcoin-testnet-4" => &["bitcoin.testnet4.mempool"],
        "bitcoin-signet" => &["bitcoin.signet.mempool"],
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
        assert_eq!(testnet.normalized_path, mainnet.normalized_path);
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
mod rpc_health_probes {
    /// Which endpoints get a JSON-RPC probe comes from the catalog's `rpc`
    /// role, not from a list in Swift.
    ///
    /// Two hand-written id lists decided this —
    /// `NearBalanceService.rpcEndpointCatalog` named three ids and
    /// `PolkadotBalanceService.sidecarEndpointCatalog` named one, tested
    /// inverted. Both agreed with the catalog when they were written. Adding a
    /// provider meant editing `endpoints.toml` and remembering the
    /// Swift list; forgetting it probes a JSON-RPC node with a GET, which many
    /// answer 405 and this reports as unreachable.
    #[test]
    fn the_catalog_decides_which_endpoints_are_rpc() {
        let catalog = super::load_app_core_catalog().expect("catalog");
        let mut checked_any = false;

        // Named endpoints used to stand here — `near.lava.build`, Polkadot's
        // `dotters` and `ibp.network` — and all three have since gone dead and
        // been removed, which broke a test whose subject was the rule and not
        // those hosts. The rule, asserted over whatever the catalog holds: a
        // record with the `rpc` role on a chain with a health method gets that
        // method, and a record without the role never does.
        for chain in crate::registry::Chain::all().filter(|c| !c.is_testnet()) {
            let Some(method) = chain.rpc_health_method() else {
                continue;
            };
            let name = chain.chain_display_name();
            let records = super::endpoint_records_for_chain(&catalog, name, 0, false);
            for check in super::diagnostics_checks(&catalog, name) {
                let Some(record) = records.iter().find(|r| r.endpoint == check.endpoint) else {
                    continue;
                };
                let is_rpc = record.kind == "rpc-node";
                checked_any = true;
                if is_rpc {
                    assert_eq!(
                        check.rpc_probe_method.as_deref(),
                        Some(method),
                        "{name} {} carries the rpc role and must be probed as one",
                        check.endpoint
                    );
                } else {
                    assert_eq!(
                        check.rpc_probe_method, None,
                        "{name} {} is not an rpc node and must keep its GET probe",
                        check.endpoint
                    );
                }
            }
        }
        assert!(checked_any, "no chain with a health method had any endpoint");
    }

    /// Every mainnet with catalog endpoints has something to check.
    ///
    /// `diagnostics_checks` used to require a `probe_url`, and no EVM RPC
    /// record has one, so twelve mainnets answered with an empty list —
    /// Arbitrum, Optimism, Avalanche, Base, Ethereum Classic, Hyperliquid,
    /// Polygon, Linea, Scroll, Blast, Mantle and Monero. Their endpoints
    /// screens were not blank, because Swift probed them down a second path
    /// (`evmEndpointChecks` over `evm_rpc`, with its own `probeEthereumRPC`
    /// and its own hardcoded explorer URLs). That path is gone; this is the
    /// assertion that keeps the one that replaced it honest.
    #[test]
    fn every_mainnet_with_endpoints_has_something_to_check() {
        let catalog = super::load_app_core_catalog().expect("catalog");
        for chain in crate::registry::Chain::all().filter(|c| !c.is_testnet()) {
            let name = chain.chain_display_name();
            // Monero is checked against the backend URL in settings, not the
            // catalog, so it is the one mainnet with no catalog endpoints.
            if chain == crate::registry::Chain::Monero {
                continue;
            }
            assert!(
                !super::diagnostics_checks(&catalog, name).is_empty(),
                "{name} has no endpoint diagnostics"
            );
        }
    }

    /// A chain with no health method never asks for a JSON-RPC probe.
    #[test]
    fn a_chain_without_a_health_method_probes_over_http() {
        let catalog = super::load_app_core_catalog().expect("catalog");
        for chain in crate::registry::Chain::all().filter(|c| c.rpc_health_method().is_none()) {
            for check in super::diagnostics_checks(&catalog, chain.chain_display_name()) {
                assert_eq!(
                    check.rpc_probe_method, None,
                    "{} {}",
                    chain.chain_display_name(),
                    check.endpoint
                );
            }
        }
    }
}



#[cfg(test)]
mod broadcast_provider_coverage {
    use super::broadcast_provider_options;
    use crate::registry::Chain;

    /// Every chain that can be sent from offers somewhere to broadcast it.
    ///
    /// The EVM arm was two hand-written halves; a chain outside both fell to
    /// `_ => &[]` and the send screen had no provider to name.
    #[test]
    fn every_sendable_chain_names_a_broadcast_provider() {
        for chain in Chain::all() {
            if !chain.has_send_preview() {
                continue;
            }
            let options = broadcast_provider_options(chain.chain_display_name());
            assert!(
                !options.is_empty(),
                "{} can be sent from but offers no broadcast provider",
                chain.chain_display_name()
            );
        }
    }

    /// A testnet folds onto its mainnet's providers rather than needing rows.
    #[test]
    fn a_testnet_reads_its_mainnets_providers() {
        assert_eq!(
            broadcast_provider_options("Ethereum Sepolia"),
            broadcast_provider_options("Ethereum")
        );
        assert_eq!(
            broadcast_provider_options("Bitcoin Testnet4"),
            broadcast_provider_options("Bitcoin")
        );
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
            assert!(!CAPABILITIES.contains(&record.kind.as_str()), "{}", record.kind);
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
