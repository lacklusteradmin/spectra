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
    TronJsonRpc,
    MoneroWalletRpc,
    MoneroDaemonRpc,
    Esplora,
    Blockbook,
    Blockchair,
    BlockchainInfo,
    BchRestV2,
    Blockcypher,
    Whatsonchain,
    SochainV2,
    Blockscout,
    ToncenterV2,
    ToncenterV3,
    Koios,
    Horizon,
    AptosRest,
    IcpRosetta,
    TronHttp,
    TrongridV1,
    Tronscan,
    Xrpscan,
    Nearblocks,
    SubstrateSidecar,
    MoneroLightWallet,
    Insight,
    KaspaRest,
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
            Self::TronJsonRpc => "tron-json-rpc",
            Self::MoneroDaemonRpc => "monero-daemon-rpc",
            Self::MoneroWalletRpc => "monero-wallet-rpc",
            Self::Esplora => "esplora",
            Self::Blockbook => "blockbook",
            Self::Blockchair => "blockchair",
            Self::BlockchainInfo => "blockchain-info",
            Self::BchRestV2 => "bch-rest-v2",
            Self::Blockcypher => "blockcypher",
            Self::Whatsonchain => "whatsonchain",
            Self::SochainV2 => "sochain-v2",
            Self::Blockscout => "blockscout",
            Self::ToncenterV2 => "toncenter-v2",
            Self::ToncenterV3 => "toncenter-v3",
            Self::Koios => "koios",
            Self::Horizon => "horizon",
            Self::AptosRest => "aptos-rest",
            Self::IcpRosetta => "icp-rosetta",
            Self::TronHttp => "tron-http",
            Self::TrongridV1 => "trongrid-v1",
            Self::Tronscan => "tronscan",
            Self::Xrpscan => "xrpscan",
            Self::Nearblocks => "nearblocks",
            Self::SubstrateSidecar => "substrate-sidecar",
            Self::MoneroLightWallet => "monero-light-wallet",
            Self::Insight => "insight",
            Self::KaspaRest => "kaspa-rest",
        }
    }

    pub(crate) fn rpc_health_method(self) -> Option<&'static str> {
        match self {
            Self::EvmJsonRpc | Self::TronJsonRpc => Some("eth_chainId"),
            Self::SolanaJsonRpc => Some("getHealth"),
            Self::MoneroDaemonRpc => Some("get_info"),
            Self::SuiJsonRpc => Some("sui_getLatestCheckpointSequenceNumber"),
            Self::NearJsonRpc => Some("status"),
            Self::SubstrateJsonRpc => Some("chain_getHeader"),
            _ => None,
        }
    }
}

/// A custom URL is interpreted using its slot's API. If it is already in the
/// catalog, reject a known mismatch before retaining or sending to it.
pub(crate) fn validate_configured_endpoint(
    chain: crate::registry::Chain,
    slot: crate::registry::EndpointSlot,
    url: &str,
) -> Result<(), String> {
    if url.is_empty() {
        return Ok(());
    }
    let expected = chain.endpoint_api(slot);
    let catalog = crate::app_core::endpoint_catalog()?;
    let matching: Vec<_> = catalog
        .endpoint_records
        .iter()
        .filter(|record| record.endpoint.trim_end_matches('/') == url.trim_end_matches('/'))
        .collect();
    if !matching.is_empty()
        && !matching.iter().any(|record| {
            record.api == expected
                && expected.is_some()
                && (slot != crate::registry::EndpointSlot::Primary
                    || record
                        .capabilities
                        .iter()
                        .any(|c| matches!(c.as_str(), "balance" | "fee" | "broadcast")))
        })
    {
        return Err(format!(
            "{} requires {} endpoints; {url} uses a different API",
            chain.str_id(),
            expected
                .map(EndpointApi::as_str)
                .unwrap_or("a supported API")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Chain, EndpointSlot};

    #[test]
    fn known_incompatible_urls_are_rejected_before_configuration() {
        assert!(validate_configured_endpoint(
            Chain::Bitcoin,
            EndpointSlot::Primary,
            "https://blockchain.info/multiaddr"
        )
        .is_err());
        assert!(validate_configured_endpoint(
            Chain::Bitcoin,
            EndpointSlot::Primary,
            "https://blockstream.info/api/"
        )
        .is_ok());
        assert!(validate_configured_endpoint(
            Chain::Monero,
            EndpointSlot::Primary,
            "https://blockstream.info/api"
        )
        .is_err());
    }
}

/// Operations implemented by Spectra's adapter. This is an editing constraint,
/// never a claim that any particular provider enables those operations.
#[uniffi::export]
pub fn endpoint_capability_options(chain_id: String, api: EndpointApi) -> Vec<String> {
    use EndpointApi::*;
    let Some(chain) = crate::registry::Chain::from_str_id(&chain_id) else {
        return vec![];
    };
    let values: &[&str] = match api {
        EvmJsonRpc => &[
            "balance",
            "fee",
            "broadcast",
            "verification",
            "token-balance",
        ],
        SolanaJsonRpc => &[
            "balance",
            "history",
            "fee",
            "broadcast",
            "verification",
            "token-balance",
            "token-discovery",
            "token-history",
            "staking",
        ],
        SuiJsonRpc | AptosRest => &[
            "balance",
            "history",
            "fee",
            "broadcast",
            "verification",
            "token-balance",
            "token-discovery",
            "staking",
        ],
        NearJsonRpc => &[
            "balance",
            "fee",
            "broadcast",
            "verification",
            "token-balance",
            "staking",
        ],
        XrplJsonRpc => &["balance", "history", "fee", "broadcast", "verification"],
        TronHttp => &[
            "balance",
            "fee",
            "broadcast",
            "verification",
            "token-balance",
        ],
        MoneroDaemonRpc => &["fee", "broadcast", "verification"],
        Esplora | Blockbook | Blockcypher | Whatsonchain | Insight => &[
            "balance",
            "history",
            "utxo",
            "fee",
            "broadcast",
            "verification",
        ],
        KaspaRest => &[
            "balance",
            "history",
            "utxo",
            "fee",
            "broadcast",
            "verification",
        ],
        Blockscout => &["history", "token-history"],
        ToncenterV2 => &[
            "balance",
            "history",
            "fee",
            "broadcast",
            "verification",
            "token-balance",
        ],
        ToncenterV3 => &["token-balance", "token-discovery"],
        Koios => &[
            "balance",
            "history",
            "utxo",
            "fee",
            "broadcast",
            "verification",
        ],
        Horizon => &["balance", "history", "fee", "broadcast", "verification"],
        IcpRosetta => &["balance", "history", "fee", "broadcast", "verification"],
        TrongridV1 => &["token-discovery"],
        Tronscan => &["history", "token-history"],
        Nearblocks => &["history"],
        SubstrateJsonRpc => &["fee", "broadcast", "verification", "staking"],
        // Catalog reference APIs without a production request adapter.
        TronJsonRpc | MoneroWalletRpc | MoneroLightWallet | Blockchair | BlockchainInfo
        | BchRestV2 | SochainV2 | Xrpscan | SubstrateSidecar => &[],
    };
    values
        .iter()
        .filter(|value| **value != "staking" || chain.staking_uses_endpoint())
        .map(|s| (*s).to_owned())
        .collect()
}
