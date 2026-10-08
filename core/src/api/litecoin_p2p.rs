//! A Litecoin node, as an MWEB light client reads it over the peer-to-peer
//! protocol (version 70017, `NODE_MWEB_LIGHT_CLIENT`), on `api::tcp`.
//!
//! Every answer is checked before it is used: headers chain from a block the
//! wallet's indexer names, each meeting its proof of work and Litecoin's
//! difficulty rule; a block's MWEB header is the one its HogEx commits to,
//! that transaction proved by the block's merkle root; the leafset hashes to
//! the header's leafset root; and every page of unspent outputs proves into
//! the header's output root. A node serves leafsets and outputs only for
//! blocks within ten of its tip.

pub(crate) mod wire;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use bitcoin::hashes::{Hash, sha256d};
use num_bigint::BigUint;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use self::wire::{HogEx, MwebHeader, Output, Reader, blake3, hogaddr_script};
use crate::api::error::ApiError;
use crate::registry::Chain;

/// What MWEB light clients speak.
const PROTOCOL_VERSION: u32 = 70017;
const NODE_MWEB_LIGHT_CLIENT: u64 = 1 << 23;
const NODE_MWEB: u64 = 1 << 24;
const MSG_MWEB_HEADER: u32 = 8 | (1 << 29);
const MSG_MWEB_LEAFSET: u32 = 9 | (1 << 29);
/// The most outputs one `getmwebutxos` may ask for.
pub(crate) const MAX_UTXOS_PER_REQUEST: u16 = 4096;
/// How long an answer may take: a node rate-limits MWEB requests by
/// dropping them, so silence past this is a refusal.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(45);
/// A node serves 32 leafset or output requests at once, then one every two
/// seconds, across every client it serves, and ignores the rest. Requests
/// from here keep to that allowance per node; other clients can still use
/// it up, which an unanswered request then shows.
const SERVE_BURST: f64 = 32.0;
const SERVE_REFILL_PER_SECOND: f64 = 0.5;

/// What is left of each node's allowance, by endpoint, as of when.
static ALLOWANCES: LazyLock<parking_lot::Mutex<HashMap<String, (f64, Instant)>>> =
    LazyLock::new(Default::default);

/// Wait until `node` would serve one more leafset or output request.
async fn pace(node: &str) {
    let wait = {
        let mut allowances = ALLOWANCES.lock();
        let now = Instant::now();
        let (tokens, at) = allowances
            .entry(node.to_string())
            .or_insert((SERVE_BURST, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * SERVE_REFILL_PER_SECOND)
            .min(SERVE_BURST)
            - 1.0;
        *at = now;
        Duration::from_secs_f64((-*tokens).max(0.0) / SERVE_REFILL_PER_SECOND)
    };
    tokio::time::sleep(wait).await;
}
/// The most peers one endpoint's name is tried at.
const MAX_PEERS_PER_ENDPOINT: usize = 8;
/// The largest message a node sends; a block is the largest.
const MAX_MESSAGE: usize = 32 << 20;

/// A network's peer-to-peer and proof-of-work parameters.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Params {
    magic: [u8; 4],
    pub port: u16,
    /// Testnet allows a block at the proof-of-work limit after five minutes
    /// with none.
    allow_min_difficulty: bool,
}

const POW_LIMIT_BITS: u32 = 0x1e0f_ffff;
/// Blocks between Litecoin's difficulty retargets.
pub(crate) const RETARGET_INTERVAL: u64 = 2016;
const TARGET_TIMESPAN: u64 = 302_400;
const TARGET_SPACING: u64 = 150;

pub(crate) fn params(chain: Chain) -> Result<Params, ApiError> {
    match chain {
        Chain::Litecoin => Ok(Params {
            magic: [0xfb, 0xc0, 0xb6, 0xdb],
            port: 9333,
            allow_min_difficulty: false,
        }),
        Chain::LitecoinTestnet => Ok(Params {
            magic: [0xfd, 0xd2, 0xc8, 0xf1],
            port: 19335,
            allow_min_difficulty: true,
        }),
        _ => Err(ApiError::invalid("Not a Litecoin network")),
    }
}

/// A block header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockHeader {
    pub raw: [u8; 80],
    pub hash: [u8; 32],
}

impl BlockHeader {
    pub(crate) fn parse(raw: [u8; 80]) -> Self {
        Self {
            raw,
            hash: sha256d::Hash::hash(&raw).to_byte_array(),
        }
    }

    pub(crate) fn previous(&self) -> [u8; 32] {
        self.raw[4..36].try_into().expect("32 bytes")
    }

    pub(crate) fn merkle_root(&self) -> [u8; 32] {
        self.raw[36..68].try_into().expect("32 bytes")
    }

    pub(crate) fn time(&self) -> u32 {
        u32::from_le_bytes(self.raw[68..72].try_into().expect("4 bytes"))
    }

    pub(crate) fn bits(&self) -> u32 {
        u32::from_le_bytes(self.raw[72..76].try_into().expect("4 bytes"))
    }

    /// Whether the header's scrypt hash is at or below the target its bits
    /// name.
    pub(crate) fn meets_proof_of_work(&self) -> bool {
        let Some(target) = compact_to_target(self.bits()) else {
            return false;
        };
        let mut hash = [0u8; 32];
        let params = scrypt::Params::new(10, 1, 1, 32).expect("Litecoin's scrypt parameters");
        if scrypt::scrypt(&self.raw, &self.raw, &params, &mut hash).is_err() {
            return false;
        }
        hash.reverse();
        BigUint::from_bytes_be(&hash) <= target
    }
}

/// A hash in the byte order explorers write it.
pub(crate) fn display(hash: &[u8; 32]) -> String {
    let mut bytes = *hash;
    bytes.reverse();
    hex::encode(bytes)
}

/// A hash as explorers write it, in the protocol's byte order.
pub(crate) fn parse_display(hash: &str) -> Result<[u8; 32], ApiError> {
    let mut bytes: [u8; 32] = hex::decode(hash.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| ApiError::decode("block hash"))?;
    bytes.reverse();
    Ok(bytes)
}

/// The target compact bits name, or `None` for a negative or overflowing one.
fn compact_to_target(bits: u32) -> Option<BigUint> {
    let exponent = bits >> 24;
    let mantissa = bits & 0x007f_ffff;
    if bits & 0x0080_0000 != 0 && mantissa != 0 {
        return None;
    }
    let target = if exponent <= 3 {
        BigUint::from(mantissa >> (8 * (3 - exponent)))
    } else {
        BigUint::from(mantissa) << (8 * (exponent - 3))
    };
    (target.bits() <= 256).then_some(target)
}

/// Compact bits for a target, as Litecoin Core's `GetCompact`.
fn target_to_compact(target: &BigUint) -> u32 {
    let mut size = target.to_bytes_be().len() as u32;
    if target.bits() == 0 {
        return 0;
    }
    let mut compact = if size <= 3 {
        let value = target.to_u64_digits().first().copied().unwrap_or(0) as u32;
        value << (8 * (3 - size))
    } else {
        let shifted = target >> (8 * (size - 3));
        shifted.to_u64_digits().first().copied().unwrap_or(0) as u32
    };
    if compact & 0x0080_0000 != 0 {
        compact >>= 8;
        size += 1;
    }
    compact | (size << 24)
}

/// The bits a retarget gives after a window that took `timespan` seconds,
/// from the window's last bits, as Litecoin Core computes them — its shift
/// that keeps the product within 256 bits included.
fn retarget(last_bits: u32, timespan: u64) -> Option<u32> {
    let timespan = timespan.clamp(TARGET_TIMESPAN / 4, TARGET_TIMESPAN * 4);
    let limit = compact_to_target(POW_LIMIT_BITS)?;
    let mut target = compact_to_target(last_bits)?;
    let shift = target.bits() > limit.bits() - 1;
    if shift {
        target >>= 1;
    }
    target = target * timespan / TARGET_TIMESPAN;
    if shift {
        target <<= 1;
    }
    if target > limit {
        target = limit;
    }
    Some(target_to_compact(&target))
}

/// Check `headers` chain on from `anchor` (at height `anchor_height`), each
/// meeting its proof of work and the bits Litecoin's rule gives it.
/// `window_start_time` answers the time of the block at a height, for the
/// window a retarget measures.
pub(crate) fn verify_headers(
    params: &Params,
    anchor: &BlockHeader,
    anchor_height: u64,
    headers: &[BlockHeader],
    window_start_time: &dyn Fn(u64) -> Option<u32>,
) -> Result<(), ApiError> {
    let refuse = |height: u64, what: &str| {
        Err(ApiError::Decode(format!(
            "Litecoin header at {height}: {what}"
        )))
    };
    let mut previous = anchor;
    // The bits of the last block that was not at the minimum difficulty,
    // which a testnet block returns to.
    let mut last_real_bits = anchor.bits();
    for (offset, header) in headers.iter().enumerate() {
        let height = anchor_height + 1 + offset as u64;
        if header.previous() != previous.hash {
            return refuse(height, "does not follow the block before it");
        }
        if !header.meets_proof_of_work() {
            return refuse(height, "fails its proof of work");
        }
        let bits = header.bits();
        if height.is_multiple_of(RETARGET_INTERVAL) {
            let first = height - 1 - RETARGET_INTERVAL;
            let Some(first_time) = window_start_time(first) else {
                return refuse(height, "its difficulty window is unknown");
            };
            let timespan = u64::from(previous.time()).saturating_sub(u64::from(first_time));
            if retarget(previous.bits(), timespan) != Some(bits) {
                return refuse(height, "does not carry the retargeted difficulty");
            }
        } else if bits != previous.bits() {
            let late = u64::from(header.time()) > u64::from(previous.time()) + 2 * TARGET_SPACING;
            let allowed = params.allow_min_difficulty
                && ((late && bits == POW_LIMIT_BITS) || bits == last_real_bits);
            if !allowed {
                return refuse(height, "changes difficulty between retargets");
            }
        }
        if bits != POW_LIMIT_BITS {
            last_real_bits = bits;
        }
        previous = header;
    }
    Ok(())
}

/// Which of a block's outputs are unspent: bit `i` (most significant first
/// within each byte) for leaf `i`.
#[derive(Debug, Clone)]
pub(crate) struct Leafset {
    bits: Vec<u8>,
    pub size: u64,
}

impl Leafset {
    pub(crate) fn contains(&self, leaf: u64) -> bool {
        leaf < self.size
            && self
                .bits
                .get((leaf / 8) as usize)
                .is_some_and(|byte| byte & (0x80 >> (leaf % 8)) != 0)
    }

    /// The first unspent leaf at or after `leaf`.
    pub(crate) fn unspent_from(&self, mut leaf: u64) -> Option<u64> {
        while leaf < self.size {
            if self.contains(leaf) {
                return Some(leaf);
            }
            leaf += 1;
        }
        None
    }
}

/// A block's MWEB header, with the block header its HogEx is proved in.
#[derive(Debug, Clone)]
pub(crate) struct ProvedMwebHeader {
    pub block: BlockHeader,
    pub mweb: MwebHeader,
}

/// An unspent output and its leaf.
#[derive(Debug, Clone)]
pub(crate) struct LeafOutput {
    pub leaf: u64,
    pub output: Output,
}

/// A chain's Litecoin nodes.
pub struct LitecoinP2pClient {
    endpoints: Arc<Vec<String>>,
}

/// One node's connection, after the handshake.
pub(crate) struct Session {
    stream: tokio::net::TcpStream,
    /// The peer, as `address:port`.
    pub node: String,
    magic: [u8; 4],
    /// The height the node said it had when it connected.
    pub peer_height: u64,
}

fn transport(error: impl std::fmt::Display) -> ApiError {
    ApiError::Transport(format!("Litecoin node: {error}"))
}

fn decode(error: impl std::fmt::Display) -> ApiError {
    ApiError::Decode(format!("Litecoin node: {error}"))
}

/// The host and port a `tcp://host:port` endpoint names.
fn host_port(endpoint: &str, params: &Params) -> Result<(String, u16), ApiError> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| ApiError::invalid(format!("Invalid Litecoin node {endpoint}")))?;
    if url.scheme() != "tcp" {
        return Err(ApiError::invalid(format!(
            "Invalid Litecoin node {endpoint}"
        )));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::invalid(format!("Invalid Litecoin node {endpoint}")))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    Ok((host, url.port().unwrap_or(params.port)))
}

impl LitecoinP2pClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self { endpoints }
    }

    /// A session with the first node that connects, completes the
    /// handshake on `chain`'s network and serves MWEB light clients: of each
    /// endpoint, each of the peers it names in turn, but those in `skip`.
    pub(crate) async fn session(
        &self,
        chain: Chain,
        skip: &std::collections::HashSet<String>,
    ) -> Result<Session, ApiError> {
        let params = params(chain)?;
        let mut last = ApiError::NoEndpoint;
        for endpoint in self.endpoints.iter() {
            let peers = async {
                let (host, port) = host_port(endpoint, &params)?;
                let peers = crate::api::tcp::peers(endpoint, &host, port).await?;
                Ok::<_, ApiError>((peers, port))
            };
            let (peers, port) = match peers.await {
                Ok(peers) => peers,
                Err(error) => {
                    last = error;
                    continue;
                }
            };
            for peer in peers
                .iter()
                .filter(|peer| !skip.contains(&format!("{peer}:{port}")))
                .take(MAX_PEERS_PER_ENDPOINT)
            {
                let attempt = async {
                    let stream = crate::api::tcp::connect(endpoint, peer, port).await?;
                    let mut session = Session {
                        stream,
                        node: format!("{peer}:{port}"),
                        magic: params.magic,
                        peer_height: 0,
                    };
                    tokio::time::timeout(ANSWER_TIMEOUT, session.handshake())
                        .await
                        .map_err(|_| transport("no handshake"))??;
                    Ok::<_, ApiError>(session)
                };
                match attempt.await {
                    Ok(session) => return Ok(session),
                    Err(error) => last = error,
                }
            }
        }
        Err(last)
    }
}

fn compact_size(n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    wire::write_compact_size(n, &mut out);
    out
}

impl Session {
    async fn send(&mut self, command: &str, payload: &[u8]) -> Result<(), ApiError> {
        let mut message = Vec::with_capacity(24 + payload.len());
        message.extend(self.magic);
        let mut name = [0u8; 12];
        name[..command.len()].copy_from_slice(command.as_bytes());
        message.extend(name);
        message.extend((payload.len() as u32).to_le_bytes());
        message.extend(&sha256d::Hash::hash(payload).to_byte_array()[..4]);
        message.extend(payload);
        self.stream.write_all(&message).await.map_err(transport)
    }

    async fn receive(&mut self) -> Result<(String, Vec<u8>), ApiError> {
        let mut header = [0u8; 24];
        self.stream
            .read_exact(&mut header)
            .await
            .map_err(transport)?;
        if header[..4] != self.magic {
            return Err(decode("a message for another network"));
        }
        let command = String::from_utf8_lossy(&header[4..16])
            .trim_end_matches('\0')
            .to_string();
        let length = u32::from_le_bytes(header[16..20].try_into().expect("4 bytes")) as usize;
        if length > MAX_MESSAGE {
            return Err(decode("an oversized message"));
        }
        let mut payload = vec![0u8; length];
        self.stream
            .read_exact(&mut payload)
            .await
            .map_err(transport)?;
        if sha256d::Hash::hash(&payload).to_byte_array()[..4] != header[20..24] {
            return Err(decode("a message with a bad checksum"));
        }
        Ok((command, payload))
    }

    /// The next message among `wanted`, answering pings and passing over
    /// what else the node sends; `notfound` answers too.
    async fn answer(&mut self, wanted: &[&str]) -> Result<(String, Vec<u8>), ApiError> {
        let read = async {
            loop {
                let (command, payload) = self.receive().await?;
                if command == "ping" {
                    self.send("pong", &payload).await?;
                    continue;
                }
                if command == "notfound" || wanted.contains(&command.as_str()) {
                    return Ok((command, payload));
                }
            }
        };
        tokio::time::timeout(ANSWER_TIMEOUT, read)
            .await
            .map_err(|_| transport("no answer"))?
    }

    async fn handshake(&mut self) -> Result<(), ApiError> {
        let mut version = Vec::new();
        version.extend(PROTOCOL_VERSION.to_le_bytes());
        version.extend(0u64.to_le_bytes());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        version.extend(now.to_le_bytes());
        // The two network addresses, which a light client leaves empty.
        version.extend([0u8; 52]);
        let mut nonce = [0u8; 8];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);
        version.extend(nonce);
        version.extend(compact_size(0));
        version.extend(0i32.to_le_bytes());
        // No transactions relayed to us.
        version.push(0);
        self.send("version", &version).await?;
        let (_, theirs) = self.answer(&["version"]).await?;
        let mut reader = Reader(&theirs);
        let their_version = reader.u32_le().map_err(|_| decode("version"))?;
        let services = reader.u64_le().map_err(|_| decode("version"))?;
        reader
            .take(8 + 26 + 26 + 8)
            .map_err(|_| decode("version"))?;
        reader.bytes().map_err(|_| decode("version"))?;
        let height = reader.u32_le().map_err(|_| decode("version"))?;
        if their_version < PROTOCOL_VERSION
            || services & (NODE_MWEB_LIGHT_CLIENT | NODE_MWEB) != NODE_MWEB_LIGHT_CLIENT | NODE_MWEB
        {
            return Err(decode("the node does not serve MWEB light clients"));
        }
        self.peer_height = u64::from(height);
        self.send("verack", &[]).await?;
        self.answer(&["verack"]).await?;
        Ok(())
    }

    /// The headers that follow `after`, through the node's tip.
    pub(crate) async fn headers_after(
        &mut self,
        after: [u8; 32],
    ) -> Result<Vec<BlockHeader>, ApiError> {
        let mut headers: Vec<BlockHeader> = Vec::new();
        loop {
            let locator = headers.last().map_or(after, |header| header.hash);
            let mut request = PROTOCOL_VERSION.to_le_bytes().to_vec();
            request.extend(compact_size(1));
            request.extend(locator);
            request.extend([0u8; 32]);
            self.send("getheaders", &request).await?;
            let (command, payload) = self.answer(&["headers"]).await?;
            if command != "headers" {
                return Err(decode("no headers"));
            }
            let mut reader = Reader(&payload);
            let count = reader.length(81).map_err(|_| decode("headers"))?;
            for _ in 0..count {
                let header = BlockHeader::parse(reader.array().map_err(|_| decode("headers"))?);
                if reader.compact_size().map_err(|_| decode("headers"))? != 0 {
                    return Err(decode("a header with transactions"));
                }
                headers.push(header);
            }
            reader.finished().map_err(|_| decode("headers"))?;
            // A node answers at most 2,000 at a time.
            if count < 2000 {
                return Ok(headers);
            }
        }
    }

    async fn get_data(
        &mut self,
        kind: u32,
        block: &[u8; 32],
        answer: &str,
    ) -> Result<Vec<u8>, ApiError> {
        let mut request = compact_size(1);
        request.extend(kind.to_le_bytes());
        request.extend(block);
        self.send("getdata", &request).await?;
        let (command, payload) = self.answer(&[answer]).await?;
        if command != answer {
            return Err(decode(format!("no {answer} for block {}", display(block))));
        }
        Ok(payload)
    }

    /// Block `block`'s MWEB header, proved: the merkle branch of the block's
    /// last transaction leads to its merkle root, that transaction is a
    /// HogEx, and its first output commits to the header's hash.
    pub(crate) async fn mweb_header(
        &mut self,
        block: &[u8; 32],
    ) -> Result<ProvedMwebHeader, ApiError> {
        let payload = self.get_data(MSG_MWEB_HEADER, block, "mwebheader").await?;
        let proved = parse_mweb_header(&payload)?;
        if proved.block.hash != *block {
            return Err(decode("an MWEB header for another block"));
        }
        Ok(proved)
    }

    /// The leafset of `header`'s block, proved by the header's leafset root.
    pub(crate) async fn leafset(&mut self, header: &ProvedMwebHeader) -> Result<Leafset, ApiError> {
        pace(&self.node).await;
        let payload = self
            .get_data(MSG_MWEB_LEAFSET, &header.block.hash, "mwebleafset")
            .await?;
        parse_leafset(&payload, header)
    }

    /// Up to `count` unspent outputs from leaf `start` (an unspent one) of
    /// `header`'s block, proved into its output root.
    pub(crate) async fn unspent_outputs(
        &mut self,
        header: &ProvedMwebHeader,
        leafset: &Leafset,
        start: u64,
        count: u16,
    ) -> Result<Vec<LeafOutput>, ApiError> {
        let mut request = header.block.hash.to_vec();
        request.extend(compact_size(start));
        request.extend(count.min(MAX_UTXOS_PER_REQUEST).to_le_bytes());
        // Compact outputs: each with its range proof's hash.
        request.push(2);
        pace(&self.node).await;
        self.send("getmwebutxos", &request).await?;
        let (command, payload) = self.answer(&["mwebutxos"]).await?;
        if command != "mwebutxos" {
            return Err(decode("no outputs"));
        }
        parse_unspent_outputs(&payload, header, leafset, start)
    }
}

/// An `mwebheader` answer, checked against itself.
pub(crate) fn parse_mweb_header(payload: &[u8]) -> Result<ProvedMwebHeader, ApiError> {
    use bitcoin::consensus::Decodable;
    let mut cursor = payload;
    let merkle =
        bitcoin::MerkleBlock::consensus_decode(&mut cursor).map_err(|_| decode("merkle block"))?;
    let mut reader = Reader(cursor);
    let hogex = HogEx::decode(&mut reader)?;
    let mweb = MwebHeader::decode(&mut reader)?;
    reader.finished()?;
    let block = BlockHeader::parse(
        bitcoin::consensus::serialize(&merkle.header)
            .try_into()
            .map_err(|_| decode("block header"))?,
    );
    let mut matches = Vec::new();
    let mut indexes = Vec::new();
    let root = merkle
        .txn
        .extract_matches(&mut matches, &mut indexes)
        .map_err(|_| decode("a merkle branch that does not resolve"))?;
    if root.to_byte_array() != block.merkle_root() {
        return Err(decode("a merkle branch to another root"));
    }
    let last = merkle.txn.num_transactions().saturating_sub(1);
    let proves_hogex = matches.last().map(|txid| txid.to_byte_array()) == Some(hogex.txid)
        && indexes.last() == Some(&last);
    if !proves_hogex {
        return Err(decode("a HogEx the merkle branch does not prove last"));
    }
    if hogex.outputs[0].script_pubkey.as_bytes() != hogaddr_script(&mweb.hash()).as_slice() {
        return Err(decode("a HogEx committing to another MWEB header"));
    }
    Ok(ProvedMwebHeader { block, mweb })
}

/// An `mwebleafset` answer for `header`'s block.
pub(crate) fn parse_leafset(
    payload: &[u8],
    header: &ProvedMwebHeader,
) -> Result<Leafset, ApiError> {
    let mut reader = Reader(payload);
    if reader.array::<32>()? != header.block.hash {
        return Err(decode("a leafset for another block"));
    }
    let bits = reader.bytes()?;
    reader.finished()?;
    if blake3(&bits) != header.mweb.leafset_root {
        return Err(decode("a leafset that is not the header's"));
    }
    let size = header.mweb.output_mmr_size;
    if bits.len() as u64 != size.div_ceil(8) {
        return Err(decode("a leafset of the wrong size"));
    }
    Ok(Leafset { bits, size })
}

/// An `mwebutxos` answer from leaf `start`, proved into `header`'s output
/// root.
pub(crate) fn parse_unspent_outputs(
    payload: &[u8],
    header: &ProvedMwebHeader,
    leafset: &Leafset,
    start: u64,
) -> Result<Vec<LeafOutput>, ApiError> {
    let mut reader = Reader(payload);
    if reader.array::<32>()? != header.block.hash {
        return Err(decode("outputs of another block"));
    }
    if reader.compact_size()? != start {
        return Err(decode("outputs from another leaf"));
    }
    if reader.u8()? != 2 {
        return Err(decode("outputs in another format"));
    }
    let count = reader.length(1)?;
    let mut outputs = Vec::with_capacity(count);
    for _ in 0..count {
        let leaf = reader.compact_size()?;
        outputs.push(LeafOutput {
            leaf,
            output: Output::decode_compact(&mut reader)?,
        });
    }
    let proof_count = reader.length(32)?;
    let mut proofs = Vec::with_capacity(proof_count);
    for _ in 0..proof_count {
        proofs.push(reader.array::<32>()?);
    }
    reader.finished()?;
    let leaves: Vec<(u64, [u8; 32])> = outputs.iter().map(|o| (o.leaf, o.output.id())).collect();
    if !mmr::verify(header.mweb.output_root, leafset, start, &leaves, &proofs) {
        return Err(decode("outputs that do not prove into the output root"));
    }
    Ok(outputs)
}

/// The output MMR: leaves are output ids, each node hashed with its
/// position. A page of consecutive unspent leaves proves into the root with
/// the hashes a node sends beside it, consumed in the order ltcd's verifier
/// (the one mwebd runs) consumes them.
pub(crate) mod mmr {
    use super::Leafset;
    use std::collections::HashSet;

    fn node_of(leaf: u64) -> u64 {
        2 * leaf - u64::from(leaf.count_ones())
    }

    fn bit_length(x: u64) -> u32 {
        64 - x.leading_zeros()
    }

    fn all_ones(bits: u32) -> u64 {
        if bits >= 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        }
    }

    fn height(node: u64) -> u64 {
        let mut height = node;
        let mut peak = all_ones(bit_length(node));
        while peak > 0 {
            if height >= peak {
                height -= peak;
            }
            peak >>= 1;
        }
        height
    }

    fn leaf_of(node: u64) -> u64 {
        let (mut leaf, mut left) = (0u64, node);
        let mut peak = all_ones(bit_length(node));
        while peak > 0 {
            if left >= peak {
                leaf += peak.div_ceil(2);
                left -= peak;
            }
            peak >>= 1;
        }
        leaf
    }

    pub(crate) fn leaf_hash(node: u64, id: &[u8; 32]) -> [u8; 32] {
        let mut preimage = node.to_le_bytes().to_vec();
        preimage.push(32);
        preimage.extend(id);
        super::blake3(&preimage)
    }

    pub(crate) fn parent_hash(node: u64, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut preimage = node.to_le_bytes().to_vec();
        preimage.extend(left);
        preimage.extend(right);
        super::blake3(&preimage)
    }

    pub(crate) fn peaks(mut nodes: u64) -> Vec<u64> {
        let mut out = Vec::new();
        let mut before = 0;
        let mut peak = all_ones(bit_length(nodes));
        while peak > 0 {
            if nodes >= peak {
                out.push(before + peak - 1);
                before += peak;
                nodes -= peak;
            }
            peak >>= 1;
        }
        out
    }

    pub(crate) fn left_child(node: u64, height: u64) -> u64 {
        node - (1u64 << height)
    }

    pub(crate) fn right_child(node: u64) -> u64 {
        node - 1
    }

    struct Verifier<'a> {
        leaves: &'a [(u64, [u8; 32])],
        proofs: &'a [[u8; 32]],
        leafset: &'a Leafset,
        first: u64,
        last: u64,
        leaves_used: usize,
        proofs_used: usize,
        is_proof: HashSet<u64>,
    }

    impl Verifier<'_> {
        fn next_leaf(&mut self) -> Option<(u64, [u8; 32])> {
            let leaf = *self.leaves.get(self.leaves_used)?;
            self.leaves_used += 1;
            Some(leaf)
        }

        fn next_proof(&mut self, node: u64) -> Option<[u8; 32]> {
            let hash = *self.proofs.get(self.proofs_used)?;
            self.proofs_used += 1;
            self.is_proof.insert(node);
            Some(hash)
        }

        fn node_hash(&mut self, node: u64, height: u64) -> Option<[u8; 32]> {
            if node < node_of(self.first) || self.is_proof.contains(&node) {
                return self.next_proof(node);
            }
            if height == 0 {
                let leaf = leaf_of(node);
                if !self.leafset.contains(leaf) {
                    return None;
                }
                let (index, id) = self.next_leaf()?;
                return (index == leaf).then(|| leaf_hash(node, &id));
            }
            let left_node = left_child(node, height);
            let right_node = right_child(node);
            let mut left = self.node_hash(left_node, height - 1);
            let mut right = if node_of(self.last) <= left_node {
                self.next_proof(right_node)
            } else {
                self.node_hash(right_node, height - 1)
            };
            match (left, right) {
                (None, None) => return None,
                (None, Some(_)) => left = Some(self.next_proof(left_node)?),
                (Some(_), None) => right = Some(self.next_proof(right_node)?),
                (Some(_), Some(_)) => {}
            }
            Some(parent_hash(node, &left?, &right?))
        }
    }

    /// Whether `leaves` — consecutive unspent leaves from `start`, with
    /// their output ids — and `proofs` resolve to `root`.
    pub(crate) fn verify(
        root: [u8; 32],
        leafset: &Leafset,
        start: u64,
        leaves: &[(u64, [u8; 32])],
        proofs: &[[u8; 32]],
    ) -> bool {
        if leaves.is_empty() || leafset.size == 0 {
            return false;
        }
        let mut verifier = Verifier {
            leaves,
            proofs,
            leafset,
            first: start,
            last: start,
            leaves_used: 0,
            proofs_used: 0,
            is_proof: HashSet::new(),
        };
        for (i, (leaf, _)) in leaves.iter().enumerate() {
            if !leafset.contains(verifier.last) || *leaf != verifier.last {
                return false;
            }
            if i + 1 == leaves.len() {
                break;
            }
            loop {
                verifier.last += 1;
                if leafset.contains(verifier.last) || verifier.last >= leafset.size {
                    break;
                }
            }
        }
        let next = node_of(leafset.size);
        let peaks = peaks(next);
        let mut hashes = Vec::new();
        for _pass in 0..2 {
            hashes.clear();
            verifier.leaves_used = 0;
            verifier.proofs_used = 0;
            for &peak in &peaks {
                let hash = match verifier.node_hash(peak, height(peak)) {
                    Some(hash) => hash,
                    None => match verifier.next_proof(peak) {
                        Some(hash) => hash,
                        None => return false,
                    },
                };
                hashes.push(hash);
                if node_of(verifier.last) <= peak {
                    if Some(&peak) != peaks.last() {
                        match verifier.next_proof(next) {
                            Some(hash) => hashes.push(hash),
                            None => return false,
                        }
                    }
                    break;
                }
            }
            if verifier.leaves_used != leaves.len() || verifier.proofs_used != proofs.len() {
                return false;
            }
        }
        let Some(mut bagged) = hashes.last().copied() else {
            return false;
        };
        for hash in hashes.iter().rev().skip(1) {
            bagged = parent_hash(next, hash, &bagged);
        }
        bagged == root
    }
}

impl Leafset {
    /// A leafset for tests and fixtures, from its bits.
    #[cfg(test)]
    pub(crate) fn from_bits(bits: Vec<u8>, size: u64) -> Self {
        Self { bits, size }
    }
}

#[cfg(test)]
#[path = "tests/litecoin_p2p.rs"]
mod tests;
