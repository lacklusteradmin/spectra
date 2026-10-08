//! A lightwalletd server: the compact blocks, note commitment trees and
//! transparent outputs a Zcash light wallet reads, and taking its
//! transactions. One gRPC service (`CompactTxStreamer`) over `api::grpc`.
//!
//! A sync session talks to one server: the first that connects and says it
//! is on the wallet's network, so the blocks, trees and tip it reads agree.

use std::sync::Arc;

use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::{
    self, compact_tx_streamer_client::CompactTxStreamerClient,
};
use zcash_protocol::consensus::{NetworkUpgrade, Parameters};

use crate::api::error::ApiError;
use crate::registry::Chain;

pub(crate) use service::{GetAddressUtxosReply, ShieldedProtocol, SubtreeRoot, TreeState};

/// What one connected server answers.
pub(crate) struct LightwalletdSession {
    client: CompactTxStreamerClient<tonic::transport::Channel>,
    /// The chain tip when the session began.
    pub tip: u32,
    /// The consensus branch the server says the next block is under.
    pub branch: u32,
}

/// A chain's lightwalletd servers.
pub struct LightwalletdClient {
    endpoints: Arc<Vec<String>>,
}

fn status(error: tonic::Status) -> ApiError {
    use tonic::Code;
    match error.code() {
        Code::Unavailable | Code::DeadlineExceeded | Code::ResourceExhausted | Code::Aborted => {
            ApiError::Transport(format!("lightwalletd: {}", error.message()))
        }
        _ => ApiError::Rejected(format!("lightwalletd: {}", error.message())),
    }
}

impl LightwalletdClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self { endpoints }
    }

    /// A session on the first server that connects and is on `chain`'s
    /// network, with the tip it reports. A server on another network, or
    /// one whose Sapling activation is not the network's, is skipped.
    pub(crate) async fn session(&self, chain: Chain) -> Result<LightwalletdSession, ApiError> {
        let network = chain.zcash_network()?;
        let expected_name = match network.network_type() {
            zcash_protocol::consensus::NetworkType::Main => "main",
            zcash_protocol::consensus::NetworkType::Test => "test",
            zcash_protocol::consensus::NetworkType::Regtest => "regtest",
        };
        let sapling = network
            .activation_height(NetworkUpgrade::Sapling)
            .map(u64::from)
            .unwrap_or_default();
        let mut last = ApiError::NoEndpoint;
        for endpoint in self.endpoints.iter() {
            let attempt = async {
                let mut client =
                    CompactTxStreamerClient::new(crate::api::grpc::channel(endpoint).await?)
                        .max_decoding_message_size(64 << 20);
                let info = client
                    .get_lightd_info(service::Empty {})
                    .await
                    .map_err(status)?
                    .into_inner();
                if info.chain_name != expected_name || info.sapling_activation_height != sapling {
                    return Err(ApiError::Decode(format!(
                        "{endpoint} serves the {} network, not {}",
                        info.chain_name,
                        chain.chain_display_name()
                    )));
                }
                let tip = client
                    .get_latest_block(service::ChainSpec {})
                    .await
                    .map_err(status)?
                    .into_inner()
                    .height;
                let tip = u32::try_from(tip).map_err(|_| ApiError::decode("tip height"))?;
                let branch = u32::from_str_radix(&info.consensus_branch_id, 16)
                    .map_err(|_| ApiError::decode("lightwalletd: consensus branch"))?;
                Ok(LightwalletdSession {
                    client,
                    tip,
                    branch,
                })
            };
            match attempt.await {
                Ok(session) => return Ok(session),
                Err(error) => last = error,
            }
        }
        Err(last)
    }
}

impl LightwalletdSession {
    /// The note commitment trees as of the end of block `height`.
    pub(crate) async fn tree_state(&mut self, height: u32) -> Result<TreeState, ApiError> {
        self.client
            .get_tree_state(service::BlockId {
                height: u64::from(height),
                hash: vec![],
            })
            .await
            .map(tonic::Response::into_inner)
            .map_err(status)
    }

    /// Every completed subtree root of `protocol`'s note commitment tree
    /// from `start_index`.
    pub(crate) async fn subtree_roots(
        &mut self,
        protocol: ShieldedProtocol,
        start_index: u32,
    ) -> Result<Vec<SubtreeRoot>, ApiError> {
        let mut request = service::GetSubtreeRootsArg {
            start_index,
            ..Default::default()
        };
        request.set_shielded_protocol(protocol);
        let mut stream = self
            .client
            .get_subtree_roots(request)
            .await
            .map_err(status)?
            .into_inner();
        let mut roots = Vec::new();
        while let Some(root) = stream.message().await.map_err(status)? {
            roots.push(root);
        }
        Ok(roots)
    }

    /// The compact blocks from `start` through `end`, in order.
    pub(crate) async fn blocks(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<Vec<CompactBlock>, ApiError> {
        let mut stream = self
            .client
            .get_block_range(service::BlockRange {
                start: Some(service::BlockId {
                    height: u64::from(start),
                    hash: vec![],
                }),
                end: Some(service::BlockId {
                    height: u64::from(end),
                    hash: vec![],
                }),
                pool_types: vec![],
            })
            .await
            .map_err(status)?
            .into_inner();
        let mut blocks = Vec::new();
        while let Some(block) = stream.message().await.map_err(status)? {
            if block.height != u64::from(start) + blocks.len() as u64 {
                return Err(ApiError::decode("lightwalletd: blocks out of order"));
            }
            blocks.push(block);
        }
        if blocks.len() as u64 != u64::from(end) - u64::from(start) + 1 {
            return Err(ApiError::decode(
                "lightwalletd: blocks missing from the range",
            ));
        }
        Ok(blocks)
    }

    /// The unspent transparent outputs of `addresses` from `start_height`.
    pub(crate) async fn utxos(
        &mut self,
        addresses: Vec<String>,
        start_height: u32,
    ) -> Result<Vec<GetAddressUtxosReply>, ApiError> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .client
            .get_address_utxos(service::GetAddressUtxosArg {
                addresses,
                start_height: u64::from(start_height),
                max_entries: 0,
            })
            .await
            .map_err(status)?
            .into_inner()
            .address_utxos)
    }

    /// A transaction by id, with the height it was mined at; `None` when
    /// the server does not know it.
    pub(crate) async fn transaction(
        &mut self,
        txid: [u8; 32],
    ) -> Result<Option<(Vec<u8>, Option<u32>)>, ApiError> {
        match self
            .client
            .get_transaction(service::TxFilter {
                block: None,
                index: 0,
                hash: txid.to_vec(),
            })
            .await
        {
            Ok(raw) => {
                let raw = raw.into_inner();
                // lightwalletd reports an unmined transaction's height as 0,
                // or as u64::MAX when it is not in the mempool either.
                let height = u32::try_from(raw.height).ok().filter(|height| *height > 0);
                Ok(Some((raw.data, height)))
            }
            Err(error) if error.code() == tonic::Code::NotFound => Ok(None),
            Err(error) => Err(status(error)),
        }
    }

    /// Hand a signed transaction to the server, which relays it to the
    /// network.
    pub(crate) async fn submit(&mut self, raw: Vec<u8>) -> Result<(), ApiError> {
        let response = self
            .client
            .send_transaction(service::RawTransaction {
                data: raw,
                height: 0,
            })
            .await
            .map_err(status)?
            .into_inner();
        if response.error_code != 0 {
            return Err(ApiError::Rejected(format!(
                "lightwalletd refused the transaction ({}): {}",
                response.error_code, response.error_message
            )));
        }
        Ok(())
    }
}
