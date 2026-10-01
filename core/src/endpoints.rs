//! The endpoint catalog: `data/endpoints.toml`, validated against the
//! adapters Spectra has, and indexed by network.

use crate::registry::Chain;
use crate::{EndpointApi, EndpointCapability};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

const ENDPOINTS_TOML: &str = include_str!("../data/endpoints.toml");

#[derive(Debug, Clone)]
pub(crate) struct EndpointCatalog {
    pub(crate) records: Vec<EndpointRecord>,
    /// Concrete network → record indices, preserving endpoint order.
    by_chain: HashMap<Chain, Vec<usize>>,
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
    chain_id: Chain,
    api: EndpointApi,
    endpoint: String,
    capabilities: Vec<EndpointCapability>,
}

impl TryFrom<TomlEndpoint> for EndpointRecord {
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
        Ok(EndpointRecord {
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
pub struct EndpointRecord {
    pub id: String,
    pub api: EndpointApi,
    pub chain_id: Chain,
    pub endpoint: String,
    /// What this endpoint is used for: a claim that has to be true of the
    /// endpoint, and one Spectra's adapter for its API can act on.
    pub capabilities: Vec<EndpointCapability>,
}

/// One network's endpoints as the settings screen groups them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EndpointSettingsGroup {
    pub chain_id: Chain,
    pub title: String,
    pub endpoints: Vec<String>,
}

/// Everything the endpoint catalog holds for one chain.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChainEndpointSettings {
    pub chain_id: Chain,
    /// What the settings screen shows, grouped by network.
    pub groups: Vec<EndpointSettingsGroup>,
}

/// The endpoint catalog, one row per chain, in catalog order.
#[uniffi::export]
pub fn endpoint_settings() -> Vec<ChainEndpointSettings> {
    Chain::all()
        .map(|chain| ChainEndpointSettings {
            chain_id: chain,
            groups: catalog().settings_groups(chain),
        })
        .collect()
}

/// A chain's endpoint records that declare any of `any_of`, or all of them
/// when `any_of` is empty.
pub fn records_for_chain(chain: Chain, any_of: &[EndpointCapability]) -> Vec<EndpointRecord> {
    catalog().records_for_chain(chain, any_of)
}

/// Embedded at compile time, so a catalog that does not load is a build
/// defect, refused on first use like the chain and token catalogs.
static CATALOG: LazyLock<EndpointCatalog> = LazyLock::new(|| {
    load_catalog(ENDPOINTS_TOML).unwrap_or_else(|error| panic!("endpoints.toml: {error}"))
});

pub(crate) fn catalog() -> &'static EndpointCatalog {
    &CATALOG
}

fn load_catalog(toml_text: &str) -> Result<EndpointCatalog, String> {
    let records = toml::from_str::<TomlEndpointFile>(toml_text)
        .map_err(|e| e.to_string())?
        .endpoints
        .into_iter()
        .map(EndpointRecord::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let mut by_chain: HashMap<Chain, Vec<usize>> = HashMap::new();
    for (idx, record) in records.iter().enumerate() {
        by_chain.entry(record.chain_id).or_default().push(idx);
    }
    Ok(EndpointCatalog { records, by_chain })
}

impl EndpointCatalog {
    pub(crate) fn records_for_chain(
        &self,
        chain: Chain,
        any_of: &[EndpointCapability],
    ) -> Vec<EndpointRecord> {
        self.by_chain
            .get(&chain)
            .into_iter()
            .flatten()
            .map(|&idx| &self.records[idx])
            .filter(|record| {
                any_of.is_empty() || any_of.iter().any(|c| record.capabilities.contains(c))
            })
            .cloned()
            .collect()
    }

    fn settings_groups(&self, chain: Chain) -> Vec<EndpointSettingsGroup> {
        Chain::all()
            .filter(|network| {
                *network == chain || (!chain.is_testnet() && network.mainnet_counterpart() == chain)
            })
            .filter_map(|network| {
                let mut endpoints = Vec::new();
                for record in self.records_for_chain(network, &[]) {
                    if !endpoints.contains(&record.endpoint) {
                        endpoints.push(record.endpoint);
                    }
                }
                (!endpoints.is_empty()).then(|| EndpointSettingsGroup {
                    chain_id: network,
                    title: network.chain_display_name().to_string(),
                    endpoints,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod network_index_tests {
    use super::*;

    /// The network index every endpoint consumer reads through.
    fn rpc_endpoints(chain_id: Chain) -> Vec<String> {
        catalog()
            .records_for_chain(chain_id, &[])
            .into_iter()
            .filter(|r| r.api == EndpointApi::EvmJsonRpc)
            .map(|r| r.endpoint)
            .collect()
    }

    /// Network IDs keep testnet lookups independent of mainnet and UI titles.
    #[test]
    fn a_testnet_resolves_its_own_rpc_endpoints() {
        assert_eq!(
            rpc_endpoints(Chain::EthereumSepolia),
            vec![
                "https://ethereum-sepolia-rpc.publicnode.com".to_string(),
                "https://1rpc.io/sepolia".to_string(),
            ]
        );
        assert_eq!(
            rpc_endpoints(Chain::EthereumHoodi),
            vec![
                "https://ethereum-hoodi-rpc.publicnode.com".to_string(),
                "https://1rpc.io/hoodi".to_string(),
            ]
        );
    }

    #[test]
    fn a_mainnet_rpc_list_holds_no_testnet_endpoints() {
        for endpoint in rpc_endpoints(Chain::Ethereum) {
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
        let titles: Vec<String> = catalog()
            .settings_groups(Chain::Bitcoin)
            .into_iter()
            .map(|group| group.title)
            .collect();
        for expected in ["Bitcoin Testnet", "Bitcoin Testnet4", "Bitcoin Signet"] {
            assert!(titles.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn every_record_belongs_to_exactly_its_network() {
        let catalog = catalog();
        for chain in Chain::all() {
            let rows = catalog.records_for_chain(chain, &[]);
            let expected: Vec<_> = catalog
                .records
                .iter()
                .filter(|r| r.chain_id == chain)
                .cloned()
                .collect();
            assert_eq!(rows, expected);
            for group in catalog.settings_groups(chain) {
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
            EndpointRecord::try_from(row).unwrap().chain_id,
            Chain::EthereumSepolia
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
            EndpointRecord::try_from(overclaimed)
                .unwrap_err()
                .contains("no history adapter")
        );
        let unused = toml::from_str::<TomlEndpoint>(&valid.replace("[\"balance\"]", "[]")).unwrap();
        assert!(EndpointRecord::try_from(unused).is_err());

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
mod capability_tests {
    use super::*;

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
        for record in &catalog().records {
            if !record.chain_id.is_evm() || record.api != EndpointApi::EvmJsonRpc {
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
            records_for_chain(Chain::parse(chain).unwrap(), &[capability.parse().unwrap()])
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

    fn record(endpoint: &str) -> EndpointRecord {
        catalog()
            .records
            .iter()
            .find(|r| r.endpoint == endpoint)
            .cloned()
            .unwrap_or_else(|| panic!("{endpoint} is in the catalog"))
    }

    #[test]
    fn a_node_and_an_indexer_declare_different_capabilities() {
        let node = record("https://ethereum-rpc.publicnode.com");
        assert_eq!(node.api, EndpointApi::EvmJsonRpc);
        assert!(node.capabilities.contains(&EndpointCapability::Balance));
        assert!(
            !node.capabilities.contains(&EndpointCapability::History),
            "an EVM node cannot serve address history"
        );
        let indexer = record("https://eth.blockscout.com");
        assert!(indexer.capabilities.contains(&EndpointCapability::History));
    }
}
