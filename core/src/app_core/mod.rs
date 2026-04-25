use crate::store::wallet_domain::CoreSeedDerivationPaths;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

mod derivation_paths;
mod registry_data;

pub(crate) use derivation_paths::{
    compile_script_type, derivation_path_segment_value, derivation_path_string,
    normalize_derivation_path, parse_derivation_path,
};

const CHAIN_PRESETS_JSON: &str = include_str!("../../data/DerivationPresets.json");
const REQUEST_COMPILATION_PRESETS_JSON: &str =
    include_str!("../../data/DerivationRequestCompilationPresets.json");
const APP_ENDPOINT_DIRECTORY_JSON: &str = include_str!("../../data/AppEndpointDirectory.json");

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreChainPreset {
    pub chain: String,
    pub curve: String,
    pub networks: Vec<AppCoreNetworkPreset>,
    pub derivation_paths: Vec<AppCorePathPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCoreNetworkPreset {
    pub network: String,
    pub title: String,
    pub detail: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppCorePathPreset {
    pub title: String,
    pub detail: String,
    pub derivation_path: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AppCoreScriptPolicy {
    BitcoinPurpose,
    Fixed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AppCoreDerivationAlgorithm {
    Bip32Secp256k1,
    Slip10Ed25519,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AppCoreAddressAlgorithm {
    Bitcoin,
    Evm,
    Solana,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AppCorePublicKeyFormat {
    Compressed,
    Uncompressed,
    XOnly,
    Raw,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AppCoreScriptType {
    P2pkh,
    P2shP2wpkh,
    P2wpkh,
    P2tr,
    Account,
}

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
pub struct AppCoreRequestCompilationPreset {
    pub chain: String,
    pub derivation_algorithm: AppCoreDerivationAlgorithm,
    pub address_algorithm: AppCoreAddressAlgorithm,
    pub public_key_format: AppCorePublicKeyFormat,
    pub script_policy: AppCoreScriptPolicy,
    pub fixed_script_type: Option<AppCoreScriptType>,
    pub bitcoin_purpose_script_map: Option<std::collections::HashMap<String, AppCoreScriptType>>,
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
struct AppCoreCatalog {
    chain_presets: Vec<AppCoreChainPreset>,
    request_compilation_presets: Vec<AppCoreRequestCompilationPreset>,
    endpoint_records: Vec<AppCoreEndpointRecord>,
    /// Parallel to `endpoint_records`: pre-computed bitmask per record so the
    /// hot-path filter avoids per-call string matching on `roles`.
    endpoint_role_masks: Vec<u32>,
    /// Pre-indexed `chain_name` → record-index list. Eliminates the linear
    /// scan done on every `endpoint_records_for_chain` lookup.
    endpoint_records_by_chain: std::collections::HashMap<String, Vec<usize>>,
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
    pub short_label: String,
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
pub fn app_core_chain_presets() -> Result<Vec<AppCoreChainPreset>, crate::SpectraBridgeError> {
    Ok(app_core_catalog()?.chain_presets.clone())
}

#[uniffi::export]
pub fn app_core_request_compilation_presets(
) -> Result<Vec<AppCoreRequestCompilationPreset>, crate::SpectraBridgeError> {
    Ok(app_core_catalog()?.request_compilation_presets.clone())
}

#[uniffi::export]
pub fn app_core_resolve_derivation_path(
    chain: String,
    derivation_path: String,
) -> Result<AppCoreDerivationPathResolution, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    let default_path = derivation_paths::default_path_from_catalog(catalog, &chain)?;
    let normalized_path = normalize_derivation_path(&derivation_path, &default_path);
    Ok(AppCoreDerivationPathResolution {
        chain: chain.clone(),
        normalized_path: normalized_path.clone(),
        account_index: derivation_paths::resolved_account_index(&chain, &normalized_path),
        flavor: derivation_paths::resolved_flavor(&chain, &normalized_path).to_string(),
    })
}

#[uniffi::export]
pub fn app_core_derivation_paths_for_preset(
    account_index: u32,
) -> Result<CoreSeedDerivationPaths, crate::SpectraBridgeError> {
    Ok(app_core_catalog().and_then(|catalog| {
        derivation_paths::seed_derivation_paths_for_account(catalog, account_index)
    })?)
}

#[uniffi::export]
pub fn app_core_endpoint_records_for_chain(
    chain_name: String,
    role_mask: u32,
    settings_visible_only: bool,
) -> Result<Vec<AppCoreEndpointRecord>, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    Ok(endpoint_records_for_chain(catalog, &chain_name, role_mask, settings_visible_only))
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
    Ok(app_core_catalog()
        .and_then(|catalog| registry_data::bitcoin_esplora_base_urls(catalog, &network))?)
}

#[uniffi::export]
pub fn app_core_bitcoin_wallet_store_default_base_urls(
    network: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().and_then(|catalog| {
        registry_data::bitcoin_wallet_store_default_base_urls(catalog, &network)
    })?)
}

#[uniffi::export]
pub fn app_core_evm_rpc_endpoints(
    chain_name: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    Ok(endpoint_records_for_chain(catalog, &chain_name, ENDPOINT_ROLE_RPC, false)
        .into_iter()
        .map(|r| r.endpoint)
        .collect())
}

#[uniffi::export]
pub fn app_core_explorer_supplemental_endpoints(
    chain_name: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    Ok(endpoint_records_for_chain(catalog, &chain_name, ENDPOINT_ROLE_EXPLORER, true)
        .into_iter()
        .map(|r| r.endpoint)
        .collect())
}

#[uniffi::export]
pub fn app_core_broadcast_provider_options(
    chain_name: String,
) -> Vec<AppCoreBroadcastProviderOption> {
    registry_data::broadcast_provider_options(&chain_name)
}

#[uniffi::export]
pub fn app_core_chain_backends() -> Vec<AppCoreChainBackend> {
    registry_data::chain_backends()
}

#[uniffi::export]
pub fn app_core_live_chain_names() -> Vec<String> {
    registry_data::live_chain_names()
}

#[uniffi::export]
pub fn app_core_app_chain_descriptors() -> Vec<AppCoreAppChainDescriptor> {
    registry_data::app_chain_descriptors()
}

// ── Internals ─────────────────────────────────────────────────────────────

fn app_core_catalog() -> Result<&'static AppCoreCatalog, String> {
    match APP_CORE_CATALOG.get_or_init(load_app_core_catalog) {
        Ok(catalog) => Ok(catalog),
        Err(message) => Err(message.clone()),
    }
}

fn load_app_core_catalog() -> Result<AppCoreCatalog, String> {
    let display_error = |e: serde_json::Error| e.to_string();
    let chain_presets =
        serde_json::from_str::<Vec<AppCoreChainPreset>>(CHAIN_PRESETS_JSON).map_err(display_error)?;
    let request_compilation_presets = serde_json::from_str::<Vec<AppCoreRequestCompilationPreset>>(
        REQUEST_COMPILATION_PRESETS_JSON,
    )
    .map_err(display_error)?;
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
    let mut endpoint_records_by_chain: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, record) in endpoint_records.iter().enumerate() {
        endpoint_records_by_chain
            .entry(record.chain_name.clone())
            .or_default()
            .push(idx);
    }
    Ok(AppCoreCatalog {
        chain_presets,
        request_compilation_presets,
        endpoint_records,
        endpoint_role_masks,
        endpoint_records_by_chain,
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
    let Some(indices) = catalog.endpoint_records_by_chain.get(chain_name) else {
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
    let visible_records = endpoint_records_for_chain(catalog, chain_name, 0, true);
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
    fn loads_chain_presets_catalog() {
        let catalog = app_core_catalog().expect("catalog");
        assert!(catalog.chain_presets.iter().any(|p| p.chain == "Bitcoin"));
        assert!(catalog
            .request_compilation_presets
            .iter()
            .any(|p| p.chain == "Ethereum"));
    }

    #[test]
    fn resolves_bitcoin_taproot_path() {
        let default_path = derivation_paths::default_path_for_chain("Bitcoin").expect("default path");
        let normalized = normalize_derivation_path("m/86'/0'/2'/0/0", &default_path);
        assert_eq!(normalized, "m/86'/0'/2'/0/0");
        assert_eq!(derivation_paths::resolved_account_index("Bitcoin", &normalized), 2);
        assert_eq!(derivation_paths::resolved_flavor("Bitcoin", &normalized), "taproot");
    }

    #[test]
    fn preserves_bitcoin_sv_default_path_for_preset_accounts() {
        let catalog = app_core_catalog().expect("catalog");
        let paths = derivation_paths::seed_derivation_paths_for_account(catalog, 2).expect("paths");
        assert_eq!(paths.bitcoin_sv, "m/44'/236'/0'/0/0");
        assert_eq!(paths.ethereum, "m/44'/60'/2'/0/0");
        assert_eq!(paths.solana, "m/44'/501'/2'/0'");
    }
}
