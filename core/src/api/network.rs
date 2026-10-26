//! Proving that an endpoint serves the network it is configured for, before
//! core signs or submits through it.

use std::sync::Arc;

use crate::api::endpoint::{Endpoint, EndpointApi};
use crate::api::error::ApiError;
use crate::registry::Chain;

/// Check that `url` answers as `chain`, through the client of the API that
/// network is read with. `api` is what the endpoint is configured to speak,
/// which only the UTXO family's indexers need to know.
///
/// `Ok(false)` when core has no way to ask the network who it is; the caller
/// decides whether an endpoint it cannot verify is acceptable.
pub(crate) async fn verify_network(
    chain: Chain,
    api: Option<EndpointApi>,
    url: &str,
) -> Result<bool, ApiError> {
    use crate::api::*;
    let urls = Arc::new(vec![url.to_string()]);
    if chain.is_evm() {
        evm_json_rpc::EvmClient::new(urls, chain.evm_chain_id()?)
            .verify_network()
            .await?;
        return Ok(true);
    }
    match chain.mainnet_counterpart() {
        Chain::Polkadot | Chain::Bittensor => {
            substrate_json_rpc::SubstrateClient::new(urls)
                .polkadot_context(chain)
                .await?;
        }
        Chain::Sui => {
            sui_json_rpc::SuiClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Solana => {
            solana_json_rpc::SolanaClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Near => {
            near_json_rpc::NearClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Stellar => {
            horizon::HorizonClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Xrp => {
            xrpl_json_rpc::XrplClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Cardano => koios::KoiosClient::new(urls).verify_network(chain).await?,
        Chain::Tron => {
            tron_http::TronHttpClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Ton => {
            toncenter_v2::ToncenterV2Client::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Aptos => {
            aptos_rest::AptosClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Zcash => {
            blockbook::BlockbookClient::new(urls, chain)
                .zcash_context()
                .await?;
        }
        Chain::Peercoin => {
            blockbook::BlockbookClient::new(urls, chain)
                .verify_peercoin_network()
                .await?
        }
        // The indexer names the network's genesis block at height 0.
        Chain::Bitcoin | Chain::Litecoin => {
            let api =
                api.ok_or_else(|| ApiError::invalid("Endpoint is not this network's indexer"))?;
            let client = utxo::UtxoClient::new(
                chain,
                vec![Endpoint {
                    api,
                    url: url.to_string(),
                }],
            );
            if client.fetch_block_hash(0).await? != chain.genesis_block_hash()? {
                return Err(ApiError::invalid("Endpoint is on the wrong network"));
            }
        }
        Chain::Icp => icp_rosetta::IcpClient::new(urls).verify_network().await?,
        Chain::Monero => {
            monero_daemon_rpc::daemon(url, chain).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
