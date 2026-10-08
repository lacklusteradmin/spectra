//! The fixture's node, as an MWEB light client reads it over Litecoin's
//! peer-to-peer protocol: the handshake, `getheaders`, `getdata` for an MWEB
//! header or leafset, and `getmwebutxos`. Like Litecoin Core, it serves
//! leafsets and outputs only for blocks within ten of its tip, and drops a
//! peer that asks for outputs from a leaf that is not unspent.

use std::sync::{Arc, Mutex};

use litecoin::BlockHash;
use litecoin::hashes::Hash as _;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::chain::Chain;

const PROTOCOL: u32 = 70017;
/// NODE_NETWORK, NODE_WITNESS, NODE_MWEB_LIGHT_CLIENT and NODE_MWEB.
const SERVICES: u64 = 1 | 8 | (1 << 23) | (1 << 24);
const MWEB_HEADER: u32 = 8 | (1 << 29);
const MWEB_LEAFSET: u32 = 9 | (1 << 29);
const SERVED_DEPTH: u32 = 10;

fn checksum(payload: &[u8]) -> [u8; 4] {
    let twice = Sha256::digest(Sha256::digest(payload));
    twice[..4].try_into().unwrap()
}

async fn send(
    stream: &mut TcpStream,
    magic: [u8; 4],
    command: &str,
    payload: &[u8],
) -> std::io::Result<()> {
    let mut message = magic.to_vec();
    let mut name = [0u8; 12];
    name[..command.len()].copy_from_slice(command.as_bytes());
    message.extend(name);
    message.extend((payload.len() as u32).to_le_bytes());
    message.extend(checksum(payload));
    message.extend(payload);
    stream.write_all(&message).await
}

async fn receive(stream: &mut TcpStream, magic: [u8; 4]) -> std::io::Result<(String, Vec<u8>)> {
    let mut header = [0u8; 24];
    stream.read_exact(&mut header).await?;
    if header[..4] != magic {
        return Err(std::io::Error::other("another network"));
    }
    let command = String::from_utf8_lossy(&header[4..16])
        .trim_end_matches('\0')
        .to_string();
    let length = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).await?;
    if checksum(&payload) != header[20..24] {
        return Err(std::io::Error::other("a bad checksum"));
    }
    Ok((command, payload))
}

/// A cursor over a message's payload.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }

    fn compact(&mut self) -> Option<u64> {
        let first = self.take(1)?[0];
        Some(match first {
            0xfd => u64::from(u16::from_le_bytes(self.take(2)?.try_into().ok()?)),
            0xfe => u64::from(u32::from_le_bytes(self.take(4)?.try_into().ok()?)),
            0xff => u64::from_le_bytes(self.take(8)?.try_into().ok()?),
            small => u64::from(small),
        })
    }

    fn hash(&mut self) -> Option<BlockHash> {
        Some(BlockHash::from_byte_array(self.take(32)?.try_into().ok()?))
    }
}

fn version(height: u32) -> Vec<u8> {
    let mut out = PROTOCOL.to_le_bytes().to_vec();
    out.extend(SERVICES.to_le_bytes());
    out.extend(1_791_000_000i64.to_le_bytes());
    out.extend([0u8; 52]);
    out.extend(rand::random::<u64>().to_le_bytes());
    let agent = b"/spectra-litecoin-mweb-fixture/";
    crate::mweb::compact_size(agent.len() as u64, &mut out);
    out.extend(agent);
    out.extend(height.to_le_bytes());
    out.push(0);
    out
}

pub async fn serve(listener: TcpListener, chain: Arc<Mutex<Chain>>, magic: [u8; 4]) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let chain = chain.clone();
        tokio::spawn(async move {
            let _ = session(stream, chain, magic).await;
        });
    }
}

/// One peer, until it leaves or asks for what a node drops it for.
async fn session(
    mut stream: TcpStream,
    chain: Arc<Mutex<Chain>>,
    magic: [u8; 4],
) -> std::io::Result<()> {
    loop {
        let (command, payload) = receive(&mut stream, magic).await?;
        let mut fields = Fields(&payload);
        let answers: Vec<(&str, Vec<u8>)> = {
            let chain = chain.lock().unwrap();
            let tip = chain.tip().height;
            let served = |height: u32| tip - height <= SERVED_DEPTH;
            match command.as_str() {
                "version" => vec![("version", version(tip)), ("verack", Vec::new())],
                "ping" => vec![("pong", payload.clone())],
                "getheaders" => {
                    fields.take(4);
                    let locator = (0..fields.compact().unwrap_or(0))
                        .filter_map(|_| fields.hash())
                        .collect::<Vec<_>>();
                    let headers = locator
                        .first()
                        .map(|after| chain.headers_after(after))
                        .unwrap_or_default();
                    let mut out = Vec::new();
                    crate::mweb::compact_size(headers.len() as u64, &mut out);
                    for header in headers {
                        out.extend(litecoin::consensus::encode::serialize(&header));
                        out.push(0);
                    }
                    vec![("headers", out)]
                }
                "getdata" => {
                    let mut answers = Vec::new();
                    for _ in 0..fields.compact().unwrap_or(0) {
                        let kind = u32::from_le_bytes(
                            fields
                                .take(4)
                                .unwrap_or_default()
                                .try_into()
                                .unwrap_or_default(),
                        );
                        let Some(hash) = fields.hash() else { break };
                        match (kind, chain.block(&hash)) {
                            (MWEB_HEADER, Some(block)) => {
                                answers.push(("mwebheader", chain.mweb_header_answer(block)));
                            }
                            (MWEB_LEAFSET, Some(block)) if served(block.height) => {
                                answers.push(("mwebleafset", chain.leafset_answer(block)));
                            }
                            _ => {}
                        }
                    }
                    answers
                }
                "getmwebutxos" => {
                    let request = (|| {
                        let hash = fields.hash()?;
                        let start = fields.compact()?;
                        let count = u16::from_le_bytes(fields.take(2)?.try_into().ok()?);
                        let format = fields.take(1)?[0];
                        Some((hash, start, count, format))
                    })();
                    match request {
                        Some((hash, start, count, 2)) if count <= 4096 => {
                            match chain.block(&hash).filter(|block| served(block.height)) {
                                Some(block) => match chain.utxos_answer(block, start, count) {
                                    Some(answer) => vec![("mwebutxos", answer)],
                                    None => return Ok(()),
                                },
                                None => Vec::new(),
                            }
                        }
                        _ => return Ok(()),
                    }
                }
                _ => Vec::new(),
            }
        };
        for (command, payload) in answers {
            send(&mut stream, magic, command, &payload).await?;
        }
    }
}
