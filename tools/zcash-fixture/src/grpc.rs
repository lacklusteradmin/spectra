//! `cash.z.wallet.sdk.rpc.CompactTxStreamer`, served by hand: zcash_client_backend
//! generates only the client, so each method here is the routing a generated
//! server would do, over the same codec.

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures::Stream;
use tonic::body::Body;
use tonic::server::{Grpc, NamedService, ServerStreamingService, UnaryService};
use tonic::{Request, Response, Status};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::{
    BlockId, BlockRange, ChainSpec, Empty, GetAddressUtxosArg, GetAddressUtxosReply,
    GetAddressUtxosReplyList, GetSubtreeRootsArg, LightdInfo, RawTransaction, SendResponse,
    SubtreeRoot, TreeState, TxFilter,
};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::consensus::{NetworkType, NetworkUpgrade, Parameters};

use crate::chain::Chain;

type BoxFuture<T> = Pin<Box<dyn std::future::Future<Output = T> + Send>>;
type Answer<T> = BoxFuture<Result<Response<T>, Status>>;
type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

/// The served chain and what it records.
pub struct State {
    pub chain: Chain,
    pub accounts: HashMap<&'static str, UnifiedFullViewingKey>,
    pub journal: Option<std::path::PathBuf>,
    /// Empty blocks mined after each accepted transaction, so its outputs
    /// are confirmed for the next step.
    pub confirmations: u32,
}

#[derive(Clone)]
pub struct Streamer(pub Arc<Mutex<State>>);

impl NamedService for Streamer {
    const NAME: &'static str = "cash.z.wallet.sdk.rpc.CompactTxStreamer";
}

struct Unary<F>(F);

impl<Req, Resp, F> UnaryService<Req> for Unary<F>
where
    F: FnMut(Req) -> Result<Resp, Status>,
    Resp: Send + 'static,
{
    type Response = Resp;
    type Future = Answer<Resp>;

    fn call(&mut self, request: Request<Req>) -> Self::Future {
        let answer = (self.0)(request.into_inner()).map(Response::new);
        Box::pin(async move { answer })
    }
}

struct Streaming<F>(F);

impl<Req, Item, F> ServerStreamingService<Req> for Streaming<F>
where
    F: FnMut(Req) -> Result<Vec<Item>, Status>,
    Item: Send + 'static,
{
    type Response = Item;
    type ResponseStream = BoxStream<Item>;
    type Future = Answer<Self::ResponseStream>;

    fn call(&mut self, request: Request<Req>) -> Self::Future {
        let answer = (self.0)(request.into_inner()).map(|items| {
            Response::new(
                Box::pin(futures::stream::iter(items.into_iter().map(Ok))) as BoxStream<Item>
            )
        });
        Box::pin(async move { answer })
    }
}

fn hex_be(bytes: &[u8]) -> String {
    let mut reversed = bytes.to_vec();
    reversed.reverse();
    hex::encode(reversed)
}

impl Streamer {
    fn info(&self) -> Result<LightdInfo, Status> {
        let state = self.0.lock().unwrap();
        let chain = &state.chain;
        Ok(LightdInfo {
            version: "spectra-fixture".into(),
            vendor: "spectra".into(),
            taddr_support: true,
            chain_name: match chain.network.network_type() {
                NetworkType::Main => "main",
                NetworkType::Test => "test",
                NetworkType::Regtest => "regtest",
            }
            .into(),
            sapling_activation_height: chain
                .network
                .activation_height(NetworkUpgrade::Sapling)
                .map_or(0, u64::from),
            consensus_branch_id: format!("{:08x}", u32::from(chain.branch())),
            block_height: u64::from(chain.tip()),
            ..Default::default()
        })
    }

    fn latest(&self) -> Result<BlockId, Status> {
        let state = self.0.lock().unwrap();
        let tip = state.chain.tip();
        Ok(BlockId {
            height: u64::from(tip),
            hash: state
                .chain
                .block(tip)
                .map(|b| b.hash.clone())
                .unwrap_or_default(),
        })
    }

    fn range(&self, range: BlockRange) -> Result<Vec<CompactBlock>, Status> {
        let state = self.0.lock().unwrap();
        let start = range.start.map_or(0, |b| b.height) as u32;
        let end = range.end.map_or(0, |b| b.height) as u32;
        if end > state.chain.tip() || start < state.chain.start || start > end {
            return Err(Status::out_of_range("block range outside the chain"));
        }
        Ok((start..=end)
            .filter_map(|height| state.chain.block(height).cloned())
            .collect())
    }

    fn tree_state(&self, block: BlockId) -> Result<TreeState, Status> {
        let state = self.0.lock().unwrap();
        let height = block.height as u32;
        if height > state.chain.tip() {
            return Err(Status::out_of_range("past the tip"));
        }
        let trees = state.chain.trees_at(height);
        let write = |tree: &dyn Fn(&mut Vec<u8>) -> std::io::Result<()>| {
            let mut bytes = Vec::new();
            tree(&mut bytes).map(|_| hex::encode(bytes))
        };
        let sapling =
            write(&|out| zcash_primitives::merkle_tree::write_commitment_tree(&trees.sapling, out))
                .map_err(|e| Status::internal(e.to_string()))?;
        let orchard =
            write(&|out| zcash_primitives::merkle_tree::write_commitment_tree(&trees.orchard, out))
                .map_err(|e| Status::internal(e.to_string()))?;
        let ironwood = write(&|out| {
            zcash_primitives::merkle_tree::write_commitment_tree(&trees.ironwood, out)
        })
        .map_err(|e| Status::internal(e.to_string()))?;
        let block = state.chain.block(height);
        Ok(TreeState {
            network: self.info_unlocked(&state.chain),
            height: u64::from(height),
            hash: block
                .map(|b| hex_be(&b.hash))
                .unwrap_or_else(|| "00".repeat(32)),
            time: block.map_or(0, |b| b.time),
            sapling_tree: sapling,
            orchard_tree: orchard,
            ironwood_tree: ironwood,
        })
    }

    fn info_unlocked(&self, chain: &Chain) -> String {
        match chain.network.network_type() {
            NetworkType::Main => "main",
            NetworkType::Test => "test",
            NetworkType::Regtest => "regtest",
        }
        .into()
    }

    fn utxos(&self, arg: GetAddressUtxosArg) -> Result<GetAddressUtxosReplyList, Status> {
        let state = self.0.lock().unwrap();
        Ok(GetAddressUtxosReplyList {
            address_utxos: state
                .chain
                .utxos
                .iter()
                .filter(|utxo| arg.addresses.contains(&utxo.address))
                .filter(|utxo| u64::from(utxo.height) >= arg.start_height)
                .map(|utxo| GetAddressUtxosReply {
                    address: utxo.address.clone(),
                    txid: utxo.txid.to_vec(),
                    index: utxo.index as i32,
                    script: utxo.script.clone(),
                    value_zat: utxo.value as i64,
                    height: u64::from(utxo.height),
                })
                .collect(),
        })
    }

    fn transaction(&self, filter: TxFilter) -> Result<RawTransaction, Status> {
        let state = self.0.lock().unwrap();
        let txid: [u8; 32] = filter
            .hash
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("txid"))?;
        let (data, height) = state
            .chain
            .transactions
            .get(&txid)
            .cloned()
            .ok_or_else(|| Status::not_found("transaction not found"))?;
        Ok(RawTransaction {
            data,
            height: u64::from(height),
        })
    }

    /// Check a submitted transaction as a node would; mine it and the empty
    /// blocks that confirm it, and journal what it did.
    fn submit(&self, raw: RawTransaction) -> Result<SendResponse, Status> {
        let mut state = self.0.lock().unwrap();
        let branch = state.chain.branch();
        let refused = |message: String| SendResponse {
            error_code: -26,
            error_message: message,
        };
        let tx = match zcash_primitives::transaction::Transaction::read(&raw.data[..], branch) {
            Ok(tx) => tx,
            Err(error) => return Ok(refused(format!("unreadable transaction: {error}"))),
        };
        let summary = match crate::verify::check(&state.chain, &tx, &state.accounts) {
            Ok(summary) => summary,
            Err(error) => return Ok(refused(error)),
        };
        state.chain.mine(vec![tx]);
        for _ in 0..state.confirmations {
            state.chain.mine(vec![]);
        }
        if let Some(path) = &state.journal {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| Status::internal(e.to_string()))?;
            writeln!(file, "{summary}").map_err(|e| Status::internal(e.to_string()))?;
        }
        Ok(SendResponse {
            error_code: 0,
            error_message: String::new(),
        })
    }
}

impl tower::Service<http::Request<Body>> for Streamer {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Result<Self::Response, Infallible>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let this = self.clone();
        let method = request
            .uri()
            .path()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        Box::pin(async move {
            let response = match method.as_str() {
                "GetLightdInfo" => {
                    let s = this.clone();
                    Grpc::new(tonic_prost::ProstCodec::default())
                        .unary(Unary(move |_: Empty| s.info()), request)
                        .await
                }
                "GetLatestBlock" => {
                    let s = this.clone();
                    Grpc::new(tonic_prost::ProstCodec::default())
                        .unary(Unary(move |_: ChainSpec| s.latest()), request)
                        .await
                }
                "GetBlockRange" => {
                    let s = this.clone();
                    Grpc::new(tonic_prost::ProstCodec::default())
                        .server_streaming(
                            Streaming(move |range: BlockRange| s.range(range)),
                            request,
                        )
                        .await
                }
                "GetTreeState" => {
                    let s = this.clone();
                    Grpc::new(tonic_prost::ProstCodec::default())
                        .unary(Unary(move |block: BlockId| s.tree_state(block)), request)
                        .await
                }
                "GetSubtreeRoots" => {
                    Grpc::new(tonic_prost::ProstCodec::default())
                        .server_streaming(
                            Streaming(|_: GetSubtreeRootsArg| Ok(Vec::<SubtreeRoot>::new())),
                            request,
                        )
                        .await
                }
                "GetAddressUtxos" => {
                    let s = this.clone();
                    Grpc::new(tonic_prost::ProstCodec::default())
                        .unary(Unary(move |arg: GetAddressUtxosArg| s.utxos(arg)), request)
                        .await
                }
                "GetTransaction" => {
                    let s = this.clone();
                    Grpc::new(tonic_prost::ProstCodec::default())
                        .unary(
                            Unary(move |filter: TxFilter| s.transaction(filter)),
                            request,
                        )
                        .await
                }
                "SendTransaction" => {
                    let s = this.clone();
                    Grpc::new(tonic_prost::ProstCodec::default())
                        .unary(Unary(move |raw: RawTransaction| s.submit(raw)), request)
                        .await
                }
                _ => Status::unimplemented(method).into_http(),
            };
            Ok(response)
        })
    }
}
