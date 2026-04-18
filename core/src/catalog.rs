use super::localization::{localization_catalog, LOCALIZATION_TABLES};
use super::types::{ChainSummary, CoreBootstrap, CoreCapabilities, LocalizationSummary};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const CHAIN_PRESETS_JSON: &str = include_str!("../embedded/DerivationPresets.json");
const APP_ENDPOINT_DIRECTORY_JSON: &str =
    include_str!("../embedded/AppEndpointDirectory.json");

const EXPLORER_ROLE: &str = "explorer";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChainPreset {
    chain: String,
    curve: String,
    networks: Vec<ChainNetworkPreset>,
    derivation_paths: Vec<ChainPathPreset>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChainNetworkPreset {
    network: String,
    is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChainPathPreset {
    derivation_path: String,
    is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EndpointRecord {
    chain_name: String,
    roles: Vec<String>,
    settings_visible: bool,
}

#[derive(Debug, Clone)]
struct CoreCatalog {
    chains: Vec<ChainSummary>,
    live_chain_names: Vec<String>,
}

static CORE_CATALOG: OnceLock<Result<CoreCatalog, String>> = OnceLock::new();
static BOOTSTRAP_CACHE: OnceLock<Result<CoreBootstrap, String>> = OnceLock::new();

pub fn core_bootstrap() -> Result<CoreBootstrap, String> {
    match BOOTSTRAP_CACHE.get_or_init(build_bootstrap) {
        Ok(bootstrap) => Ok(bootstrap.clone()),
        Err(message) => Err(message.clone()),
    }
}

fn build_bootstrap() -> Result<CoreBootstrap, String> {
    let catalog = core_catalog()?;
    let localization = localization_catalog()?;
    let supported_locales = localization.supported_locales();
    let tables: Vec<String> = LOCALIZATION_TABLES
        .iter()
        .map(|table| (*table).to_string())
        .collect();

    Ok(CoreBootstrap {
        capabilities: CoreCapabilities {
            schema_version: 1,
            supports_derivation: true,
            supports_fetch_contracts: true,
            supports_send_contracts: true,
            supports_store_contracts: true,
            supports_localization_catalogs: true,
            supports_state_reducer: true,
            supported_locales: supported_locales.clone(),
            localization_tables: tables.clone(),
        },
        chains: catalog.chains.clone(),
        localization: LocalizationSummary {
            supported_locales,
            tables,
        },
        live_chain_names: catalog.live_chain_names.clone(),
    })
}

pub fn live_chain_names() -> Result<Vec<String>, String> {
    Ok(core_catalog()?.live_chain_names.clone())
}

fn core_catalog() -> Result<&'static CoreCatalog, String> {
    match CORE_CATALOG.get_or_init(load_core_catalog) {
        Ok(catalog) => Ok(catalog),
        Err(message) => Err(message.clone()),
    }
}

fn load_core_catalog() -> Result<CoreCatalog, String> {
    let chain_presets =
        serde_json::from_str::<Vec<ChainPreset>>(CHAIN_PRESETS_JSON).map_err(display_error)?;
    let endpoint_records = serde_json::from_str::<Vec<EndpointRecord>>(APP_ENDPOINT_DIRECTORY_JSON)
        .map_err(display_error)?;

    let mut endpoint_summary = BTreeMap::<String, (usize, usize, usize)>::new();
    for record in endpoint_records {
        let entry = endpoint_summary
            .entry(record.chain_name)
            .or_insert((0, 0, 0));
        entry.0 += 1;
        if record.settings_visible {
            entry.1 += 1;
        }
        if record.roles.iter().any(|role| role == EXPLORER_ROLE) {
            entry.2 += 1;
        }
    }

    let mut live_chain_names = BTreeSet::new();
    let mut chains = Vec::with_capacity(chain_presets.len());
    for preset in chain_presets {
        let counts = endpoint_summary
            .get(&preset.chain)
            .copied()
            .unwrap_or((0, 0, 0));
        let default_network = preset
            .networks
            .iter()
            .find(|network| network.is_default)
            .or_else(|| preset.networks.first())
            .map(|network| network.network.clone());
        let default_derivation_path = preset
            .derivation_paths
            .iter()
            .find(|path| path.is_default)
            .or_else(|| preset.derivation_paths.first())
            .map(|path| path.derivation_path.clone());
        live_chain_names.insert(preset.chain.clone());
        chains.push(ChainSummary {
            chain_name: preset.chain,
            curve: preset.curve,
            default_network,
            default_derivation_path,
            endpoint_count: counts.0 as u64,
            settings_visible_endpoint_count: counts.1 as u64,
            explorer_endpoint_count: counts.2 as u64,
        });
    }

    Ok(CoreCatalog {
        chains,
        live_chain_names: live_chain_names.into_iter().collect(),
    })
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_exposes_expected_capabilities() {
        let bootstrap = core_bootstrap().expect("bootstrap");
        assert!(bootstrap.capabilities.supports_localization_catalogs);
        assert!(bootstrap
            .chains
            .iter()
            .any(|chain| chain.chain_name == "Bitcoin"));
        assert!(bootstrap
            .localization
            .tables
            .iter()
            .any(|table| table == "CommonContent"));
    }
}
