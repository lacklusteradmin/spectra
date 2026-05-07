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
    /// Pre-indexed `chain_name` → record-index list. Eliminates the linear
    /// scan done on every `endpoint_records_for_chain` lookup.
    pub(crate) endpoint_records_by_chain: std::collections::HashMap<String, Vec<usize>>,
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
        flavor: resolved_flavor(&chain, &normalized_path).to_string(),
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
        .and_then(|catalog| bitcoin_esplora_base_urls(catalog, &network))?)
}

#[uniffi::export]
pub fn app_core_bitcoin_wallet_store_default_base_urls(
    network: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().and_then(|catalog| {
        bitcoin_wallet_store_default_base_urls(catalog, &network)
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
    let mut endpoint_records_by_chain: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, record) in endpoint_records.iter().enumerate() {
        endpoint_records_by_chain
            .entry(record.chain_name.clone())
            .or_default()
            .push(idx);
    }
    Ok(AppCoreCatalog {
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
    fn resolves_bitcoin_taproot_path() {
        let default_path = default_path_for_chain("Bitcoin").expect("default path");
        let normalized = normalize_derivation_path("m/86'/0'/2'/0/0", &default_path);
        assert_eq!(normalized, "m/86'/0'/2'/0/0");
        assert_eq!(resolved_account_index("Bitcoin", &normalized), 2);
        assert_eq!(resolved_flavor("Bitcoin", &normalized), "taproot");
    }

    #[test]
    fn preserves_bitcoin_sv_default_path_for_preset_accounts() {
        let paths = seed_derivation_paths_for_account(2).expect("paths");
        assert_eq!(paths.bitcoin_sv, "m/44'/236'/0'/0/0");
        assert_eq!(paths.ethereum, "m/44'/60'/2'/0/0");
        assert_eq!(paths.solana, "m/44'/501'/2'/0'");
    }
}

// ── FFI surface (relocated from ffi.rs) ──────────────────────────────────

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

pub(super) fn resolved_flavor(chain_name: &str, normalized_path: &str) -> &'static str {
    match chain_name {
        "Bitcoin" => match normalized_path {
            p if p.starts_with("m/86'") => "taproot",
            p if p.starts_with("m/84'") => "nativeSegWit",
            p if p.starts_with("m/49'") => "nestedSegWit",
            "m/0'/0" | "m/0'/0/0" => "electrumLegacy",
            p if p.starts_with("m/44'") => "legacy",
            _ => "standard",
        },
        "Litecoin" => match normalized_path {
            p if p.starts_with("m/84'/2'") => "nativeSegWit",
            p if p.starts_with("m/49'/2'") => "nestedSegWit",
            p if p.starts_with("m/44'/2'") => "legacy",
            _ => "standard",
        },
        "Bitcoin Cash" => match normalized_path {
            "m/0" => "electrumLegacy",
            p if p.starts_with("m/44'/0'") || p.starts_with("m/44'/145'") => "legacy",
            _ => "standard",
        },
        "Solana" if normalized_path == "m/44'/501'/0'" => "legacy",
        "Cardano" if normalized_path.starts_with("m/44'/1815'") => "legacy",
        "Tron"
            if normalized_path == "m/44'/195'/0'"
                || normalized_path.starts_with("m/44'/60'") =>
        {
            "legacy"
        }
        "XRP Ledger" if normalized_path == "m/44'/144'/0'" => "legacy",
        _ => "standard",
    }
}

pub(super) fn seed_derivation_paths_for_account(
    account: u32,
) -> Result<CoreSeedDerivationPaths, String> {
    // SLIP-44 standard `m/44'/coin'/account'/0/0` is the most common shape;
    // a few chains diverge (Solana, Stellar, NEAR, Polkadot, Sui, Aptos).
    let evm = slip44(60, account);
    Ok(CoreSeedDerivationPaths {
        is_custom_enabled: false,
        bitcoin: format!("m/84'/0'/{account}'/0/0"),
        bitcoin_cash: format!("m/44'/145'/{account}'/0/0"),
        bitcoin_sv: default_path_from_catalog("Bitcoin SV")?,
        litecoin: slip44(2, account),
        dogecoin: slip44(3, account),
        ethereum: evm.clone(),
        ethereum_classic: slip44(61, account),
        arbitrum: evm.clone(),
        optimism: evm.clone(),
        avalanche: evm.clone(),
        hyperliquid: evm.clone(),
        polygon: evm.clone(),
        base: evm.clone(),
        linea: evm.clone(),
        scroll: evm.clone(),
        blast: evm.clone(),
        mantle: evm.clone(),
        tron: slip44(195, account),
        solana: format!("m/44'/501'/{account}'/0'"),
        stellar: format!("m/44'/148'/{account}'"),
        xrp: slip44(144, account),
        cardano: format!("m/1852'/1815'/{account}'/0/0"),
        sui: format!("m/44'/784'/{account}'/0'/0'"),
        aptos: format!("m/44'/637'/{account}'/0'/0'"),
        ton: slip44(607, account),
        internet_computer: slip44(223, account),
        near: format!("m/44'/397'/{account}'"),
        polkadot: format!("m/44'/354'/{account}'"),
        zcash: slip44(133, account),
        bitcoin_gold: slip44(156, account),
        // EVM L1/L2s share the EVM derivation path (SLIP-44 60).
        sei: evm.clone(),
        celo: evm.clone(),
        cronos: evm.clone(),
        op_bnb: evm.clone(),
        zksync_era: evm.clone(),
        sonic: evm.clone(),
        berachain: evm.clone(),
        unichain: evm.clone(),
        ink: evm,
        decred: slip44(42, account),
        // Kaspa SLIP-44 coin type 111111.
        kaspa: format!("m/44'/111111'/{account}'/0/0"),
        dash: slip44(5, account),
        // X Layer is an EVM L2 — uses the standard EVM derivation path.
        x_layer: slip44(60, account),
        // Bittensor uses SLIP-44 1005 (Polkadot.js convention for substrate
        // chains; the substrate-bip39 expansion ignores BIP-32 path nodes
        // but we include the canonical path for downstream display).
        bittensor: format!("m/44'/1005'/{account}'/0'/0'"),
    })
}

fn slip44(coin_type: u32, account: u32) -> String {
    format!("m/44'/{coin_type}'/{account}'/0/0")
}

pub(super) fn default_path_from_catalog(chain_name: &str) -> Result<String, String> {
    crate::chains::catalog()
        .iter()
        .find(|c| c.name == chain_name)
        .map(|c| c.derivation_path.clone())
        .filter(|p| p.starts_with("m/"))
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

#[uniffi::export]
pub fn core_normalize_derivation_path(raw_path: String, fallback: String) -> String {
    normalize_derivation_path(&raw_path, &fallback)
}

#[uniffi::export]
pub fn core_derivation_path_segment_value(path: String, index: u32) -> Option<u32> {
    derivation_path_segment_value(&path, index as usize)
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
            id:                      c.id.clone(),
            chain_name:              c.name.clone(),
            short_label:             c.short_label.clone(),
            native_symbol:           c.gas_token_symbol.clone(),
            search_keywords:         c.search_keywords.clone(),
            supports_diagnostics:    c.supports_diagnostics,
            supports_endpoint_catalog: c.supports_endpoint_catalog,
            is_evm:                  c.is_evm,
        })
        .collect()
}

// ── broadcast_provider_options ─────────────────────────────────────────────

pub(super) fn broadcast_provider_options(chain_name: &str) -> Vec<AppCoreBroadcastProviderOption> {
    let resolved = crate::registry::Chain::from_display_name(chain_name)
        .map(|c| c.mainnet_counterpart().chain_display_name())
        .unwrap_or(chain_name);
    let pairs: &[(&str, &str)] = match resolved {
        "Bitcoin" => &[("esplora", "Esplora"), ("maestro-esplora", "Maestro Esplora")],
        "Bitcoin Cash" => &[("blockchair", "Blockchair"), ("actorforth", "ActorForth REST")],
        "Bitcoin SV" => &[("whatsonchain", "WhatsOnChain"), ("blockchair", "Blockchair")],
        "Litecoin" => &[("litecoinspace", "LitecoinSpace"), ("blockcypher", "BlockCypher")],
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
        "Sei" | "Celo" | "Cronos" | "opBNB" | "zkSync Era" | "Sonic" | "Berachain"
        | "Unichain" | "Ink" | "X Layer" => &[("rpc", "RPC Broadcast")],
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
