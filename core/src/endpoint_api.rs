//! Endpoint wire contracts, independent of operator and network identity.
use serde::{Deserialize, Serialize};

/// The adapter a URL speaks, not the company operating it. A catalog entry
/// does not imply every operation of that API is implemented by Spectra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointApi {
    EvmJsonRpc,
    SolanaJsonRpc,
    SuiJsonRpc,
    NearJsonRpc,
    XrplJsonRpc,
    SubstrateJsonRpc,
    MoneroDaemonRpc,
    Esplora,
    Blockbook,
    Blockcypher,
    Whatsonchain,
    Blockscout,
    ToncenterV2,
    ToncenterV3,
    Koios,
    Horizon,
    AptosRest,
    IcpRosetta,
    TronHttp,
    TrongridV1,
    Nearblocks,
    Insight,
    KaspaRest,
    BchRestV2,
}

impl EndpointApi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EvmJsonRpc => "evm-json-rpc",
            Self::SolanaJsonRpc => "solana-json-rpc",
            Self::SuiJsonRpc => "sui-json-rpc",
            Self::NearJsonRpc => "near-json-rpc",
            Self::XrplJsonRpc => "xrpl-json-rpc",
            Self::SubstrateJsonRpc => "substrate-json-rpc",
            Self::MoneroDaemonRpc => "monero-daemon-rpc",
            Self::Esplora => "esplora",
            Self::Blockbook => "blockbook",
            Self::Blockcypher => "blockcypher",
            Self::Whatsonchain => "whatsonchain",
            Self::Blockscout => "blockscout",
            Self::ToncenterV2 => "toncenter-v2",
            Self::ToncenterV3 => "toncenter-v3",
            Self::Koios => "koios",
            Self::Horizon => "horizon",
            Self::AptosRest => "aptos-rest",
            Self::IcpRosetta => "icp-rosetta",
            Self::TronHttp => "tron-http",
            Self::TrongridV1 => "trongrid-v1",
            Self::Nearblocks => "nearblocks",
            Self::Insight => "insight",
            Self::KaspaRest => "kaspa-rest",
            Self::BchRestV2 => "bch-rest-v2",
        }
    }

    /// An address indexer for the Bitcoin family, served by `api::utxo`.
    pub fn is_utxo_indexer(self) -> bool {
        matches!(
            self,
            Self::Esplora
                | Self::Blockbook
                | Self::Blockcypher
                | Self::Whatsonchain
                | Self::BchRestV2
        )
    }
}

/// One URL and the API it speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub api: EndpointApi,
    pub url: String,
}

/// What an endpoint is used *for*. A capability is a claim about the endpoint
/// that has to be true — see `no_evm_node_claims_history`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointCapability {
    /// The native coin's balance.
    Balance,
    /// The native coin's address history.
    History,
    /// Fungible-token transfers, not approvals or arbitrary contract activity.
    TokenHistory,
    /// Holdings enumerated without a caller-supplied token list.
    TokenDiscovery,
    /// A specified token's balance, including its metadata.
    TokenBalance,
    Utxo,
    Fee,
    Broadcast,
    Verification,
    Staking,
}

impl EndpointCapability {
    pub const ALL: [Self; 10] = [
        Self::Balance,
        Self::History,
        Self::TokenHistory,
        Self::TokenDiscovery,
        Self::TokenBalance,
        Self::Utxo,
        Self::Fee,
        Self::Broadcast,
        Self::Verification,
        Self::Staking,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Balance => "balance",
            Self::History => "history",
            Self::TokenHistory => "token-history",
            Self::TokenDiscovery => "token-discovery",
            Self::TokenBalance => "token-balance",
            Self::Utxo => "utxo",
            Self::Fee => "fee",
            Self::Broadcast => "broadcast",
            Self::Verification => "verification",
            Self::Staking => "staking",
        }
    }

    /// A chain's primary endpoint list holds only URLs that answer one of
    /// these; anything else there is an operation-only URL, not a base.
    pub(crate) fn is_primary_read(self) -> bool {
        matches!(self, Self::Balance | Self::Fee | Self::Broadcast)
    }
}

impl std::str::FromStr for EndpointCapability {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str() == value)
            .ok_or_else(|| format!("unknown endpoint capability {value:?}"))
    }
}

/// The capability's catalog name, such as `token-history`.
#[uniffi::export]
pub fn endpoint_capability_id(capability: EndpointCapability) -> String {
    capability.as_str().into()
}

/// A configured URL must speak one of the chain's APIs. If it is already in
/// the catalog, reject a known mismatch before retaining or sending to it.
pub(crate) fn validate_configured_endpoint(
    chain: crate::registry::Chain,
    url: &str,
) -> Result<(), String> {
    if url.is_empty() {
        return Ok(());
    }
    let expected = chain.endpoint_apis();
    let catalog = crate::app_core::endpoint_catalog()?;
    let matching: Vec<_> = catalog
        .endpoint_records
        .iter()
        .filter(|record| record.endpoint.trim_end_matches('/') == url.trim_end_matches('/'))
        .collect();
    if !matching.is_empty()
        && !matching.iter().any(|record| {
            expected.contains(&record.api)
                && record.capabilities.iter().any(|c| c.is_primary_read())
        })
    {
        let names: Vec<_> = expected.iter().map(|api| api.as_str()).collect();
        return Err(format!(
            "{} requires {} endpoints; {url} uses a different API",
            chain.str_id(),
            if names.is_empty() {
                "a supported API".to_string()
            } else {
                names.join(" or ")
            }
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Chain;

    #[test]
    fn known_incompatible_urls_are_rejected_before_configuration() {
        assert!(
            validate_configured_endpoint(Chain::Bitcoin, "https://ethereum-rpc.publicnode.com")
                .is_err()
        );
        assert!(
            validate_configured_endpoint(Chain::Bitcoin, "https://blockstream.info/api/").is_ok()
        );
        assert!(
            validate_configured_endpoint(Chain::Monero, "https://blockstream.info/api").is_err()
        );
    }

    #[test]
    fn capability_names_round_trip() {
        for capability in EndpointCapability::ALL {
            assert_eq!(capability.as_str().parse(), Ok(capability));
            assert_eq!(
                serde_json::to_value(capability).unwrap(),
                capability.as_str()
            );
        }
        assert!("native-history".parse::<EndpointCapability>().is_err());
    }
}

/// Operations implemented by Spectra's adapter. This is an editing constraint,
/// never a claim that any particular provider enables those operations.
#[uniffi::export]
pub fn endpoint_capability_options(
    chain: crate::registry::Chain,
    api: EndpointApi,
) -> Vec<EndpointCapability> {
    use EndpointApi::*;
    use EndpointCapability::{
        Balance, Broadcast, Fee, History, Staking, TokenBalance, TokenDiscovery, TokenHistory,
        Utxo, Verification,
    };
    let values: &[EndpointCapability] = match api {
        EvmJsonRpc => &[Balance, Fee, Broadcast, Verification, TokenBalance],
        SolanaJsonRpc => &[
            Balance,
            History,
            Fee,
            Broadcast,
            Verification,
            TokenBalance,
            TokenDiscovery,
            TokenHistory,
            Staking,
        ],
        SuiJsonRpc | AptosRest => &[
            Balance,
            History,
            Fee,
            Broadcast,
            Verification,
            TokenBalance,
            TokenDiscovery,
            Staking,
        ],
        NearJsonRpc => &[Balance, Fee, Broadcast, Verification, TokenBalance, Staking],
        XrplJsonRpc => &[Balance, History, Fee, Broadcast, Verification],
        TronHttp => &[Balance, Fee, Broadcast, Verification, TokenBalance],
        MoneroDaemonRpc => &[Fee, Broadcast, Verification],
        Esplora | Blockbook | Blockcypher | Whatsonchain | Insight => {
            &[Balance, History, Utxo, Fee, Broadcast, Verification]
        }
        KaspaRest => &[Balance, History, Utxo, Fee, Broadcast, Verification],
        BchRestV2 => &[Balance, History, Utxo, Broadcast, Verification],
        Blockscout => &[History, TokenHistory],
        ToncenterV2 => &[Balance, History, Fee, Broadcast, Verification, TokenBalance],
        ToncenterV3 => &[TokenBalance, TokenDiscovery],
        Koios => &[Balance, History, Utxo, Fee, Broadcast, Verification],
        Horizon => &[Balance, History, Fee, Broadcast, Verification],
        IcpRosetta => &[Balance, History, Fee, Broadcast, Verification],
        TrongridV1 => &[History, TokenHistory, TokenDiscovery],
        Nearblocks => &[History],
        SubstrateJsonRpc => &[Balance, Fee, Broadcast, Verification, Staking],
    };
    values
        .iter()
        .copied()
        .filter(|value| *value != Staking || chain.staking_uses_endpoint())
        .collect()
}
