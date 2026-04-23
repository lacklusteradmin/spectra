use crate::store::wallet_domain::CoreSeedDerivationPaths;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const CHAIN_PRESETS_JSON: &str = include_str!("../data/DerivationPresets.json");
const REQUEST_COMPILATION_PRESETS_JSON: &str =
    include_str!("../data/DerivationRequestCompilationPresets.json");
const APP_ENDPOINT_DIRECTORY_JSON: &str =
    include_str!("../data/AppEndpointDirectory.json");

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

/// Endpoint-table slot for a given chain. Mirrors `chains::registry::EndpointSlot`
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct DerivationPathSegment {
    pub value: u32,
    pub is_hardened: bool,
}

static APP_CORE_CATALOG: OnceLock<Result<AppCoreCatalog, String>> = OnceLock::new();

#[uniffi::export]
pub fn app_core_chain_presets() -> Result<Vec<AppCoreChainPreset>, crate::SpectraBridgeError> {
    Ok(app_core_catalog()?.chain_presets.clone())
}

#[uniffi::export]
pub fn app_core_request_compilation_presets() -> Result<Vec<AppCoreRequestCompilationPreset>, crate::SpectraBridgeError> {
    Ok(app_core_catalog()?.request_compilation_presets.clone())
}

#[uniffi::export]
pub fn app_core_resolve_derivation_path(
    chain: String,
    derivation_path: String,
) -> Result<AppCoreDerivationPathResolution, crate::SpectraBridgeError> {
    let catalog = app_core_catalog()?;
    let default_path = default_path_from_catalog(catalog, &chain)?;
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
    Ok(app_core_catalog()
        .and_then(|catalog| seed_derivation_paths_for_account(catalog, account_index))?)
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
    Ok(app_core_catalog()
        .and_then(|catalog| {
            catalog
                .endpoint_records
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.endpoint.clone())
                .ok_or_else(|| format!("Missing endpoint record for id: {id}"))
        })?)
}

#[uniffi::export]
pub fn app_core_endpoints_for_ids(ids: Vec<String>) -> Result<Vec<String>, crate::SpectraBridgeError> {
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
    Ok(app_core_catalog()
        .map(|catalog| grouped_settings_entries(catalog, &chain_name))?)
}

#[uniffi::export]
pub fn app_core_diagnostics_checks(
    chain_name: String,
) -> Result<Vec<AppCoreDiagnosticsCheck>, crate::SpectraBridgeError> {
    Ok(app_core_catalog()
        .map(|catalog| diagnostics_checks(catalog, &chain_name))?)
}

#[uniffi::export]
pub fn app_core_transaction_explorer_entry(
    chain_name: String,
) -> Result<Option<AppCoreExplorerEntry>, crate::SpectraBridgeError> {
    Ok(app_core_catalog()
        .map(|catalog| transaction_explorer_entry(catalog, &chain_name))?)
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
    Ok(app_core_catalog()
        .and_then(|catalog| bitcoin_wallet_store_default_base_urls(catalog, &network))?)
}

#[uniffi::export]
pub fn app_core_evm_rpc_endpoints(
    chain_name: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().and_then(|catalog| {
        Ok(endpoint_records_for_chain(catalog, &chain_name, ENDPOINT_ROLE_RPC, false)
            .into_iter()
            .map(|r| r.endpoint)
            .collect())
    })?)
}

#[uniffi::export]
pub fn app_core_explorer_supplemental_endpoints(
    chain_name: String,
) -> Result<Vec<String>, crate::SpectraBridgeError> {
    Ok(app_core_catalog().and_then(|catalog| {
        Ok(endpoint_records_for_chain(catalog, &chain_name, ENDPOINT_ROLE_EXPLORER, true)
            .into_iter()
            .map(|r| r.endpoint)
            .collect())
    })?)
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

fn app_core_catalog() -> Result<&'static AppCoreCatalog, String> {
    match APP_CORE_CATALOG.get_or_init(load_app_core_catalog) {
        Ok(catalog) => Ok(catalog),
        Err(message) => Err(message.clone()),
    }
}

fn load_app_core_catalog() -> Result<AppCoreCatalog, String> {
    let chain_presets = serde_json::from_str::<Vec<AppCoreChainPreset>>(CHAIN_PRESETS_JSON)
        .map_err(display_error)?;
    let request_compilation_presets = serde_json::from_str::<Vec<AppCoreRequestCompilationPreset>>(
        REQUEST_COMPILATION_PRESETS_JSON,
    )
    .map_err(display_error)?;
    let endpoint_records =
        serde_json::from_str::<Vec<AppCoreEndpointRecord>>(APP_ENDPOINT_DIRECTORY_JSON)
            .map_err(display_error)?;
    Ok(AppCoreCatalog {
        chain_presets,
        request_compilation_presets,
        endpoint_records,
    })
}

#[cfg(test)]
fn default_path_for_chain(chain_name: &str) -> Result<String, String> {
    let catalog = app_core_catalog()?;
    catalog
        .chain_presets
        .iter()
        .find(|preset| preset.chain == chain_name)
        .and_then(|preset| {
            preset
                .derivation_paths
                .iter()
                .find(|path| path.is_default)
                .or_else(|| preset.derivation_paths.first())
        })
        .map(|path| path.derivation_path.clone())
        .ok_or_else(|| format!("Missing default derivation path for {chain_name}."))
}

fn seed_derivation_paths_for_account(
    catalog: &AppCoreCatalog,
    account_index: u32,
) -> Result<CoreSeedDerivationPaths, String> {
    Ok(CoreSeedDerivationPaths {
        is_custom_enabled: false,
        bitcoin: format!("m/84'/0'/{account_index}'/0/0"),
        bitcoin_cash: format!("m/44'/145'/{account_index}'/0/0"),
        bitcoin_sv: default_path_from_catalog(catalog, "Bitcoin SV")?,
        litecoin: format!("m/44'/2'/{account_index}'/0/0"),
        dogecoin: format!("m/44'/3'/{account_index}'/0/0"),
        ethereum: format!("m/44'/60'/{account_index}'/0/0"),
        ethereum_classic: format!("m/44'/61'/{account_index}'/0/0"),
        arbitrum: format!("m/44'/60'/{account_index}'/0/0"),
        optimism: format!("m/44'/60'/{account_index}'/0/0"),
        avalanche: format!("m/44'/60'/{account_index}'/0/0"),
        hyperliquid: format!("m/44'/60'/{account_index}'/0/0"),
        tron: format!("m/44'/195'/{account_index}'/0/0"),
        solana: format!("m/44'/501'/{account_index}'/0'"),
        stellar: format!("m/44'/148'/{account_index}'"),
        xrp: format!("m/44'/144'/{account_index}'/0/0"),
        cardano: format!("m/1852'/1815'/{account_index}'/0/0"),
        sui: format!("m/44'/784'/{account_index}'/0'/0'"),
        aptos: format!("m/44'/637'/{account_index}'/0'/0'"),
        ton: format!("m/44'/607'/{account_index}'/0/0"),
        internet_computer: format!("m/44'/223'/{account_index}'/0/0"),
        near: format!("m/44'/397'/{account_index}'"),
        polkadot: format!("m/44'/354'/{account_index}'"),
    })
}

fn default_path_from_catalog(catalog: &AppCoreCatalog, chain_name: &str) -> Result<String, String> {
    catalog
        .chain_presets
        .iter()
        .find(|preset| preset.chain == chain_name)
        .and_then(|preset| {
            preset
                .derivation_paths
                .iter()
                .find(|path| path.is_default)
                .or_else(|| preset.derivation_paths.first())
        })
        .map(|path| path.derivation_path.clone())
        .ok_or_else(|| format!("Missing default derivation path for {chain_name}."))
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
    catalog
        .endpoint_records
        .iter()
        .filter(|record| record.chain_name == chain_name)
        .filter(|record| !settings_visible_only || record.settings_visible)
        .filter(|record| {
            role_mask == 0
                || record
                    .roles
                    .iter()
                    .any(|role| endpoint_role_bit(role) & role_mask != 0)
        })
        .cloned()
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

fn endpoints_for_known_ids(catalog: &AppCoreCatalog, ids: &[&str]) -> Result<Vec<String>, String> {
    ids.iter()
        .map(|id| {
            catalog
                .endpoint_records
                .iter()
                .find(|record| record.id == *id)
                .map(|record| record.endpoint.clone())
                .ok_or_else(|| format!("Missing endpoint record for id: {id}"))
        })
        .collect()
}

fn bitcoin_esplora_base_urls(
    catalog: &AppCoreCatalog,
    network: &str,
) -> Result<Vec<String>, String> {
    match network {
        "mainnet" => endpoints_for_known_ids(
            catalog,
            &[
                "bitcoin.mainnet.blockstream",
                "bitcoin.mainnet.mempool",
                "bitcoin.mainnet.mempool_emzy",
                "bitcoin.mainnet.maestro",
            ],
        ),
        "testnet" => endpoints_for_known_ids(
            catalog,
            &["bitcoin.testnet.blockstream", "bitcoin.testnet.mempool"],
        ),
        "testnet4" => endpoints_for_known_ids(catalog, &["bitcoin.testnet4.mempool"]),
        "signet" => endpoints_for_known_ids(
            catalog,
            &["bitcoin.signet.blockstream", "bitcoin.signet.mempool"],
        ),
        _ => Err(format!("Unsupported Bitcoin network mode: {network}")),
    }
}

fn bitcoin_wallet_store_default_base_urls(
    catalog: &AppCoreCatalog,
    network: &str,
) -> Result<Vec<String>, String> {
    match network {
        "mainnet" => endpoints_for_known_ids(
            catalog,
            &[
                "bitcoin.mainnet.blockstream",
                "bitcoin.mainnet.mempool",
                "bitcoin.mainnet.maestro",
            ],
        ),
        "testnet" => endpoints_for_known_ids(
            catalog,
            &["bitcoin.testnet.blockstream", "bitcoin.testnet.mempool"],
        ),
        "testnet4" => endpoints_for_known_ids(catalog, &["bitcoin.testnet4.mempool"]),
        "signet" => endpoints_for_known_ids(catalog, &["bitcoin.signet.mempool"]),
        _ => Err(format!("Unsupported Bitcoin network mode: {network}")),
    }
}

fn broadcast_provider_options(chain_name: &str) -> Vec<AppCoreBroadcastProviderOption> {
    let pairs: &[(&str, &str)] = match chain_name {
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
        | "Hyperliquid" => &[("rpc", "RPC Broadcast")],
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

fn chain_backends() -> Vec<AppCoreChainBackend> {
    vec![
        AppCoreChainBackend {
            chain_name: "Bitcoin".to_string(),
            supported_symbols: vec!["BTC".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Bitcoin Cash".to_string(),
            supported_symbols: vec!["BCH".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Bitcoin SV".to_string(),
            supported_symbols: vec!["BSV".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Litecoin".to_string(),
            supported_symbols: vec!["LTC".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Ethereum".to_string(),
            supported_symbols: vec![
                "ETH".to_string(),
                "USDT".to_string(),
                "USDC".to_string(),
                "DAI".to_string(),
            ],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Arbitrum".to_string(),
            supported_symbols: vec!["ETH".to_string(), "Tracked ERC-20s".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Optimism".to_string(),
            supported_symbols: vec!["ETH".to_string(), "Tracked ERC-20s".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Ethereum Classic".to_string(),
            supported_symbols: vec!["ETC".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Dogecoin".to_string(),
            supported_symbols: vec!["DOGE".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "BNB Chain".to_string(),
            supported_symbols: vec!["BNB".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Avalanche".to_string(),
            supported_symbols: vec!["AVAX".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Hyperliquid".to_string(),
            supported_symbols: vec!["HYPE".to_string(), "Tracked ERC-20s".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Tron".to_string(),
            supported_symbols: vec!["TRX".to_string(), "USDT".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Solana".to_string(),
            supported_symbols: vec!["SOL".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "XRP Ledger".to_string(),
            supported_symbols: vec!["XRP".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Monero".to_string(),
            supported_symbols: vec!["XMR".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Cardano".to_string(),
            supported_symbols: vec!["ADA".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Sui".to_string(),
            supported_symbols: vec!["SUI".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Aptos".to_string(),
            supported_symbols: vec!["APT".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "TON".to_string(),
            supported_symbols: vec!["TON".to_string(), "Tracked Jettons".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Internet Computer".to_string(),
            supported_symbols: vec!["ICP".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "NEAR".to_string(),
            supported_symbols: vec!["NEAR".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Polkadot".to_string(),
            supported_symbols: vec!["DOT".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Stellar".to_string(),
            supported_symbols: vec!["XLM".to_string()],
            integration_state: AppCoreChainIntegrationState::Live,
            supports_seed_import: true,
            supports_balance_refresh: true,
            supports_receive_address: true,
            supports_send: true,
        },
        AppCoreChainBackend {
            chain_name: "Polygon".to_string(),
            supported_symbols: vec!["MATIC".to_string()],
            integration_state: AppCoreChainIntegrationState::Planned,
            supports_seed_import: true,
            supports_balance_refresh: false,
            supports_receive_address: false,
            supports_send: false,
        },
    ]
}

fn live_chain_names() -> Vec<String> {
    [
        "Bitcoin",
        "Bitcoin Cash",
        "Litecoin",
        "Ethereum",
        "Arbitrum",
        "Optimism",
        "Ethereum Classic",
        "Dogecoin",
        "BNB Chain",
        "Avalanche",
        "Hyperliquid",
        "Tron",
        "Solana",
        "XRP Ledger",
        "Monero",
        "Cardano",
        "Sui",
        "Aptos",
        "TON",
        "Internet Computer",
        "NEAR",
        "Polkadot",
        "Stellar",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn app_chain_descriptors() -> Vec<AppCoreAppChainDescriptor> {
    vec![
        AppCoreAppChainDescriptor {
            id: "bitcoin".to_string(),
            chain_name: "Bitcoin".to_string(),
            short_label: "BTC".to_string(),
            native_symbol: "BTC".to_string(),
            search_keywords: vec!["Bitcoin".to_string(), "BTC".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "bitcoinCash".to_string(),
            chain_name: "Bitcoin Cash".to_string(),
            short_label: "BCH".to_string(),
            native_symbol: "BCH".to_string(),
            search_keywords: vec!["Bitcoin Cash".to_string(), "BCH".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "bitcoinSV".to_string(),
            chain_name: "Bitcoin SV".to_string(),
            short_label: "BSV".to_string(),
            native_symbol: "BSV".to_string(),
            search_keywords: vec!["Bitcoin SV".to_string(), "BSV".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: false,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "litecoin".to_string(),
            chain_name: "Litecoin".to_string(),
            short_label: "LTC".to_string(),
            native_symbol: "LTC".to_string(),
            search_keywords: vec!["Litecoin".to_string(), "LTC".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "dogecoin".to_string(),
            chain_name: "Dogecoin".to_string(),
            short_label: "DOGE".to_string(),
            native_symbol: "DOGE".to_string(),
            search_keywords: vec!["Dogecoin".to_string(), "DOGE".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "ethereum".to_string(),
            chain_name: "Ethereum".to_string(),
            short_label: "ETH".to_string(),
            native_symbol: "ETH".to_string(),
            search_keywords: vec!["Ethereum".to_string(), "ETH".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: true,
        },
        AppCoreAppChainDescriptor {
            id: "ethereumClassic".to_string(),
            chain_name: "Ethereum Classic".to_string(),
            short_label: "ETC".to_string(),
            native_symbol: "ETC".to_string(),
            search_keywords: vec!["Ethereum Classic".to_string(), "ETC".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: true,
        },
        AppCoreAppChainDescriptor {
            id: "arbitrum".to_string(),
            chain_name: "Arbitrum".to_string(),
            short_label: "ARB".to_string(),
            native_symbol: "ETH".to_string(),
            search_keywords: vec!["Arbitrum".to_string(), "ARB".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: true,
        },
        AppCoreAppChainDescriptor {
            id: "optimism".to_string(),
            chain_name: "Optimism".to_string(),
            short_label: "OP".to_string(),
            native_symbol: "ETH".to_string(),
            search_keywords: vec!["Optimism".to_string(), "OP".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: true,
        },
        AppCoreAppChainDescriptor {
            id: "bnb".to_string(),
            chain_name: "BNB Chain".to_string(),
            short_label: "BNB".to_string(),
            native_symbol: "BNB".to_string(),
            search_keywords: vec!["BNB Chain".to_string(), "BNB".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: true,
        },
        AppCoreAppChainDescriptor {
            id: "avalanche".to_string(),
            chain_name: "Avalanche".to_string(),
            short_label: "AVAX".to_string(),
            native_symbol: "AVAX".to_string(),
            search_keywords: vec!["Avalanche".to_string(), "AVAX".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: true,
        },
        AppCoreAppChainDescriptor {
            id: "hyperliquid".to_string(),
            chain_name: "Hyperliquid".to_string(),
            short_label: "HYPE".to_string(),
            native_symbol: "HYPE".to_string(),
            search_keywords: vec!["Hyperliquid".to_string(), "HYPE".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: true,
        },
        AppCoreAppChainDescriptor {
            id: "tron".to_string(),
            chain_name: "Tron".to_string(),
            short_label: "TRX".to_string(),
            native_symbol: "TRX".to_string(),
            search_keywords: vec!["Tron".to_string(), "TRX".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "solana".to_string(),
            chain_name: "Solana".to_string(),
            short_label: "SOL".to_string(),
            native_symbol: "SOL".to_string(),
            search_keywords: vec!["Solana".to_string(), "SOL".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "cardano".to_string(),
            chain_name: "Cardano".to_string(),
            short_label: "ADA".to_string(),
            native_symbol: "ADA".to_string(),
            search_keywords: vec!["Cardano".to_string(), "ADA".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "xrp".to_string(),
            chain_name: "XRP Ledger".to_string(),
            short_label: "XRP".to_string(),
            native_symbol: "XRP".to_string(),
            search_keywords: vec!["XRP".to_string(), "XRP Ledger".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "stellar".to_string(),
            chain_name: "Stellar".to_string(),
            short_label: "XLM".to_string(),
            native_symbol: "XLM".to_string(),
            search_keywords: vec!["Stellar".to_string(), "XLM".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "monero".to_string(),
            chain_name: "Monero".to_string(),
            short_label: "XMR".to_string(),
            native_symbol: "XMR".to_string(),
            search_keywords: vec!["Monero".to_string(), "XMR".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "sui".to_string(),
            chain_name: "Sui".to_string(),
            short_label: "SUI".to_string(),
            native_symbol: "SUI".to_string(),
            search_keywords: vec!["Sui".to_string(), "SUI".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "aptos".to_string(),
            chain_name: "Aptos".to_string(),
            short_label: "APT".to_string(),
            native_symbol: "APT".to_string(),
            search_keywords: vec!["Aptos".to_string(), "APT".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "ton".to_string(),
            chain_name: "TON".to_string(),
            short_label: "TON".to_string(),
            native_symbol: "TON".to_string(),
            search_keywords: vec!["TON".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "icp".to_string(),
            chain_name: "Internet Computer".to_string(),
            short_label: "ICP".to_string(),
            native_symbol: "ICP".to_string(),
            search_keywords: vec!["Internet Computer".to_string(), "ICP".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "near".to_string(),
            chain_name: "NEAR".to_string(),
            short_label: "NEAR".to_string(),
            native_symbol: "NEAR".to_string(),
            search_keywords: vec!["NEAR".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
        AppCoreAppChainDescriptor {
            id: "polkadot".to_string(),
            chain_name: "Polkadot".to_string(),
            short_label: "DOT".to_string(),
            native_symbol: "DOT".to_string(),
            search_keywords: vec!["Polkadot".to_string(), "DOT".to_string()],
            supports_diagnostics: true,
            supports_endpoint_catalog: true,
            is_evm: false,
        },
    ]
}

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
    parse_derivation_path(path)
        .and_then(|segments| segments.get(index).map(|segment| segment.value))
}

pub(crate) fn compile_script_type(
    preset: &AppCoreRequestCompilationPreset,
    derivation_path: Option<&str>,
) -> Result<AppCoreScriptType, String> {
    match preset.script_policy {
        AppCoreScriptPolicy::BitcoinPurpose => {
            let purpose = derivation_path
                .and_then(|path| derivation_path_segment_value(path, 0))
                .ok_or_else(|| {
                    "Unable to compile Bitcoin script type from derivation path.".to_string()
                })?;
            let map = preset.bitcoin_purpose_script_map.as_ref().ok_or_else(|| {
                "Bitcoin purpose script policy requires bitcoinPurposeScriptMap.".to_string()
            })?;
            map.get(&purpose.to_string())
                .copied()
                .ok_or_else(|| format!("Unsupported Bitcoin derivation purpose {purpose}."))
        }
        AppCoreScriptPolicy::Fixed => preset
            .fixed_script_type
            .ok_or_else(|| "Fixed script policy requires fixedScriptType.".to_string()),
    }
}

fn resolved_account_index(chain_name: &str, normalized_path: &str) -> u32 {
    match chain_name {
        "Bitcoin" if normalized_path == "m/0'/0" || normalized_path == "m/0'/0/0" => 0,
        "Bitcoin Cash" if normalized_path == "m/0" => 0,
        "Bitcoin SV" if normalized_path == "m/0" => 0,
        _ => derivation_path_segment_value(normalized_path, 2).unwrap_or(0),
    }
}

fn resolved_flavor(chain_name: &str, normalized_path: &str) -> &'static str {
    match chain_name {
        "Bitcoin" => {
            if normalized_path.starts_with("m/86'") {
                "taproot"
            } else if normalized_path.starts_with("m/84'") {
                "nativeSegWit"
            } else if normalized_path.starts_with("m/49'") {
                "nestedSegWit"
            } else if normalized_path == "m/0'/0" || normalized_path == "m/0'/0/0" {
                "electrumLegacy"
            } else if normalized_path.starts_with("m/44'") {
                "legacy"
            } else {
                "standard"
            }
        }
        "Litecoin" => {
            if normalized_path.starts_with("m/84'/2'") {
                "nativeSegWit"
            } else if normalized_path.starts_with("m/49'/2'") {
                "nestedSegWit"
            } else if normalized_path.starts_with("m/44'/2'") {
                "legacy"
            } else {
                "standard"
            }
        }
        "Bitcoin Cash" => {
            if normalized_path == "m/0" {
                "electrumLegacy"
            } else if normalized_path.starts_with("m/44'/0'")
                || normalized_path.starts_with("m/44'/145'")
            {
                "legacy"
            } else {
                "standard"
            }
        }
        "Solana" => {
            if normalized_path == "m/44'/501'/0'" {
                "legacy"
            } else {
                "standard"
            }
        }
        "Cardano" => {
            if normalized_path.starts_with("m/44'/1815'") {
                "legacy"
            } else {
                "standard"
            }
        }
        "Tron" => {
            if normalized_path == "m/44'/195'/0'" || normalized_path.starts_with("m/44'/60'") {
                "legacy"
            } else {
                "standard"
            }
        }
        "XRP Ledger" => {
            if normalized_path == "m/44'/144'/0'" {
                "legacy"
            } else {
                "standard"
            }
        }
        _ => "standard",
    }
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_chain_presets_catalog() {
        let catalog = app_core_catalog().expect("catalog");
        assert!(catalog
            .chain_presets
            .iter()
            .any(|preset| preset.chain == "Bitcoin"));
        assert!(catalog
            .request_compilation_presets
            .iter()
            .any(|preset| preset.chain == "Ethereum"));
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
    fn preserves_bitcoin_sv_default_path_for_preset_accounts() {
        let catalog = app_core_catalog().expect("catalog");
        let paths = seed_derivation_paths_for_account(catalog, 2).expect("paths");

        assert_eq!(paths.bitcoin_sv, "m/44'/236'/0'/0/0");
        assert_eq!(paths.ethereum, "m/44'/60'/2'/0/0");
        assert_eq!(paths.solana, "m/44'/501'/2'/0'");
    }
}
