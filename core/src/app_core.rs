use crate::store::wallet_domain::CoreSeedDerivationPaths;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const APP_ENDPOINT_DIRECTORY_JSON: &str = include_str!("../data/AppEndpointDirectory.json");

const ENDPOINT_ROLE_READ: u32 = 1 << 0;
const ENDPOINT_ROLE_BALANCE: u32 = 1 << 1;
const ENDPOINT_ROLE_HISTORY: u32 = 1 << 2;
const ENDPOINT_ROLE_UTXO: u32 = 1 << 3;
const ENDPOINT_ROLE_FEE: u32 = 1 << 4;
const ENDPOINT_ROLE_BROADCAST: u32 = 1 << 5;
const ENDPOINT_ROLE_VERIFICATION: u32 = 1 << 6;
const ENDPOINT_ROLE_RPC: u32 = 1 << 7;
const ENDPOINT_ROLE_EXPLORER: u32 = 1 << 8;
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
    pub(crate) endpoint_records_by_settings_chain:
        std::collections::HashMap<String, Vec<usize>>,
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
    pub roles: Vec<String>,
    #[serde(rename = "probeURL")]
    pub probe_url: Option<String>,
    pub settings_visible: bool,
    pub explorer_label: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreExplorerEntry {
    pub endpoint: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreBroadcastProviderOption {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
pub enum AppCoreChainIntegrationState {
    Live,
    Planned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreChainBackend {
    pub chain_name: String,
    pub supported_symbols: Vec<String>,
    pub integration_state: AppCoreChainIntegrationState,
    pub supports_seed_import: bool,
    pub supports_balance_refresh: bool,
    pub supports_receive_address: bool,
    pub supports_send: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreAppChainDescriptor {
    pub id: String,
    pub chain_name: String,
    pub native_symbol: String,
    pub search_keywords: Vec<String>,
    pub supports_diagnostics: bool,
    pub supports_endpoint_catalog: bool,
    pub is_evm: bool,
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

#[uniffi::export]
pub fn app_core_endpoint_records_for_chain(
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

#[uniffi::export]
pub fn app_core_endpoint_for_id(id: String) -> Result<String, crate::SpectraBridgeError> {
    Ok(app_core_catalog().and_then(|catalog| {
        catalog
            .endpoint_records
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.endpoint.clone())
            .ok_or_else(|| format!("Missing endpoint record for id: {id}"))
    })?)
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

#[uniffi::export]
pub fn app_core_grouped_settings_entries(
    chain_name: String,
) -> Result<Vec<AppCoreGroupedSettingsEntry>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().map(|catalog| grouped_settings_entries(catalog, &chain_name))?)
}

#[uniffi::export]
pub fn app_core_diagnostics_checks(
    chain_name: String,
) -> Result<Vec<AppCoreDiagnosticsCheck>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().map(|catalog| diagnostics_checks(catalog, &chain_name))?)
}

#[uniffi::export]
pub fn app_core_transaction_explorer_entry(
    chain_name: String,
) -> Result<Option<AppCoreExplorerEntry>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().map(|catalog| transaction_explorer_entry(catalog, &chain_name))?)
}

#[uniffi::export]
pub fn app_core_bitcoin_esplora_base_urls(
    network: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().and_then(|catalog| bitcoin_esplora_base_urls(catalog, &network))?)
}

#[uniffi::export]
pub fn app_core_bitcoin_wallet_store_default_base_urls(
    network: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    Ok(app_core_catalog()
        .and_then(|catalog| bitcoin_wallet_store_default_base_urls(catalog, &network))?)
}

#[uniffi::export]
pub fn app_core_evm_rpc_endpoints(
    chain_name: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    Ok(
        endpoint_records_for_chain(catalog, &chain_name, ENDPOINT_ROLE_RPC, false)
            .into_iter()
            .map(|r| r.endpoint)
            .collect(),
    )
}

#[uniffi::export]
pub fn app_core_explorer_supplemental_endpoints(
    chain_name: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    Ok(
        endpoint_records_for_chain(catalog, &chain_name, ENDPOINT_ROLE_EXPLORER, true)
            .into_iter()
            .map(|r| r.endpoint)
            .collect(),
    )
}

#[uniffi::export]
pub fn app_core_broadcast_provider_options(
    chain_name: String,
) -> Vec<AppCoreBroadcastProviderOption> {
    broadcast_provider_options(&chain_name)
}

#[uniffi::export]
pub fn app_core_chain_backends() -> Vec<AppCoreChainBackend> {
    chain_backends()
}

#[uniffi::export]
pub fn app_core_live_chain_names() -> Vec<String> {
    live_chain_names()
}

#[uniffi::export]
pub fn app_core_app_chain_descriptors() -> Vec<AppCoreAppChainDescriptor> {
    app_chain_descriptors()
}

// ── Internals ─────────────────────────────────────────────────────────────

pub(crate) fn app_core_catalog() -> Result<&'static AppCoreCatalog, String> {
    match APP_CORE_CATALOG.get_or_init(load_app_core_catalog) {
        Ok(catalog) => Ok(catalog),
        Err(message) => Err(message.clone()),
    }
}

fn load_app_core_catalog() -> Result<AppCoreCatalog, String> {
    let display_error = |e: serde_json::Error| e.to_string();
    let endpoint_records =
        serde_json::from_str::<Vec<AppCoreEndpointRecord>>(APP_ENDPOINT_DIRECTORY_JSON)
            .map_err(display_error)?;
    let endpoint_role_masks: Vec<u32> = endpoint_records
        .iter()
        .map(|r| {
            r.roles
                .iter()
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
        "rpc" => ENDPOINT_ROLE_RPC,
        "explorer" => ENDPOINT_ROLE_EXPLORER,
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

fn diagnostics_checks(catalog: &AppCoreCatalog, chain_name: &str) -> Vec<AppCoreDiagnosticsCheck> {
    endpoint_records_for_chain(catalog, chain_name, 0, false)
        .into_iter()
        .filter_map(|record| {
            record.probe_url.map(|probe_url| AppCoreDiagnosticsCheck {
                endpoint: record.endpoint,
                probe_url,
            })
        })
        .collect()
}

fn transaction_explorer_entry(
    catalog: &AppCoreCatalog,
    chain_name: &str,
) -> Option<AppCoreExplorerEntry> {
    endpoint_records_for_chain(catalog, chain_name, ENDPOINT_ROLE_EXPLORER, false)
        .into_iter()
        .find_map(|record| {
            record.explorer_label.map(|label| AppCoreExplorerEntry {
                endpoint: record.endpoint,
                label,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

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

/// Build the full transaction-explorer URL for a chain. Encapsulates the
/// per-chain URL format (Aptos appends `?network=mainnet`, every other chain
/// just concatenates the hash to the base URL). Returns `None` when the chain
/// has no explorer entry.
#[uniffi::export]
pub fn core_transaction_explorer_url(
    chain_name: String,
    transaction_hash: String,
) -> Result<Option<String>, crate::SpectraBridgeError> {
    let entry = app_core_transaction_explorer_entry(chain_name.clone())?;
    Ok(entry.map(|e| {
        if chain_name == "Aptos" {
            format!("{}{transaction_hash}?network=mainnet", e.endpoint)
        } else {
            format!("{}{transaction_hash}", e.endpoint)
        }
    }))
}

#[uniffi::export]
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
        // `display_names_match_the_catalog` now guarantees the names agree too.
        if let Some(template) = crate::chains::default_derivation_path_template_by_id(chain.str_id())
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
            crate::chains::default_derivation_path_template_by_id(chain.mainnet_counterpart().str_id())
        })
    });
    template
        .map(|template| render_derivation_path_template(template, account))
        .ok_or_else(|| format!("Missing default derivation path for {chain_name}."))
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

/// Key under which a chain's derivation path is stored in
/// `CoreSeedDerivationPaths::by_chain`.
///
/// Testnets resolve to their mainnet counterpart: the derivation recipe is
/// identical and only the address encoding differs, so both share one stored
/// path. Unknown names return an empty string.
#[uniffi::export]
pub fn core_seed_derivation_path_key(chain_name: String) -> String {
    crate::registry::Chain::from_display_name(&chain_name)
        .map(|chain| chain.mainnet_counterpart().str_id().to_string())
        .unwrap_or_default()
}



#[uniffi::export]
pub fn core_derivation_path_replacing_last_two(
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

// ── chain_backends ─────────────────────────────────────────────────────────
// Most chains are Live with full feature support, so the `live(...)` builder
// captures the default and lets each entry collapse to a single line.

fn live(name: &str, symbols: &[&str]) -> AppCoreChainBackend {
    AppCoreChainBackend {
        chain_name: name.to_string(),
        supported_symbols: symbols.iter().map(|s| s.to_string()).collect(),
        integration_state: AppCoreChainIntegrationState::Live,
        supports_seed_import: true,
        supports_balance_refresh: true,
        supports_receive_address: true,
        supports_send: true,
    }
}

const TRACKED_ERC20: &str = "Tracked ERC-20s";

pub(super) fn chain_backends() -> Vec<AppCoreChainBackend> {
    vec![
        live("Bitcoin", &["BTC"]),
        live("Bitcoin Cash", &["BCH"]),
        live("Bitcoin SV", &["BSV"]),
        live("Litecoin", &["LTC"]),
        live("Ethereum", &["ETH", "USDT", "USDC", "DAI"]),
        live("Arbitrum", &["ETH", TRACKED_ERC20]),
        live("Optimism", &["ETH", TRACKED_ERC20]),
        live("Ethereum Classic", &["ETC"]),
        live("Dogecoin", &["DOGE"]),
        live("BNB Chain", &["BNB"]),
        live("Avalanche", &["AVAX"]),
        live("Hyperliquid", &["HYPE", TRACKED_ERC20]),
        live("Tron", &["TRX", "USDT"]),
        live("Solana", &["SOL"]),
        live("XRP Ledger", &["XRP"]),
        live("Monero", &["XMR"]),
        live("Cardano", &["ADA"]),
        live("Sui", &["SUI"]),
        live("Aptos", &["APT"]),
        live("TON", &["TON", "Tracked Jettons"]),
        live("Internet Computer", &["ICP"]),
        live("NEAR", &["NEAR"]),
        live("Polkadot", &["DOT"]),
        live("Stellar", &["XLM"]),
        live("Polygon", &["POL", TRACKED_ERC20]),
        live("Base", &["ETH", TRACKED_ERC20]),
        live("Linea", &["ETH", TRACKED_ERC20]),
        live("Scroll", &["ETH", TRACKED_ERC20]),
        live("Blast", &["ETH", TRACKED_ERC20]),
        live("Mantle", &["MNT", TRACKED_ERC20]),
        live("Zcash", &["ZEC"]),
        live("Bitcoin Gold", &["BTG"]),
        live("Decred", &["DCR"]),
        live("Kaspa", &["KAS"]),
        live("Dash", &["DASH"]),
        live("Sei", &["SEI", TRACKED_ERC20]),
        live("Celo", &["CELO", TRACKED_ERC20]),
        live("Cronos", &["CRO", TRACKED_ERC20]),
        live("opBNB", &["BNB", TRACKED_ERC20]),
        live("zkSync Era", &["ETH", TRACKED_ERC20]),
        live("Sonic", &["S", TRACKED_ERC20]),
        live("Berachain", &["BERA", TRACKED_ERC20]),
        live("Unichain", &["ETH", TRACKED_ERC20]),
        live("Ink", &["ETH", TRACKED_ERC20]),
        live("X Layer", &["OKB", TRACKED_ERC20]),
        live("Bittensor", &["TAO"]),
        // ── Testnets ────────────────────────────────────────────────────
        // Each testnet is its own first-class chain row. Same supported
        // symbols as its mainnet counterpart (the asset is logically
        // the same — only the chain row makes clear it isn't real money).
        live("Bitcoin Testnet", &["BTC"]),
        live("Bitcoin Testnet4", &["BTC"]),
        live("Bitcoin Signet", &["BTC"]),
        live("Litecoin Testnet", &["LTC"]),
        live("Bitcoin Cash Testnet", &["BCH"]),
        live("Bitcoin SV Testnet", &["BSV"]),
        live("Dogecoin Testnet", &["DOGE"]),
        live("Zcash Testnet", &["ZEC"]),
        live("Decred Testnet", &["DCR"]),
        live("Kaspa Testnet", &["KAS"]),
        live("Dash Testnet", &["DASH"]),
        live("Ethereum Sepolia", &["ETH", TRACKED_ERC20]),
        live("Ethereum Hoodi", &["ETH", TRACKED_ERC20]),
        live("Arbitrum Sepolia", &["ETH", TRACKED_ERC20]),
        live("Optimism Sepolia", &["ETH", TRACKED_ERC20]),
        live("Base Sepolia", &["ETH", TRACKED_ERC20]),
        live("BNB Chain Testnet", &["BNB", TRACKED_ERC20]),
        live("Avalanche Fuji", &["AVAX", TRACKED_ERC20]),
        live("Polygon Amoy", &["POL", TRACKED_ERC20]),
        live("Hyperliquid Testnet", &["HYPE", TRACKED_ERC20]),
        live("Ethereum Classic Mordor", &["ETC"]),
        live("Tron Nile", &["TRX"]),
        live("Solana Devnet", &["SOL"]),
        live("XRP Ledger Testnet", &["XRP"]),
        live("Stellar Testnet", &["XLM"]),
        live("Cardano Preprod", &["ADA"]),
        live("Sui Testnet", &["SUI"]),
        live("Aptos Testnet", &["APT"]),
        live("TON Testnet", &["TON"]),
        live("NEAR Testnet", &["NEAR"]),
        live("Polkadot Westend", &["DOT"]),
        live("Monero Stagenet", &["XMR"]),
    ]
}

pub(super) fn live_chain_names() -> Vec<String> {
    chain_backends()
        .into_iter()
        .filter(|b| matches!(b.integration_state, AppCoreChainIntegrationState::Live))
        .map(|b| b.chain_name)
        .collect()
}

// ── app_chain_descriptors ─────────────────────────────────────────────────

pub(super) fn app_chain_descriptors() -> Vec<AppCoreAppChainDescriptor> {
    crate::chains::catalog()
        .iter()
        .map(|c| AppCoreAppChainDescriptor {
            id: c.id.clone(),
            chain_name: c.name.clone(),
            native_symbol: c.gas_token_symbol.clone(),
            search_keywords: c.search_keywords.clone(),
            supports_diagnostics: c.supports_diagnostics,
            supports_endpoint_catalog: c.supports_endpoint_catalog,
            is_evm: c.is_evm,
        })
        .collect()
}

// ── broadcast_provider_options ─────────────────────────────────────────────

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
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche"
        | "Hyperliquid" | "Polygon" | "Base" | "Linea" | "Scroll" | "Blast" | "Mantle" => {
            &[("rpc", "RPC Broadcast")]
        }
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
        "Sei" | "Celo" | "Cronos" | "opBNB" | "zkSync Era" | "Sonic" | "Berachain" | "Unichain"
        | "Ink" | "X Layer" => &[("rpc", "RPC Broadcast")],
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
    network: &str,
) -> Result<Vec<String>, String> {
    let ids: &[&str] = match network {
        "mainnet" => &[
            "bitcoin.mainnet.blockstream",
            "bitcoin.mainnet.mempool",
            "bitcoin.mainnet.mempool_emzy",
            "bitcoin.mainnet.maestro",
        ],
        "testnet" => &["bitcoin.testnet.blockstream", "bitcoin.testnet.mempool"],
        "testnet4" => &["bitcoin.testnet4.mempool"],
        "signet" => &["bitcoin.signet.blockstream", "bitcoin.signet.mempool"],
        _ => return Err(format!("Unsupported Bitcoin network mode: {network}")),
    };
    endpoints_for_known_ids(catalog, ids)
}

pub(super) fn bitcoin_wallet_store_default_base_urls(
    catalog: &AppCoreCatalog,
    network: &str,
) -> Result<Vec<String>, String> {
    let ids: &[&str] = match network {
        "mainnet" => &[
            "bitcoin.mainnet.blockstream",
            "bitcoin.mainnet.mempool",
            "bitcoin.mainnet.maestro",
        ],
        "testnet" => &["bitcoin.testnet.blockstream", "bitcoin.testnet.mempool"],
        "testnet4" => &["bitcoin.testnet4.mempool"],
        "signet" => &["bitcoin.signet.mempool"],
        _ => return Err(format!("Unsupported Bitcoin network mode: {network}")),
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
        let testnet = super::app_core_resolve_derivation_path(
            "Bitcoin Testnet4".to_string(),
            String::new(),
        )
        .expect("testnet4");
        let mainnet =
            super::app_core_resolve_derivation_path("Bitcoin".to_string(), String::new())
                .expect("bitcoin");
        assert_eq!(testnet.normalized_path, mainnet.normalized_path);
    }
}

#[cfg(test)]
mod endpoint_network_index_tests {
    use super::*;

    fn rpc_endpoints(chain_name: &str) -> Vec<String> {
        app_core_evm_rpc_endpoints(chain_name.to_string()).expect("catalog")
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
