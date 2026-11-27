//! Proving that an endpoint serves the network it is configured for, through
//! the client of the API it speaks: what the health probe reports and what
//! core requires before it submits through an endpoint.

use std::sync::Arc;

use crate::api::endpoint::{Endpoint, EndpointApi};
use crate::api::error::ApiError;
use crate::registry::Chain;

/// Whether an endpoint of `api` on `chain` can be asked which network it
/// serves. A Bitcoin-family indexer can where the registry names the
/// network's identity block and the indexer names blocks by height.
pub(crate) fn asks_network(chain: Chain, api: EndpointApi) -> bool {
    use EndpointApi as Api;
    match api {
        Api::Blockbook if chain.mainnet_counterpart() == Chain::Peercoin => true,
        Api::Esplora | Api::Blockbook | Api::Blockcypher | Api::Chronik | Api::UnisatOpenApi => {
            chain.identity_block().is_ok()
        }
        Api::Whatsonchain
        | Api::BchRestV2
        | Api::Insight
        | Api::KaspaRest
        | Api::Blockscout
        | Api::TrongridV1
        | Api::Nearblocks
        | Api::Fastnear
        | Api::Filfox
        | Api::IcpReplica => false,
        _ => true,
    }
}

/// Check that `url`, speaking `api`, answers as `chain`.
///
/// `Ok(false)` when the API gives core no way to ask the network who it is
/// ([`asks_network`]); the caller decides whether an endpoint it cannot
/// verify is acceptable.
pub(crate) async fn verify_network(
    chain: Chain,
    api: EndpointApi,
    url: &str,
) -> Result<bool, ApiError> {
    use crate::api::*;
    use EndpointApi as Api;
    if !asks_network(chain, api) {
        return Ok(false);
    }
    let urls = Arc::new(vec![url.to_string()]);
    match api {
        Api::EvmJsonRpc => {
            evm_json_rpc::EvmClient::new(urls, chain.evm_chain_id()?)
                .verify_network()
                .await?
        }
        Api::CosmosRest => {
            cosmos_rest::CosmosClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::SubstrateJsonRpc => {
            substrate_json_rpc::SubstrateClient::new(urls)
                .polkadot_context(chain)
                .await?;
        }
        // The address book's own mirror node.
        Api::HederaHapi => {
            hedera_hapi::HederaHapiClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::HederaMirror => {
            hedera_mirror::HederaMirrorClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::SuiGraphql => {
            sui_graphql::SuiClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::SolanaJsonRpc => {
            solana_json_rpc::SolanaClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::NearJsonRpc => {
            near_json_rpc::NearClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::Horizon => {
            horizon::HorizonClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::XrplJsonRpc => {
            xrpl_json_rpc::XrplClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::Koios => koios::KoiosClient::new(urls).verify_network(chain).await?,
        Api::ThorRest => {
            thor_rest::ThorClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::TronHttp => {
            tron_http::TronHttpClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::ToncenterV2 => {
            toncenter_v2::ToncenterV2Client::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::ToncenterV3 => {
            toncenter_v3::ToncenterV3Client::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::AptosRest => {
            aptos_rest::AptosClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::AptosIndexer => {
            aptos_indexer::AptosIndexerClient::new(urls, chain.aptos_chain_id()?)
                .verify_network()
                .await?
        }
        Api::StarknetJsonRpc => {
            starknet_json_rpc::StarknetClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::Blockbook if chain.mainnet_counterpart() == Chain::Peercoin => {
            blockbook::BlockbookClient::new(urls, chain)
                .verify_peercoin_network()
                .await?
        }
        // The indexer names the network's identity block: its genesis block,
        // or a later one where networks share it.
        Api::Esplora | Api::Blockbook | Api::Blockcypher | Api::Chronik | Api::UnisatOpenApi => {
            utxo::UtxoClient::new(
                chain,
                vec![Endpoint {
                    api,
                    url: url.to_string(),
                }],
            )
            .verify_every_endpoint()
            .await?
        }
        Api::IcpRosetta => icp_rosetta::IcpClient::new(urls).verify_network().await?,
        Api::Algod => algod::AlgodClient::new(urls).verify_network(chain).await?,
        Api::AlgorandIndexer => {
            algorand_indexer::AlgorandIndexerClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::LotusJsonRpc => {
            lotus_json_rpc::LotusClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::TezosRpc => {
            tezos_rpc::TezosClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Api::Tzkt => tzkt::TzktClient::new(urls).verify_network(chain).await?,
        Api::MoneroDaemonRpc => {
            monero_daemon_rpc::daemon(url, chain).await?;
        }
        // The server's chain name and consensus branch.
        Api::Lightwalletd => {
            lightwalletd::LightwalletdClient::new(urls)
                .session(chain)
                .await?;
        }
        // The node's headers, checked back to the network's checkpoint.
        Api::LitecoinP2p => {
            litecoin_p2p::LitecoinP2pClient::new(urls)
                .session(chain, &Default::default())
                .await?;
        }
        Api::Whatsonchain
        | Api::BchRestV2
        | Api::Insight
        | Api::KaspaRest
        | Api::Blockscout
        | Api::TrongridV1
        | Api::Nearblocks
        | Api::Fastnear
        | Api::Filfox
        | Api::IcpReplica => return Ok(false),
    }
    Ok(true)
}
