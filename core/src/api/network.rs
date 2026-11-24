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
    if chain.is_cosmos() {
        cosmos_rest::CosmosClient::new(urls)
            .verify_network(chain)
            .await?;
        return Ok(true);
    }
    if chain.is_substrate() {
        substrate_json_rpc::SubstrateClient::new(urls)
            .polkadot_context(chain)
            .await?;
        return Ok(true);
    }
    match chain.mainnet_counterpart() {
        // A mirror node, or the mirror node an address book belongs to.
        Chain::Hedera if api == Some(EndpointApi::HederaHapi) => {
            hedera_hapi::HederaHapiClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Hedera => {
            hedera_mirror::HederaMirrorClient::new(urls)
                .verify_network(chain)
                .await?
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
        Chain::Vechain => {
            thor_rest::ThorClient::new(urls)
                .verify_network(chain)
                .await?
        }
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
        Chain::Starknet => {
            starknet_json_rpc::StarknetClient::new(urls)
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
        // The indexer names the network's identity block: its genesis
        // block, or a later one where networks share it.
        Chain::Bitcoin
        | Chain::Litecoin
        | Chain::Qtum
        | Chain::FractalBitcoin
        | Chain::Ecash
        | Chain::Digibyte => {
            let api =
                api.ok_or_else(|| ApiError::invalid("Endpoint is not this network's indexer"))?;
            utxo::UtxoClient::new(
                chain,
                vec![Endpoint {
                    api,
                    url: url.to_string(),
                }],
            )
            .verify_every_endpoint()
            .await?;
        }
        Chain::Icp => icp_rosetta::IcpClient::new(urls).verify_network().await?,
        Chain::Algorand => algod::AlgodClient::new(urls).verify_network(chain).await?,
        Chain::Filecoin => {
            lotus_json_rpc::LotusClient::new(urls)
                .verify_network(chain)
                .await?
        }
        Chain::Tezos => match api {
            Some(EndpointApi::Tzkt) => tzkt::TzktClient::new(urls).verify_network(chain).await?,
            _ => {
                tezos_rpc::TezosClient::new(urls)
                    .verify_network(chain)
                    .await?
            }
        },
        Chain::Monero => {
            monero_daemon_rpc::daemon(url, chain).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
