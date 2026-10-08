//! The fixture's indexer: the part of Esplora's HTTP API a Litecoin wallet
//! reads — the tip, block hashes by height, an address's balance, outputs
//! and transactions, fee estimates — and its broadcast, which checks and
//! mines what it is handed.

use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::chain::Chain;

pub async fn serve(listener: TcpListener, chain: Arc<Mutex<Chain>>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let chain = chain.clone();
        tokio::spawn(async move {
            let _ = handle(stream, chain).await;
        });
    }
}

/// Read one request: its method, path and body.
async fn request(stream: &mut TcpStream) -> std::io::Result<(String, String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let head_end = loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::other("closed"));
        }
        buffer.extend(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let mut lines = head.lines();
    let mut start = lines.next().unwrap_or_default().split_whitespace();
    let method = start.next().unwrap_or_default().to_string();
    let path = start.next().unwrap_or_default().to_string();
    let length: usize = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0u8; length - body.len()];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend(&chunk[..read]);
    }
    Ok((method, path, body))
}

async fn handle(mut stream: TcpStream, chain: Arc<Mutex<Chain>>) -> std::io::Result<()> {
    let (method, path, body) = request(&mut stream).await?;
    let path = path.split('?').next().unwrap_or_default().to_string();
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let (status, content_type, body) = {
        let mut chain = chain.lock().unwrap();
        let ok = |value: serde_json::Value| (200, "application/json", value.to_string());
        match (method.as_str(), parts.as_slice()) {
            ("GET", ["blocks", "tip", "height"]) => ok(json!(chain.tip().height)),
            // The synthetic chain stands on the real network's genesis.
            ("GET", ["block-height", "0"]) => (200, "text/plain", chain.genesis().into()),
            ("GET", ["block-height", height]) => {
                match height.parse().ok().and_then(|h| chain.block_at(h)) {
                    Some(block) => (200, "text/plain", block.hash.to_string()),
                    None => (404, "text/plain", "Block not found".into()),
                }
            }
            ("GET", ["address", address]) => ok(chain.address_json(address)),
            ("GET", ["address", address, "utxo"]) => ok(chain.utxos_json(address)),
            ("GET", ["address", address, "txs"]) => ok(chain.history_json(address)),
            ("GET", ["address", _, "txs", "chain", _]) => ok(json!([])),
            ("GET", ["fee-estimates"]) => ok(json!({"1": 1.0, "3": 1.0, "6": 1.0, "144": 1.0})),
            ("GET", ["tx", txid, "status"]) => match chain.status_json(txid) {
                Some(status) => ok(status),
                None => (404, "text/plain", "Transaction not found".into()),
            },
            ("POST", ["tx"]) => {
                let accepted = String::from_utf8(body)
                    .ok()
                    .and_then(|text| hex::decode(text.trim()).ok())
                    .ok_or_else(|| "TX decode failed".to_string())
                    .and_then(|raw| chain.accept(&raw));
                match accepted {
                    Ok(txid) => (200, "text/plain", txid),
                    Err(error) => (
                        400,
                        "text/plain",
                        format!(
                            "sendrawtransaction RPC error: {{\"code\":-26,\"message\":\"{error}\"}}"
                        ),
                    ),
                }
            }
            _ => (404, "text/plain", "Not found".into()),
        }
    };
    let reason = if status == 200 { "OK" } else { "Error" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}
