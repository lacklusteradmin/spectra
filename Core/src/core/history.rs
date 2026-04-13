use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// ----------------------------------------------------------------
// Normalized chain history — standard output from fetch_normalized_history_json
// ----------------------------------------------------------------

/// A chain history entry normalized to a standard format that Swift can map
/// directly to `TransactionRecord` without any chain-specific parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainHistoryEntry {
    pub kind: String,         // "receive" | "send"
    pub status: String,       // "confirmed" | "pending"
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub amount: f64,
    pub counterparty: String,
    pub tx_hash: String,
    pub block_height: Option<i64>,
    pub timestamp: f64,       // Unix seconds
}

/// Convert a raw history JSON string (as returned by `fetch_history`) into
/// normalized `ChainHistoryEntry` records that Swift can consume without
/// any chain-specific parsing logic.
pub fn normalize_chain_history(chain_id: u32, raw_json: &str) -> Vec<ChainHistoryEntry> {
    let Ok(value) = serde_json::from_str::<Value>(raw_json) else {
        return vec![];
    };
    let arr = match &value {
        Value::Array(a) => a,
        _ => return vec![],
    };

    match chain_id {
        // ── Bitcoin (0): {txid, confirmed, block_height, block_time, net_sats}
        0 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let net_sats = e["net_sats"].as_i64()?;
            let confirmed = e["confirmed"].as_bool().unwrap_or(false);
            let block_height = e["block_height"].as_i64();
            let timestamp = e["block_time"].as_f64().unwrap_or(0.0);
            Some(ChainHistoryEntry {
                kind: if net_sats >= 0 { "receive" } else { "send" }.to_string(),
                status: if confirmed { "confirmed" } else { "pending" }.to_string(),
                asset_name: "Bitcoin".to_string(), symbol: "BTC".to_string(),
                chain_name: "Bitcoin".to_string(),
                amount: net_sats.unsigned_abs() as f64 / 1e8,
                counterparty: String::new(), tx_hash: txid.to_string(),
                block_height, timestamp,
            })
        }).collect(),

        // ── Litecoin (5), BCH (6), BSV (22): {txid, amount_sat, block_height, timestamp, is_incoming}
        5 | 6 | 22 => {
            let (asset_name, symbol, chain_name) = match chain_id {
                5  => ("Litecoin",    "LTC", "Litecoin"),
                6  => ("Bitcoin Cash","BCH", "Bitcoin Cash"),
                22 => ("Bitcoin SV",  "BSV", "Bitcoin SV"),
                _  => unreachable!(),
            };
            arr.iter().filter_map(|e| {
                let txid = e["txid"].as_str()?;
                let amount_sat = e["amount_sat"].as_i64()?;
                let block_height = e["block_height"].as_i64();
                let timestamp = e["timestamp"].as_f64().unwrap_or(0.0);
                let is_incoming = e["is_incoming"].as_bool().unwrap_or(amount_sat >= 0);
                Some(ChainHistoryEntry {
                    kind: if is_incoming { "receive" } else { "send" }.to_string(),
                    status: if block_height.unwrap_or(0) > 0 { "confirmed" } else { "pending" }.to_string(),
                    asset_name: asset_name.to_string(), symbol: symbol.to_string(),
                    chain_name: chain_name.to_string(),
                    amount: amount_sat.unsigned_abs() as f64 / 1e8,
                    counterparty: String::new(), tx_hash: txid.to_string(),
                    block_height, timestamp,
                })
            }).collect()
        }

        // ── Dogecoin (3): {txid, amount_koin, block_height, timestamp, is_incoming}
        3 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let amount_koin = e["amount_koin"].as_i64().unwrap_or(0);
            let block_height = e["block_height"].as_i64();
            let timestamp = e["timestamp"].as_f64().unwrap_or(0.0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(amount_koin >= 0);
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: if block_height.unwrap_or(0) > 0 { "confirmed" } else { "pending" }.to_string(),
                asset_name: "Dogecoin".to_string(), symbol: "DOGE".to_string(),
                chain_name: "Dogecoin".to_string(),
                amount: amount_koin.unsigned_abs() as f64 / 1e8,
                counterparty: String::new(), tx_hash: txid.to_string(),
                block_height, timestamp,
            })
        }).collect(),

        // ── XRP (4): {txid, timestamp, from, to, amount_drops, is_incoming}
        4 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let drops = e["amount_drops"].as_u64().unwrap_or(0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp = e["timestamp"].as_f64().unwrap_or(0.0);
            let from = e["from"].as_str().unwrap_or("");
            let to = e["to"].as_str().unwrap_or("");
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "XRP".to_string(), symbol: "XRP".to_string(),
                chain_name: "XRP Ledger".to_string(),
                amount: drops as f64 / 1e6,
                counterparty: (if is_incoming { from } else { to }).to_string(),
                tx_hash: txid.to_string(), block_height: None, timestamp,
            })
        }).collect(),

        // ── Stellar (8): {txid, timestamp (ISO or unix), from/to, amount_stroops, is_incoming}
        8 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let stroops = e["amount_stroops"].as_i64().unwrap_or(0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let from = e["from"].as_str().unwrap_or("");
            let to = e["to"].as_str().unwrap_or("");
            // timestamp can be ISO string or unix number
            let timestamp: f64 = if let Some(n) = e["timestamp"].as_f64() {
                n
            } else if let Some(s) = e["timestamp"].as_str() {
                parse_iso8601_timestamp(s)
            } else { 0.0 };
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "Stellar Lumens".to_string(), symbol: "XLM".to_string(),
                chain_name: "Stellar".to_string(),
                amount: stroops.unsigned_abs() as f64 / 1e7,
                counterparty: (if is_incoming { from } else { to }).to_string(),
                tx_hash: txid.to_string(), block_height: None, timestamp,
            })
        }).collect(),

        // ── Cardano (9): {txid, block_time, amount_lovelace, is_incoming}
        9 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let lovelace = e["amount_lovelace"].as_i64().unwrap_or(0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp = e["block_time"].as_f64().unwrap_or(0.0);
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "Cardano".to_string(), symbol: "ADA".to_string(),
                chain_name: "Cardano".to_string(),
                amount: lovelace.unsigned_abs() as f64 / 1e6,
                counterparty: String::new(), tx_hash: txid.to_string(),
                block_height: None, timestamp,
            })
        }).collect(),

        // ── Polkadot (10): {txid, amount_planck, timestamp, from, to, is_incoming}
        10 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let planck = e["amount_planck"].as_f64().unwrap_or(0.0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp = e["timestamp"].as_f64().unwrap_or(0.0);
            let from = e["from"].as_str().unwrap_or("");
            let to = e["to"].as_str().unwrap_or("");
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "Polkadot".to_string(), symbol: "DOT".to_string(),
                chain_name: "Polkadot".to_string(),
                amount: planck / 1e10,
                counterparty: (if is_incoming { from } else { to }).to_string(),
                tx_hash: txid.to_string(), block_height: None, timestamp,
            })
        }).collect(),

        // ── Solana (2): SolanaTransfer {signature, timestamp, is_incoming, amount_display, symbol, mint, from, to}
        2 => arr.iter().filter_map(|e| {
            let sig = e["signature"].as_str()?;
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let amount: f64 = e["amount_display"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let symbol = e["symbol"].as_str().unwrap_or("SOL");
            let from = e["from"].as_str().unwrap_or("");
            let to = e["to"].as_str().unwrap_or("");
            let timestamp = e["timestamp"].as_f64().unwrap_or(0.0);
            let asset_name = if symbol == "SOL" { "Solana" } else { symbol };
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: asset_name.to_string(), symbol: symbol.to_string(),
                chain_name: "Solana".to_string(), amount,
                counterparty: (if is_incoming { from } else { to }).to_string(),
                tx_hash: sig.to_string(), block_height: None, timestamp,
            })
        }).collect(),

        // ── Tron (7): TronTransfer {txid, timestamp_ms, from, to, amount_display, symbol}
        7 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let amount: f64 = e["amount_display"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let symbol = e["symbol"].as_str().unwrap_or("TRX");
            let from = e["from"].as_str().unwrap_or("");
            let to = e["to"].as_str().unwrap_or("");
            let timestamp_ms = e["timestamp_ms"].as_f64().unwrap_or(0.0);
            let asset_name = tron_asset_name(symbol);
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: asset_name.to_string(), symbol: symbol.to_string(),
                chain_name: "Tron".to_string(), amount,
                counterparty: (if is_incoming { from } else { to }).to_string(),
                tx_hash: txid.to_string(), block_height: None,
                timestamp: timestamp_ms / 1000.0,
            })
        }).collect(),

        // ── Sui (14): {digest, amount_mist, timestamp_ms, is_incoming}
        14 => arr.iter().filter_map(|e| {
            let digest = e["digest"].as_str()?;
            let mist = e["amount_mist"].as_f64().unwrap_or(0.0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp_ms = e["timestamp_ms"].as_f64().unwrap_or(0.0);
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "Sui".to_string(), symbol: "SUI".to_string(),
                chain_name: "Sui".to_string(),
                amount: mist / 1e9,
                counterparty: String::new(), tx_hash: digest.to_string(),
                block_height: None, timestamp: timestamp_ms / 1000.0,
            })
        }).collect(),

        // ── Aptos (15): {txid, amount_octas, timestamp_us, from, to, is_incoming}
        15 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let octas = e["amount_octas"].as_f64().unwrap_or(0.0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp_us = e["timestamp_us"].as_f64().unwrap_or(0.0);
            let from = e["from"].as_str().unwrap_or("");
            let to = e["to"].as_str().unwrap_or("");
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "Aptos".to_string(), symbol: "APT".to_string(),
                chain_name: "Aptos".to_string(),
                amount: octas / 1e8,
                counterparty: (if is_incoming { from } else { to }).to_string(),
                tx_hash: txid.to_string(), block_height: None,
                timestamp: timestamp_us / 1e6,
            })
        }).collect(),

        // ── TON (16): {txid, amount_nanotons, timestamp, from, to, is_incoming}
        16 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let nanotons = e["amount_nanotons"].as_f64().unwrap_or(0.0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp = e["timestamp"].as_f64().unwrap_or(0.0);
            let from = e["from"].as_str().unwrap_or("");
            let to = e["to"].as_str().unwrap_or("");
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "Toncoin".to_string(), symbol: "TON".to_string(),
                chain_name: "TON".to_string(),
                amount: nanotons / 1e9,
                counterparty: (if is_incoming { from } else { to }).to_string(),
                tx_hash: txid.to_string(), block_height: None, timestamp,
            })
        }).collect(),

        // ── NEAR (17): {txid, timestamp_ns, signer_id, receiver_id, amount_yocto, is_incoming}
        17 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let yocto: f64 = e["amount_yocto"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp_ns = e["timestamp_ns"].as_f64().unwrap_or(0.0);
            let signer = e["signer_id"].as_str().unwrap_or("");
            let receiver = e["receiver_id"].as_str().unwrap_or("");
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "NEAR Protocol".to_string(), symbol: "NEAR".to_string(),
                chain_name: "NEAR".to_string(),
                amount: yocto / 1e24,
                counterparty: (if is_incoming { signer } else { receiver }).to_string(),
                tx_hash: txid.to_string(), block_height: None,
                timestamp: timestamp_ns / 1e9,
            })
        }).collect(),

        // ── ICP (18): {block_index, amount_e8s, timestamp_ns, from, to, is_incoming}
        18 => arr.iter().filter_map(|e| {
            let block_index = e["block_index"].as_i64().unwrap_or(0);
            let e8s = e["amount_e8s"].as_f64().unwrap_or(0.0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp_ns = e["timestamp_ns"].as_f64().unwrap_or(0.0);
            let from = e["from"].as_str().unwrap_or("");
            let to = e["to"].as_str().unwrap_or("");
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "Internet Computer".to_string(), symbol: "ICP".to_string(),
                chain_name: "Internet Computer".to_string(),
                amount: e8s / 1e8,
                counterparty: (if is_incoming { from } else { to }).to_string(),
                tx_hash: block_index.to_string(), block_height: None,
                timestamp: timestamp_ns / 1e9,
            })
        }).collect(),

        // ── Monero (19): {txid, amount_piconeros, timestamp, is_incoming}
        19 => arr.iter().filter_map(|e| {
            let txid = e["txid"].as_str()?;
            let piconeros = e["amount_piconeros"].as_f64().unwrap_or(0.0);
            let is_incoming = e["is_incoming"].as_bool().unwrap_or(false);
            let timestamp = e["timestamp"].as_f64().unwrap_or(0.0);
            Some(ChainHistoryEntry {
                kind: if is_incoming { "receive" } else { "send" }.to_string(),
                status: "confirmed".to_string(),
                asset_name: "Monero".to_string(), symbol: "XMR".to_string(),
                chain_name: "Monero".to_string(),
                amount: piconeros / 1e12,
                counterparty: String::new(), tx_hash: txid.to_string(),
                block_height: None, timestamp,
            })
        }).collect(),

        _ => vec![],
    }
}

fn tron_asset_name(symbol: &str) -> &str {
    match symbol {
        "TRX"  => "Tron",
        "USDT" => "Tether USD",
        "USDC" => "USD Coin",
        "BTT"  => "BitTorrent",
        _      => symbol,
    }
}

/// Parse an ISO-8601 timestamp string to Unix seconds.
/// Falls back to 0.0 on failure.
fn parse_iso8601_timestamp(s: &str) -> f64 {
    // Try common formats: "2023-01-01T00:00:00Z" and "2023-01-01T00:00:00+00:00"
    // Use a manual parser to avoid heavy dependencies.
    let s = s.trim();
    // Expect at minimum: "YYYY-MM-DDTHH:MM:SS"
    if s.len() < 19 { return 0.0; }
    let year:  i64 = s[0..4].parse().unwrap_or(0);
    let month: i64 = s[5..7].parse().unwrap_or(0);
    let day:   i64 = s[8..10].parse().unwrap_or(0);
    let hour:  i64 = s[11..13].parse().unwrap_or(0);
    let min:   i64 = s[14..16].parse().unwrap_or(0);
    let sec:   i64 = s[17..19].parse().unwrap_or(0);
    if year == 0 || month == 0 || day == 0 { return 0.0; }
    // Days since Unix epoch (1970-01-01)
    let days = days_from_civil(year, month, day);
    (days * 86400 + hour * 3600 + min * 60 + sec) as f64
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryWallet {
    pub wallet_id: String,
    pub selected_chain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTransaction {
    pub id: String,
    pub wallet_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub wallet_name: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub address: String,
    pub transaction_hash: Option<String>,
    pub transaction_history_source: Option<String>,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeHistoryRequest {
    pub wallets: Vec<HistoryWallet>,
    pub transactions: Vec<HistoryTransaction>,
    pub unknown_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedHistoryEntry {
    pub id: String,
    pub transaction_id: String,
    pub dedupe_key: String,
    pub created_at_unix: f64,
    pub kind: String,
    pub status: String,
    pub wallet_name: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub address: String,
    pub transaction_hash: Option<String>,
    pub source_tag: String,
    pub provider_count: usize,
    pub search_index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BitcoinHistorySnapshot {
    pub txid: String,
    pub amount_btc: f64,
    pub kind: String,
    pub status: String,
    pub counterparty_address: String,
    pub block_height: Option<i64>,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeBitcoinHistorySnapshotsRequest {
    pub snapshots: Vec<BitcoinHistorySnapshot>,
    pub owned_addresses: Vec<String>,
    pub limit: usize,
}

pub fn normalize_history(request: NormalizeHistoryRequest) -> Vec<NormalizedHistoryEntry> {
    let wallet_by_id = request
        .wallets
        .into_iter()
        .map(|wallet| (wallet.wallet_id, wallet.selected_chain))
        .collect::<BTreeMap<_, _>>();

    let mut grouped_by_dedupe_key = BTreeMap::<String, Vec<NormalizedHistoryEntry>>::new();
    for transaction in request.transactions {
        let Some(wallet_id) = transaction.wallet_id.as_ref() else {
            continue;
        };
        let Some(selected_chain) = wallet_by_id.get(wallet_id) else {
            continue;
        };
        if *selected_chain != transaction.chain_name {
            continue;
        }
        let entry = normalized_entry(transaction, &request.unknown_label);
        grouped_by_dedupe_key
            .entry(entry.dedupe_key.clone())
            .or_default()
            .push(entry);
    }

    let mut deduped = grouped_by_dedupe_key
        .into_values()
        .filter_map(|entries| {
            if entries.is_empty() {
                return None;
            }
            let provider_count = entries
                .iter()
                .map(|entry| entry.source_tag.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                .max(1);
            let best = entries
                .into_iter()
                .max_by(|lhs, rhs| compare_entries(lhs, rhs))?;
            Some(NormalizedHistoryEntry {
                provider_count,
                ..best
            })
        })
        .collect::<Vec<_>>();

    deduped.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| lhs.id.cmp(&rhs.id))
    });
    deduped
}

pub fn merge_bitcoin_history_snapshots(
    request: MergeBitcoinHistorySnapshotsRequest,
) -> Vec<BitcoinHistorySnapshot> {
    let owned_addresses = request
        .owned_addresses
        .into_iter()
        .map(|address| address.trim().to_lowercase())
        .filter(|address| !address.is_empty())
        .collect::<std::collections::BTreeSet<_>>();

    let grouped = request.snapshots.into_iter().fold(
        BTreeMap::<String, Vec<BitcoinHistorySnapshot>>::new(),
        |mut grouped, snapshot| {
            grouped
                .entry(snapshot.txid.clone())
                .or_default()
                .push(snapshot);
            grouped
        },
    );

    let mut merged = grouped
        .into_values()
        .filter_map(|entries| merge_bitcoin_snapshot_group(entries, &owned_addresses))
        .collect::<Vec<_>>();

    merged.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| lhs.txid.cmp(&rhs.txid))
    });
    merged.truncate(request.limit.max(1));
    merged
}

fn normalized_entry(
    transaction: HistoryTransaction,
    unknown_label: &str,
) -> NormalizedHistoryEntry {
    let wallet_key = transaction
        .wallet_id
        .clone()
        .unwrap_or_else(|| "unknown-wallet".to_string())
        .to_lowercase();
    let normalized_chain = transaction.chain_name.to_lowercase();
    let normalized_symbol = transaction.symbol.trim().to_lowercase();
    let (dedupe_key, stable_id) = match transaction
        .transaction_hash
        .as_ref()
        .map(|value| value.to_lowercase())
        .filter(|value| !value.is_empty())
    {
        Some(transaction_hash) => {
            let key = format!(
                "{}|{}|{}|{}",
                wallet_key, normalized_chain, normalized_symbol, transaction_hash
            );
            (key.clone(), key)
        }
        None => {
            let key = format!("local|{}|{}", wallet_key, transaction.id.to_lowercase());
            (key.clone(), key)
        }
    };

    let source_tag = normalized_source_tag(
        transaction.transaction_history_source.as_deref(),
        unknown_label,
    );
    let search_index = [
        transaction.wallet_name.as_str(),
        transaction.asset_name.as_str(),
        transaction.symbol.as_str(),
        transaction.chain_name.as_str(),
        transaction.address.as_str(),
        transaction.transaction_hash.as_deref().unwrap_or(""),
        source_tag.as_str(),
    ]
    .join(" ")
    .to_lowercase();

    NormalizedHistoryEntry {
        id: stable_id,
        transaction_id: transaction.id,
        dedupe_key,
        created_at_unix: transaction.created_at_unix,
        kind: transaction.kind,
        status: transaction.status,
        wallet_name: transaction.wallet_name,
        asset_name: transaction.asset_name,
        symbol: transaction.symbol,
        chain_name: transaction.chain_name,
        address: transaction.address,
        transaction_hash: transaction.transaction_hash,
        source_tag,
        provider_count: 1,
        search_index,
    }
}

fn merge_bitcoin_snapshot_group(
    entries: Vec<BitcoinHistorySnapshot>,
    owned_addresses: &std::collections::BTreeSet<String>,
) -> Option<BitcoinHistorySnapshot> {
    if entries.is_empty() {
        return None;
    }

    let net_amount = entries.iter().fold(0.0, |amount, entry| {
        amount
            + if entry.kind == "receive" {
                entry.amount_btc
            } else {
                -entry.amount_btc
            }
    });
    if net_amount == 0.0 {
        return None;
    }

    let mut ordered_entries = entries;
    ordered_entries.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                rhs.amount_btc
                    .partial_cmp(&lhs.amount_btc)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let counterparty_address = ordered_entries
        .iter()
        .map(|entry| entry.counterparty_address.trim().to_string())
        .find(|address| {
            let normalized = address.to_lowercase();
            !normalized.is_empty() && !owned_addresses.contains(&normalized)
        })
        .or_else(|| {
            ordered_entries
                .first()
                .map(|entry| entry.counterparty_address.clone())
        })
        .unwrap_or_default();

    let representative = ordered_entries.first()?.clone();
    Some(BitcoinHistorySnapshot {
        txid: representative.txid,
        amount_btc: net_amount.abs(),
        kind: if net_amount > 0.0 {
            "receive".to_string()
        } else {
            "send".to_string()
        },
        status: if ordered_entries
            .iter()
            .any(|entry| entry.status == "pending")
        {
            "pending".to_string()
        } else {
            "confirmed".to_string()
        },
        counterparty_address,
        block_height: ordered_entries
            .iter()
            .filter_map(|entry| entry.block_height)
            .max(),
        created_at_unix: ordered_entries
            .iter()
            .map(|entry| entry.created_at_unix)
            .fold(representative.created_at_unix, f64::max),
    })
}

fn compare_entries(
    lhs: &NormalizedHistoryEntry,
    rhs: &NormalizedHistoryEntry,
) -> std::cmp::Ordering {
    status_rank(&lhs.status)
        .cmp(&status_rank(&rhs.status))
        .then_with(|| {
            lhs.created_at_unix
                .partial_cmp(&rhs.created_at_unix)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| lhs.transaction_id.cmp(&rhs.transaction_id))
}

fn status_rank(status: &str) -> i32 {
    match status {
        "confirmed" => 3,
        "pending" => 2,
        "failed" => 1,
        _ => 0,
    }
}

fn normalized_source_tag(raw_source: Option<&str>, unknown_label: &str) -> String {
    let trimmed = raw_source.unwrap_or("").trim().to_lowercase();
    if trimmed.is_empty() {
        return unknown_label.to_string();
    }
    match trimmed.as_str() {
        "esplora" => "Esplora".to_string(),
        "litecoinspace" => "LitecoinSpace".to_string(),
        "blockchair" => "Blockchair".to_string(),
        "blockcypher" => "BlockCypher".to_string(),
        "dogecoin.providers" => "DOGE Providers".to_string(),
        "rpc" => "RPC".to_string(),
        "etherscan" => "Etherscan".to_string(),
        "blockscout" => "Blockscout".to_string(),
        "ethplorer" => "Ethplorer".to_string(),
        "none" => unknown_label.to_string(),
        _ => title_case(&trimmed),
    }
}

fn title_case(value: &str) -> String {
    value
        .split_whitespace()
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_history_by_hash_and_prefers_confirmed() {
        let entries = normalize_history(NormalizeHistoryRequest {
            wallets: vec![HistoryWallet {
                wallet_id: "wallet-1".to_string(),
                selected_chain: "Ethereum".to_string(),
            }],
            transactions: vec![
                HistoryTransaction {
                    id: "tx-1".to_string(),
                    wallet_id: Some("wallet-1".to_string()),
                    kind: "send".to_string(),
                    status: "pending".to_string(),
                    wallet_name: "Main".to_string(),
                    asset_name: "Ether".to_string(),
                    symbol: "ETH".to_string(),
                    chain_name: "Ethereum".to_string(),
                    address: "0xabc".to_string(),
                    transaction_hash: Some("0xhash".to_string()),
                    transaction_history_source: Some("rpc".to_string()),
                    created_at_unix: 100.0,
                },
                HistoryTransaction {
                    id: "tx-2".to_string(),
                    wallet_id: Some("wallet-1".to_string()),
                    kind: "send".to_string(),
                    status: "confirmed".to_string(),
                    wallet_name: "Main".to_string(),
                    asset_name: "Ether".to_string(),
                    symbol: "ETH".to_string(),
                    chain_name: "Ethereum".to_string(),
                    address: "0xabc".to_string(),
                    transaction_hash: Some("0xhash".to_string()),
                    transaction_history_source: Some("etherscan".to_string()),
                    created_at_unix: 110.0,
                },
            ],
            unknown_label: "Unknown".to_string(),
        });

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "confirmed");
        assert_eq!(entries[0].provider_count, 2);
    }

    #[test]
    fn merges_bitcoin_inventory_snapshots_into_net_entries() {
        let merged = merge_bitcoin_history_snapshots(MergeBitcoinHistorySnapshotsRequest {
            snapshots: vec![
                BitcoinHistorySnapshot {
                    txid: "tx-1".to_string(),
                    amount_btc: 0.75,
                    kind: "receive".to_string(),
                    status: "confirmed".to_string(),
                    counterparty_address: "bc1-owned".to_string(),
                    block_height: Some(100),
                    created_at_unix: 100.0,
                },
                BitcoinHistorySnapshot {
                    txid: "tx-1".to_string(),
                    amount_btc: 0.25,
                    kind: "send".to_string(),
                    status: "pending".to_string(),
                    counterparty_address: "bc1-other".to_string(),
                    block_height: Some(101),
                    created_at_unix: 110.0,
                },
            ],
            owned_addresses: vec!["bc1-owned".to_string()],
            limit: 25,
        });

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].txid, "tx-1");
        assert_eq!(merged[0].amount_btc, 0.5);
        assert_eq!(merged[0].kind, "receive");
        assert_eq!(merged[0].status, "pending");
        assert_eq!(merged[0].counterparty_address, "bc1-other");
        assert_eq!(merged[0].block_height, Some(101));
        assert_eq!(merged[0].created_at_unix, 110.0);
    }
}
