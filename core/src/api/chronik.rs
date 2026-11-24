//! Chronik, the indexer built into eCash's node (Bitcoin ABC): an address's
//! outputs and history, blocks, transactions and broadcast, over HTTP in
//! protobuf (`chronik/chronik-proto/proto/chronik.proto`). Only the fields
//! read here are declared; prost skips the rest.
//!
//! Chronik indexes scripts rather than addresses, so an address is asked
//! for as the P2PKH or P2SH script it pays. Hashes travel in wire order and
//! are reversed into the order explorers write them.
use std::sync::Arc;

use prost::Message;

use crate::api::error::{ApiError, OrDecode};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::utxo::{Utxo, UtxoBalance, UtxoHistoryEntry, UtxoStatus, UtxoTxStatus};
use crate::registry::Chain;

/// The largest answer read: a page of history or an address's outputs.
const MAX_ANSWER_BYTES: usize = 16 * 1024 * 1024;
/// Transactions per history page; Chronik serves at most 200.
const PAGE_SIZE: u32 = 50;

#[derive(Clone, PartialEq, Message)]
struct BlockchainInfo {
    #[prost(bytes = "vec", tag = "1")]
    tip_hash: Vec<u8>,
    #[prost(int32, tag = "2")]
    tip_height: i32,
}

#[derive(Clone, PartialEq, Message)]
struct Block {
    #[prost(message, optional, tag = "1")]
    block_info: Option<BlockInfo>,
}

#[derive(Clone, PartialEq, Message)]
struct BlockInfo {
    #[prost(bytes = "vec", tag = "1")]
    hash: Vec<u8>,
    #[prost(int32, tag = "3")]
    height: i32,
}

#[derive(Clone, PartialEq, Message)]
struct ScriptUtxos {
    #[prost(bytes = "vec", tag = "1")]
    script: Vec<u8>,
    #[prost(message, repeated, tag = "2")]
    utxos: Vec<ScriptUtxo>,
}

#[derive(Clone, PartialEq, Message)]
struct ScriptUtxo {
    #[prost(message, optional, tag = "1")]
    outpoint: Option<OutPoint>,
    /// The height of the block holding it, or -1 in the mempool.
    #[prost(int32, tag = "2")]
    block_height: i32,
    #[prost(bool, tag = "3")]
    is_coinbase: bool,
    #[prost(int64, tag = "5")]
    sats: i64,
    /// An eToken (SLP or ALP) amount the output carries.
    #[prost(message, optional, tag = "11")]
    token: Option<TokenPresence>,
}

/// A token value attached to an output: only whether it is there matters.
#[derive(Clone, PartialEq, Message)]
struct TokenPresence {}

#[derive(Clone, PartialEq, Message)]
struct OutPoint {
    #[prost(bytes = "vec", tag = "1")]
    txid: Vec<u8>,
    #[prost(uint32, tag = "2")]
    out_idx: u32,
}

#[derive(Clone, PartialEq, Message)]
struct TxHistoryPage {
    #[prost(message, repeated, tag = "1")]
    txs: Vec<Tx>,
    #[prost(uint32, tag = "2")]
    num_pages: u32,
    #[prost(uint32, tag = "3")]
    num_txs: u32,
}

#[derive(Clone, PartialEq, Message)]
struct Tx {
    #[prost(bytes = "vec", tag = "1")]
    txid: Vec<u8>,
    #[prost(message, repeated, tag = "3")]
    inputs: Vec<TxInput>,
    #[prost(message, repeated, tag = "4")]
    outputs: Vec<TxOutput>,
    /// The block holding it; absent while it is in the mempool.
    #[prost(message, optional, tag = "8")]
    block: Option<BlockMetadata>,
    #[prost(bool, tag = "12")]
    is_coinbase: bool,
}

#[derive(Clone, PartialEq, Message)]
struct TxInput {
    /// The script of the output it spends.
    #[prost(bytes = "vec", tag = "3")]
    output_script: Vec<u8>,
    #[prost(int64, tag = "4")]
    sats: i64,
}

#[derive(Clone, PartialEq, Message)]
struct TxOutput {
    #[prost(int64, tag = "1")]
    sats: i64,
    #[prost(bytes = "vec", tag = "2")]
    output_script: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct BlockMetadata {
    #[prost(int32, tag = "1")]
    height: i32,
    #[prost(int64, tag = "3")]
    timestamp: i64,
}

#[derive(Clone, PartialEq, Message)]
struct BroadcastTxRequest {
    #[prost(bytes = "vec", tag = "1")]
    raw_tx: Vec<u8>,
    /// Chronik refuses a transaction that burns eTokens unless this is set.
    #[prost(bool, tag = "2")]
    skip_token_checks: bool,
}

#[derive(Clone, PartialEq, Message)]
struct BroadcastTxResponse {
    #[prost(bytes = "vec", tag = "1")]
    txid: Vec<u8>,
}

/// What Chronik answers a request it refuses with.
#[derive(Clone, PartialEq, Message)]
struct ErrorMessage {
    #[prost(string, tag = "2")]
    msg: String,
}

/// A 32-byte hash in wire order, written as explorers write it.
fn display_hash(bytes: &[u8]) -> Result<String, ApiError> {
    let mut hash: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ApiError::decode("Chronik hash"))?;
    hash.reverse();
    Ok(hex::encode(hash))
}

fn units(sats: i64) -> Result<u64, ApiError> {
    u64::try_from(sats).map_err(|_| ApiError::decode("negative Chronik amount"))
}

/// A refusal's message, where the body is Chronik's error message.
fn refusal(error: ApiError) -> ApiError {
    match error {
        ApiError::Status { status, body } => {
            let message = ErrorMessage::decode(body.as_bytes())
                .ok()
                .map(|error| error.msg)
                .filter(|msg| !msg.is_empty())
                .unwrap_or(body);
            if status == 400 {
                ApiError::Rejected(message)
            } else {
                ApiError::Status {
                    status,
                    body: message,
                }
            }
        }
        other => other,
    }
}

pub struct ChronikClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
    chain: Chain,
}

impl ChronikClient {
    pub fn new(endpoints: Arc<Vec<String>>, chain: Chain) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
            chain,
        }
    }

    async fn get<T: Message + Default>(&self, path: &str) -> Result<T, ApiError> {
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{path}", base.trim_end_matches('/'));
            async move {
                let bytes = client
                    .get_bytes(&url, MAX_ANSWER_BYTES)
                    .await
                    .map_err(refusal)?;
                T::decode(bytes.as_slice())
                    .map_err(|e| ApiError::Decode(format!("Chronik answer: {e}")))
            }
        })
        .await
    }

    /// The path of the script `address` pays: Chronik's index key.
    fn script_path(&self, address: &str) -> Result<String, ApiError> {
        use crate::derivation::utxo_address::{ParsedUtxoAddress, parse_utxo_address};
        Ok(
            match parse_utxo_address(self.chain, address).map_err(ApiError::invalid)? {
                ParsedUtxoAddress::P2pkh(hash) => format!("/script/p2pkh/{}", hex::encode(hash)),
                ParsedUtxoAddress::P2sh(hash) => format!("/script/p2sh/{}", hex::encode(hash)),
                ParsedUtxoAddress::Witness { .. } => {
                    return Err(ApiError::invalid("eCash has no SegWit addresses"));
                }
            },
        )
    }

    fn script(&self, address: &str) -> Result<Vec<u8>, ApiError> {
        Ok(
            crate::derivation::utxo_address::parse_utxo_address(self.chain, address)
                .map_err(ApiError::invalid)?
                .script_pubkey(),
        )
    }

    pub(crate) async fn fetch_tip_height(&self) -> Result<u64, ApiError> {
        let info: BlockchainInfo = self.get("/blockchain-info").await?;
        u64::try_from(info.tip_height).map_err(|_| ApiError::decode("Chronik tip height"))
    }

    /// The hash of the block at `height`: `GET /block/{height}`.
    pub(crate) async fn fetch_block_hash(&self, height: u64) -> Result<String, ApiError> {
        let block: Block = self.get(&format!("/block/{height}")).await?;
        let info = block.block_info.or_decode("Chronik block has no info")?;
        if u64::try_from(info.height).ok() != Some(height) {
            return Err(ApiError::decode("Chronik answered another block"));
        }
        display_hash(&info.hash)
    }

    /// Refuse a server that is not on `chain`'s network: it must name the
    /// network's identity block.
    pub(crate) async fn verify_network(&self) -> Result<(), ApiError> {
        let (height, expected) = self.chain.identity_block()?;
        if self.fetch_block_hash(height).await? != expected {
            return Err(ApiError::invalid("Chronik server is on another network"));
        }
        Ok(())
    }

    async fn script_utxos(&self, address: &str) -> Result<ScriptUtxos, ApiError> {
        let path = self.script_path(address)?;
        let answer: ScriptUtxos = self.get(&format!("{path}/utxos")).await?;
        if answer.script != self.script(address)? {
            return Err(ApiError::decode("Chronik answered for another script"));
        }
        Ok(answer)
    }

    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, ApiError> {
        let path = self.script_path(address)?;
        let page: TxHistoryPage = self
            .get(&format!("{path}/history?page=0&page_size=1"))
            .await?;
        Ok(page.num_txs > 0)
    }

    /// Chronik lists the outputs the mempool leaves unspent, the mempool's
    /// own among them: the confirmed balance is theirs in blocks, and a
    /// pending spend of one has already removed it. Outputs carrying an
    /// eToken count, for the XEC they hold.
    pub(crate) async fn fetch_balance(&self, address: &str) -> Result<UtxoBalance, ApiError> {
        let answer = self.script_utxos(address).await?;
        let (mut confirmed, mut pending) = (0u64, 0u64);
        for utxo in &answer.utxos {
            let value = units(utxo.sats)?;
            let total = if utxo.block_height >= 0 {
                &mut confirmed
            } else {
                &mut pending
            };
            *total = total
                .checked_add(value)
                .or_decode("Chronik balance overflow")?;
        }
        Ok(UtxoBalance {
            confirmed_sats: confirmed,
            unconfirmed_sats: i64::try_from(pending).map_err(ApiError::decode)?,
        })
    }

    /// The outputs a plain transfer can spend, the mempool's included: not
    /// one carrying an eToken, which spending would burn, nor a coinbase
    /// output short of its maturity.
    pub(crate) async fn fetch_utxos(&self, address: &str) -> Result<Vec<Utxo>, ApiError> {
        let answer = self.script_utxos(address).await?;
        let tip = if answer.utxos.iter().any(|utxo| utxo.is_coinbase) {
            Some(self.fetch_tip_height().await?)
        } else {
            None
        };
        let maturity = u64::from(self.chain.generated_output_maturity()?);
        let mut utxos = Vec::new();
        for utxo in answer.utxos {
            if utxo.token.is_some() {
                continue;
            }
            let height = u64::try_from(utxo.block_height).ok();
            if utxo.is_coinbase
                && !height
                    .zip(tip)
                    .is_some_and(|(height, tip)| tip + 1 >= height + maturity)
            {
                continue;
            }
            let outpoint = utxo.outpoint.or_decode("Chronik output has no outpoint")?;
            utxos.push(Utxo {
                txid: display_hash(&outpoint.txid)?,
                vout: outpoint.out_idx,
                value: units(utxo.sats)?,
                status: UtxoStatus {
                    confirmed: height.is_some(),
                    block_height: height,
                },
            });
        }
        Ok(utxos)
    }

    /// A page of 50 transactions touching `address`, newest first, with the
    /// address's net change in each and the transaction's whole fee.
    pub(crate) async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<UtxoHistoryEntry>, ApiError> {
        let number = crate::api::history_page::page_number(cursor)?;
        let path = self.script_path(address)?;
        let script = self.script(address)?;
        let page: TxHistoryPage = self
            .get(&format!(
                "{path}/history?page={}&page_size={PAGE_SIZE}",
                number - 1
            ))
            .await?;
        let mut items = Vec::with_capacity(page.txs.len());
        for tx in page.txs {
            let txid = display_hash(&tx.txid)?;
            let mut net = 0i64;
            let mut inputs = 0i64;
            let mut outputs = 0i64;
            for input in &tx.inputs {
                inputs = inputs
                    .checked_add(input.sats)
                    .or_decode("Chronik input total")?;
                if input.output_script == script {
                    net = net
                        .checked_sub(input.sats)
                        .or_decode("Chronik net change")?;
                }
            }
            for output in &tx.outputs {
                outputs = outputs
                    .checked_add(output.sats)
                    .or_decode("Chronik output total")?;
                if output.output_script == script {
                    net = net
                        .checked_add(output.sats)
                        .or_decode("Chronik net change")?;
                }
            }
            if net == 0 {
                continue;
            }
            let block_height = tx
                .block
                .as_ref()
                .and_then(|block| u64::try_from(block.height).ok());
            let block_time = crate::api::time::history_time(
                block_height.is_some(),
                tx.block
                    .as_ref()
                    .and_then(|block| u64::try_from(block.timestamp).ok()),
                &txid,
            )?;
            items.push(UtxoHistoryEntry {
                confirmed: block_height.is_some(),
                block_height,
                block_time,
                net_sats: net,
                fee_sats: (!tx.is_coinbase)
                    .then(|| u64::try_from(inputs - outputs).ok())
                    .flatten(),
                txid,
            });
        }
        Ok(crate::api::HistoryPage {
            items,
            next_cursor: (number < page.num_pages).then(|| (number + 1).to_string()),
        })
    }

    pub(crate) async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, ApiError> {
        let tx: Tx = self.get(&format!("/tx/{txid}")).await?;
        let id = display_hash(&tx.txid)?;
        if !id.eq_ignore_ascii_case(txid) {
            return Err(ApiError::decode("Chronik answered another transaction"));
        }
        let block_height = tx
            .block
            .as_ref()
            .and_then(|block| u64::try_from(block.height).ok());
        Ok(UtxoTxStatus {
            txid: id,
            confirmed: block_height.is_some(),
            block_height,
            block_time: tx
                .block
                .as_ref()
                .and_then(|block| u64::try_from(block.timestamp).ok()),
            confirmations: None,
        })
    }

    /// Submit a signed transaction; Chronik answers with its txid. Its token
    /// checks stay on, so a transaction that would burn an eToken is refused.
    pub(crate) async fn broadcast_raw_tx(&self, raw_tx_hex: &str) -> Result<String, ApiError> {
        let body = BroadcastTxRequest {
            raw_tx: hex::decode(raw_tx_hex).map_err(ApiError::invalid)?,
            skip_token_checks: false,
        }
        .encode_to_vec();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let url = format!("{}/broadcast-tx", base.trim_end_matches('/'));
            async move {
                let (_, answer) = client
                    .post_bytes(
                        &url,
                        "application/x-protobuf",
                        body,
                        RetryProfile::ChainWrite,
                    )
                    .await
                    .map_err(refusal)?;
                let answer = BroadcastTxResponse::decode(answer.as_slice())
                    .map_err(|e| ApiError::Decode(format!("Chronik broadcast answer: {e}")))?;
                display_hash(&answer.txid)
            }
        })
        .await
    }
}

/// Chronik's answer listing `utxos`, each `(txid as explorers write it,
/// vout, sats, block height or -1)`, of `script`: what a test standing in
/// for a server serves.
#[cfg(test)]
pub(crate) fn script_utxos_answer(script: Vec<u8>, utxos: &[(String, u32, i64, i32)]) -> Vec<u8> {
    ScriptUtxos {
        script,
        utxos: utxos
            .iter()
            .map(|(txid, vout, sats, height)| {
                let mut txid = hex::decode(txid).expect("a hex txid");
                txid.reverse();
                ScriptUtxo {
                    outpoint: Some(OutPoint {
                        txid,
                        out_idx: *vout,
                    }),
                    block_height: *height,
                    is_coinbase: false,
                    sats: *sats,
                    token: None,
                }
            })
            .collect(),
    }
    .encode_to_vec()
}

/// Chronik's answer naming the block at `height` with `hash` as explorers
/// write it.
#[cfg(test)]
pub(crate) fn block_answer(height: i32, hash: &str) -> Vec<u8> {
    let mut hash = hex::decode(hash).expect("a hex hash");
    hash.reverse();
    Block {
        block_info: Some(BlockInfo { hash, height }),
    }
    .encode_to_vec()
}

/// The raw transaction a broadcast request carries, and Chronik's answer
/// accepting it under its txid.
#[cfg(test)]
pub(crate) fn accept_broadcast(request: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let raw = BroadcastTxRequest::decode(request)
        .expect("a broadcast request")
        .raw_tx;
    let txid = crate::send::bitcoin_wire::dsha256(&raw).to_vec();
    (raw, BroadcastTxResponse { txid }.encode_to_vec())
}

#[cfg(test)]
#[path = "tests/chronik.rs"]
mod tests;
