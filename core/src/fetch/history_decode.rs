// Typed decode helpers for the JSON shapes that the Swift layer
// previously parsed inline in `StoreHistoryRefresh.swift`.
//
// These helpers exist so Swift no longer needs to run
// `JSONDecoder`/`JSONSerialization` over Rust-produced payloads —
// it can call a typed UniFFI function and get native records back.
//
// Additionally we expose the small `HistoryChainID` enum-like mapping
// that Swift used to duplicate as a private struct of constants.

use serde_json::Value;

// ────────────────────────────────────────────────────────────────────
// Normalized chain history — typed item produced by
// `WalletService::fetch_normalized_history` (see `history::ChainHistoryEntry`).
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, uniffi::Record)]
pub struct NormalizedHistoryItem {
    pub kind: String,
    pub status: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub amount: f64,
    pub counterparty: String,
    pub tx_hash: String,
    pub block_height: Option<i64>,
    pub timestamp: f64,
}

// ────────────────────────────────────────────────────────────────────
// EVM history page decode — shape produced by
// `fetch_evm_history_page_json` (an object with `tokens` and `native`).
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmTokenTransferItem {
    pub contract_address: String,
    pub token_name: String,
    pub symbol: String,
    pub decimals: i32,
    pub from_address: String,
    pub to_address: String,
    /// Decimal amount serialized as a string so Swift can reconstruct a
    /// `Decimal` without floating-point loss.
    pub amount_decimal: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub log_index: i64,
    pub timestamp: f64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmNativeTransferItem {
    pub from_address: String,
    pub to_address: String,
    pub amount_decimal: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub timestamp: f64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmHistoryPageDecoded {
    pub tokens: Vec<EvmTokenTransferItem>,
    pub native: Vec<EvmNativeTransferItem>,
}

pub(crate) fn decimal_string_from_wei(wei_str: &str) -> String {
    // Divide the integer wei string by 1e18 without floats.
    let digits: &str = wei_str.trim_start_matches('-');
    let negative = wei_str.starts_with('-');
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return "0".to_string();
    }
    let (int_part, frac_part) = if digits.len() <= 18 {
        let pad = "0".repeat(18 - digits.len());
        ("0".to_string(), format!("{pad}{digits}"))
    } else {
        let split = digits.len() - 18;
        (digits[..split].to_string(), digits[split..].to_string())
    };
    let frac_trimmed = frac_part.trim_end_matches('0');
    let body = if frac_trimmed.is_empty() {
        int_part
    } else {
        format!("{int_part}.{frac_trimmed}")
    };
    if negative { format!("-{body}") } else { body }
}

fn decimal_string_from_raw(raw: &str, decimals: i32) -> String {
    if decimals <= 0 {
        return raw.to_string();
    }
    let negative = raw.starts_with('-');
    let digits: &str = raw.trim_start_matches('-');
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return "0".to_string();
    }
    let d = decimals as usize;
    let (int_part, frac_part) = if digits.len() <= d {
        let pad = "0".repeat(d - digits.len());
        ("0".to_string(), format!("{pad}{digits}"))
    } else {
        let split = digits.len() - d;
        (digits[..split].to_string(), digits[split..].to_string())
    };
    let frac_trimmed = frac_part.trim_end_matches('0');
    let body = if frac_trimmed.is_empty() {
        int_part
    } else {
        format!("{int_part}.{frac_trimmed}")
    };
    if negative { format!("-{body}") } else { body }
}

pub fn history_decode_evm_page(json: String) -> EvmHistoryPageDecoded {
    let empty = EvmHistoryPageDecoded { tokens: vec![], native: vec![] };
    let Ok(obj) = serde_json::from_str::<Value>(&json) else {
        return empty;
    };
    let Some(obj) = obj.as_object() else { return empty };

    let mut tokens: Vec<EvmTokenTransferItem> = Vec::new();
    if let Some(raw_tokens) = obj.get("tokens").and_then(Value::as_array) {
        for item in raw_tokens {
            let Some(contract) = item.get("contract").and_then(Value::as_str) else { continue };
            let Some(symbol) = item.get("symbol").and_then(Value::as_str) else { continue };
            let Some(token_name) = item.get("token_name").and_then(Value::as_str) else { continue };
            let Some(from_addr) = item.get("from").and_then(Value::as_str) else { continue };
            let Some(to_addr) = item.get("to").and_then(Value::as_str) else { continue };
            let Some(txid) = item.get("txid").and_then(Value::as_str) else { continue };
            let Some(block_num) = item.get("block_number").and_then(Value::as_i64) else { continue };
            let Some(log_idx) = item.get("log_index").and_then(Value::as_i64) else { continue };
            let Some(tsecs) = item.get("timestamp").and_then(Value::as_f64) else { continue };
            let decimals = item.get("decimals").and_then(Value::as_i64).unwrap_or(18) as i32;
            let amount_decimal = if let Some(display) = item.get("amount_display").and_then(Value::as_str) {
                display.to_string()
            } else if let Some(raw) = item.get("amount_raw").and_then(Value::as_str) {
                decimal_string_from_raw(raw, decimals)
            } else {
                "0".to_string()
            };
            tokens.push(EvmTokenTransferItem {
                contract_address: contract.to_string(),
                token_name: token_name.to_string(),
                symbol: symbol.to_string(),
                decimals,
                from_address: from_addr.to_string(),
                to_address: to_addr.to_string(),
                amount_decimal,
                transaction_hash: txid.to_string(),
                block_number: block_num,
                log_index: log_idx,
                timestamp: tsecs,
            });
        }
    }

    let mut native: Vec<EvmNativeTransferItem> = Vec::new();
    if let Some(raw_native) = obj.get("native").and_then(Value::as_array) {
        for item in raw_native {
            let Some(from_addr) = item.get("from").and_then(Value::as_str) else { continue };
            let Some(to_addr) = item.get("to").and_then(Value::as_str) else { continue };
            let Some(txid) = item.get("txid").and_then(Value::as_str) else { continue };
            let Some(block_num) = item.get("block_number").and_then(Value::as_i64) else { continue };
            let Some(tsecs) = item.get("timestamp").and_then(Value::as_f64) else { continue };
            let Some(wei_str) = item.get("value_wei").and_then(Value::as_str) else { continue };
            native.push(EvmNativeTransferItem {
                from_address: from_addr.to_string(),
                to_address: to_addr.to_string(),
                amount_decimal: decimal_string_from_wei(wei_str),
                transaction_hash: txid.to_string(),
                block_number: block_num,
                timestamp: tsecs,
            });
        }
    }

    EvmHistoryPageDecoded { tokens, native }
}

// ────────────────────────────────────────────────────────────────────
// EVM history page → per-wallet transaction record projection.
// Given a decoded page and the target wallets, emits one record per
// (wallet × matching transfer) where "matching" means the transfer
// touches the wallet's normalized address as sender or receiver.
// Swift wraps each output in `TransactionRecord` with a fresh UUID.
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmPlannedTransactionRecord {
    pub wallet_id: String,
    pub wallet_name: String,
    pub kind: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub amount_decimal: String,
    pub counterparty: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub source_address: String,
    pub source_used: String,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmTransactionRecordWalletInput {
    pub wallet_id: String,
    pub wallet_name: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmTransactionRecordRequest {
    pub decoded_page: EvmHistoryPageDecoded,
    pub normalized_address: String,
    pub chain_name: String,
    pub token_source_used: Option<String>,
    pub native_asset_name: String,
    pub native_asset_symbol: String,
    pub wallets: Vec<EvmTransactionRecordWalletInput>,
    pub unknown_timestamp_sentinel_unix: f64,
}

#[uniffi::export]
pub fn plan_evm_transaction_records(
    request: EvmTransactionRecordRequest,
) -> Vec<EvmPlannedTransactionRecord> {
    let normalized = request.normalized_address;
    let token_source = request
        .token_source_used
        .unwrap_or_else(|| "none".to_string());
    let native_source = "etherscan".to_string();
    let mut out: Vec<EvmPlannedTransactionRecord> = Vec::new();

    for wallet in &request.wallets {
        for transfer in &request.decoded_page.tokens {
            let is_outgoing = transfer.from_address == normalized;
            let is_incoming = transfer.to_address == normalized;
            if !is_outgoing && !is_incoming {
                continue;
            }
            let (counterparty, wallet_side) = if is_outgoing {
                (transfer.to_address.clone(), transfer.from_address.clone())
            } else {
                (transfer.from_address.clone(), transfer.to_address.clone())
            };
            let created_at = if transfer.timestamp > 0.0 {
                transfer.timestamp
            } else {
                request.unknown_timestamp_sentinel_unix
            };
            out.push(EvmPlannedTransactionRecord {
                wallet_id: wallet.wallet_id.clone(),
                wallet_name: wallet.wallet_name.clone(),
                kind: if is_outgoing { "send" } else { "receive" }.to_string(),
                asset_name: transfer.token_name.clone(),
                symbol: transfer.symbol.clone(),
                chain_name: request.chain_name.clone(),
                amount_decimal: transfer.amount_decimal.clone(),
                counterparty,
                transaction_hash: transfer.transaction_hash.clone(),
                block_number: transfer.block_number,
                source_address: wallet_side,
                source_used: token_source.clone(),
                created_at_unix: created_at,
            });
        }
        for transfer in &request.decoded_page.native {
            let is_outgoing = transfer.from_address == normalized;
            let is_incoming = transfer.to_address == normalized;
            if !is_outgoing && !is_incoming {
                continue;
            }
            let (counterparty, wallet_side) = if is_outgoing {
                (transfer.to_address.clone(), transfer.from_address.clone())
            } else {
                (transfer.from_address.clone(), transfer.to_address.clone())
            };
            let created_at = if transfer.timestamp > 0.0 {
                transfer.timestamp
            } else {
                request.unknown_timestamp_sentinel_unix
            };
            out.push(EvmPlannedTransactionRecord {
                wallet_id: wallet.wallet_id.clone(),
                wallet_name: wallet.wallet_name.clone(),
                kind: if is_outgoing { "send" } else { "receive" }.to_string(),
                asset_name: request.native_asset_name.clone(),
                symbol: request.native_asset_symbol.clone(),
                chain_name: request.chain_name.clone(),
                amount_decimal: transfer.amount_decimal.clone(),
                counterparty,
                transaction_hash: transfer.transaction_hash.clone(),
                block_number: transfer.block_number,
                source_address: wallet_side,
                source_used: native_source.clone(),
                created_at_unix: created_at,
            });
        }
    }
    out
}

// ────────────────────────────────────────────────────────────────────
// HistoryChainID mapping — moved out of Swift.
// ────────────────────────────────────────────────────────────────────

// ────────────────────────────────────────────────────────────────────
// Dogecoin per-wallet aggregation: groups normalized entries by
// transaction hash, nets signed amounts, picks a counterparty, and
// produces a single aggregated record per hash.
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, uniffi::Record)]
pub struct DogecoinAggregateInput {
    pub own_addresses: Vec<String>,
    pub entries: Vec<NormalizedHistoryItem>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DogecoinAggregatedTx {
    pub hash: String,
    pub kind: String,
    pub status: String,
    pub amount: f64,
    pub counterparty: String,
    pub block_number: Option<i64>,
    /// Earliest known non-distant-past timestamp (Unix seconds). 0 when unknown.
    pub created_at_unix: f64,
}

#[uniffi::export]
pub fn history_aggregate_dogecoin(input: DogecoinAggregateInput) -> Vec<DogecoinAggregatedTx> {
    use std::collections::HashMap;
    let own: std::collections::HashSet<String> =
        input.own_addresses.into_iter().map(|a| a.to_lowercase()).collect();
    let mut by_hash: HashMap<String, Vec<NormalizedHistoryItem>> = HashMap::new();
    for e in input.entries {
        if e.tx_hash.is_empty() { continue; }
        by_hash.entry(e.tx_hash.clone()).or_default().push(e);
    }
    let mut out = Vec::new();
    for (_hash, group) in by_hash {
        let Some(first) = group.first().cloned() else { continue };
        let signed: f64 = group.iter().map(|s| if s.kind == "receive" { s.amount } else { -s.amount }).sum();
        if signed.abs() == 0.0 { continue; }
        let kind = if signed > 0.0 { "receive" } else { "send" };
        let amount = signed.abs();
        let status = if group.iter().any(|s| s.status == "pending") { "pending" } else { "confirmed" };
        let block_number = group.iter().filter_map(|s| s.block_height).max();
        let known_ts: Vec<f64> = group.iter().filter_map(|s| if s.timestamp > 0.0 { Some(s.timestamp) } else { None }).collect();
        let created_at_unix = known_ts.iter().copied().fold(f64::INFINITY, f64::min);
        let created_at_unix = if created_at_unix.is_finite() { created_at_unix } else { first.timestamp };
        let counterparty = group.iter()
            .map(|s| s.counterparty.clone())
            .find(|c| {
                let trimmed = c.trim();
                !trimmed.is_empty() && !own.contains(&c.to_lowercase())
            })
            .unwrap_or_else(|| first.counterparty.clone());
        out.push(DogecoinAggregatedTx {
            hash: first.tx_hash.clone(), kind: kind.into(), status: status.into(),
            amount, counterparty, block_number, created_at_unix,
        });
    }
    out
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmNativeAsset {
    pub asset_name: String,
    pub symbol: String,
}

/// Native asset name/symbol for an EVM `chain_name`. Returns `None` when
/// the chain name is not a known EVM chain.
#[uniffi::export]
pub fn history_evm_native_asset(chain_name: String) -> Option<EvmNativeAsset> {
    let (name, symbol) = match chain_name.as_str() {
        "Ethereum" | "Ethereum Sepolia" | "Ethereum Hoodi" | "Arbitrum" | "Optimism" => ("Ether", "ETH"),
        "Avalanche" => ("Avalanche", "AVAX"),
        "BNB Chain" => ("BNB", "BNB"),
        "Ethereum Classic" => ("Ethereum Classic", "ETC"),
        "Hyperliquid" => ("Hyperliquid", "HYPE"),
        _ => return None,
    };
    Some(EvmNativeAsset { asset_name: name.into(), symbol: symbol.into() })
}

/// Map a user-visible chain name to the `chain_id` used by the
/// history pagination store. Returns `None` for unsupported names.
/// Mirrors the private `HistoryChainID` table in Swift.
#[uniffi::export]
pub fn history_pagination_chain_id(chain_name: String) -> Option<u32> {
    Some(match chain_name.as_str() {
        "Bitcoin" => 0,
        "Ethereum" => 1,
        "Dogecoin" => 3,
        "Litecoin" => 5,
        "Bitcoin Cash" => 6,
        "Tron" => 7,
        "Arbitrum" => 11,
        "Optimism" => 12,
        "Bitcoin SV" => 22,
        "BNB Chain" => 23,
        "Hyperliquid" => 24,
        _ => return None,
    })
}

// ────────────────────────────────────────────────────────────────────
// Bitcoin raw-history decode: turns the `fetch_history` Bitcoin JSON
// (array of `{txid, net_sats, confirmed, block_height, block_time}`)
// into `CoreBitcoinHistorySnapshot` payloads ready for the merge step.
// ────────────────────────────────────────────────────────────────────

use crate::history::CoreBitcoinHistorySnapshot;

pub fn history_decode_bitcoin_raw_snapshots(json: String) -> Vec<CoreBitcoinHistorySnapshot> {
    let Ok(arr) = serde_json::from_str::<Vec<Value>>(&json) else {
        return Vec::new();
    };
    arr.into_iter()
        .filter_map(|entry| {
            let txid = entry.get("txid").and_then(Value::as_str)?;
            let net_sats = entry.get("net_sats").and_then(Value::as_i64).unwrap_or(0);
            let confirmed = entry.get("confirmed").and_then(Value::as_bool).unwrap_or(false);
            let block_height = entry.get("block_height").and_then(Value::as_i64);
            let created_at_unix = entry.get("block_time").and_then(Value::as_f64).unwrap_or(0.0);
            Some(CoreBitcoinHistorySnapshot {
                txid: txid.to_string(),
                amount_btc: net_sats.unsigned_abs() as f64 / 100_000_000.0,
                kind: if net_sats >= 0 { "receive" } else { "send" }.to_string(),
                status: if confirmed { "confirmed" } else { "pending" }.to_string(),
                counterparty_address: String::new(),
                block_height,
                created_at_unix,
            })
        })
        .collect()
}

/// Extract the `address` field from each entry in a JSON array of
/// `{ "address": ..., ... }` objects (the shape returned by
/// `derive_bitcoin_hd_addresses_json`). Non-string/missing entries are skipped.
pub fn history_decode_hd_addresses(json: String) -> Vec<String> {
    serde_json::from_str::<Vec<Value>>(&json)
        .ok()
        .map(|arr| {
            arr.into_iter()
                .filter_map(|e| e.get("address").and_then(Value::as_str).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_evm_page_empty() {
        let d = history_decode_evm_page("not json".into());
        assert!(d.tokens.is_empty() && d.native.is_empty());
        let d = history_decode_evm_page("{}".into());
        assert!(d.tokens.is_empty() && d.native.is_empty());
    }

    #[test]
    fn decode_evm_page_tokens_and_native() {
        let json = r#"{
          "tokens":[{
            "contract":"0xabc","symbol":"USDC","token_name":"USD Coin",
            "decimals":6,"from":"0x1","to":"0x2","txid":"0xhash",
            "block_number":123,"log_index":4,"timestamp":1700000000.0,
            "amount_raw":"1500000"
          }],
          "native":[{
            "from":"0x1","to":"0x2","txid":"0xhash2",
            "block_number":456,"timestamp":1700000001.0,
            "value_wei":"1000000000000000000"
          }]
        }"#;
        let d = history_decode_evm_page(json.into());
        assert_eq!(d.tokens.len(), 1);
        assert_eq!(d.tokens[0].amount_decimal, "1.5");
        assert_eq!(d.native.len(), 1);
        assert_eq!(d.native[0].amount_decimal, "1");
    }

    #[test]
    fn plans_evm_transaction_records_for_matching_transfers() {
        let page = EvmHistoryPageDecoded {
            tokens: vec![EvmTokenTransferItem {
                contract_address: "0xabc".into(), token_name: "USD Coin".into(),
                symbol: "USDC".into(), decimals: 6,
                from_address: "0xself".into(), to_address: "0xother".into(),
                amount_decimal: "1.5".into(), transaction_hash: "0xhash".into(),
                block_number: 100, log_index: 0, timestamp: 1700000000.0,
            }],
            native: vec![EvmNativeTransferItem {
                from_address: "0xother".into(), to_address: "0xself".into(),
                amount_decimal: "0.25".into(), transaction_hash: "0xhash2".into(),
                block_number: 101, timestamp: 0.0,
            }],
        };
        let out = plan_evm_transaction_records(EvmTransactionRecordRequest {
            decoded_page: page,
            normalized_address: "0xself".into(),
            chain_name: "Ethereum".into(),
            token_source_used: Some("rust/etherscan".into()),
            native_asset_name: "Ether".into(),
            native_asset_symbol: "ETH".into(),
            wallets: vec![EvmTransactionRecordWalletInput {
                wallet_id: "w1".into(), wallet_name: "Primary".into(),
            }],
            unknown_timestamp_sentinel_unix: -1.0,
        });
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, "send");
        assert_eq!(out[0].symbol, "USDC");
        assert_eq!(out[0].counterparty, "0xother");
        assert_eq!(out[0].source_address, "0xself");
        assert_eq!(out[0].created_at_unix, 1700000000.0);
        assert_eq!(out[1].kind, "receive");
        assert_eq!(out[1].symbol, "ETH");
        assert_eq!(out[1].source_used, "etherscan");
        assert_eq!(out[1].created_at_unix, -1.0);
    }

    #[test]
    fn plans_evm_transaction_records_skips_unrelated_transfers() {
        let page = EvmHistoryPageDecoded {
            tokens: vec![EvmTokenTransferItem {
                contract_address: "0xabc".into(), token_name: "USD Coin".into(),
                symbol: "USDC".into(), decimals: 6,
                from_address: "0xA".into(), to_address: "0xB".into(),
                amount_decimal: "1".into(), transaction_hash: "0xhash".into(),
                block_number: 100, log_index: 0, timestamp: 1700000000.0,
            }],
            native: vec![],
        };
        let out = plan_evm_transaction_records(EvmTransactionRecordRequest {
            decoded_page: page,
            normalized_address: "0xself".into(),
            chain_name: "Ethereum".into(),
            token_source_used: None,
            native_asset_name: "Ether".into(),
            native_asset_symbol: "ETH".into(),
            wallets: vec![EvmTransactionRecordWalletInput {
                wallet_id: "w1".into(), wallet_name: "Primary".into(),
            }],
            unknown_timestamp_sentinel_unix: -1.0,
        });
        assert!(out.is_empty());
    }

    #[test]
    fn wei_conversion_fractional() {
        assert_eq!(decimal_string_from_wei("1500000000000000000"), "1.5");
        assert_eq!(decimal_string_from_wei("500000000000000"), "0.0005");
        assert_eq!(decimal_string_from_wei("0"), "0");
    }

    #[test]
    fn raw_to_decimal_fractional() {
        assert_eq!(decimal_string_from_raw("1500000", 6), "1.5");
        assert_eq!(decimal_string_from_raw("1", 18), "0.000000000000000001");
        assert_eq!(decimal_string_from_raw("100", 0), "100");
    }

    #[test]
    fn btc_raw_snapshots_decode() {
        let json = r#"[
            {"txid":"a","net_sats":150000000,"confirmed":true,"block_height":800000,"block_time":1700000000.0},
            {"txid":"b","net_sats":-25000,"confirmed":false}
        ]"#;
        let out = history_decode_bitcoin_raw_snapshots(json.into());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].amount_btc, 1.5);
        assert_eq!(out[0].kind, "receive");
        assert_eq!(out[0].status, "confirmed");
        assert_eq!(out[1].kind, "send");
        assert_eq!(out[1].status, "pending");
    }

    #[test]
    fn hd_addresses_decode() {
        let json = r#"[{"address":"addr1"},{"address":"addr2"},{"foo":"x"}]"#;
        assert_eq!(history_decode_hd_addresses(json.into()), vec!["addr1", "addr2"]);
        assert!(history_decode_hd_addresses("garbage".into()).is_empty());
    }

    #[test]
    fn dogecoin_aggregate_nets_amounts() {
        let entry = |kind: &str, amount, counterparty: &str, ts: f64, status: &str| NormalizedHistoryItem {
            kind: kind.into(), status: status.into(),
            asset_name: "Dogecoin".into(), symbol: "DOGE".into(), chain_name: "Dogecoin".into(),
            amount, counterparty: counterparty.into(), tx_hash: "tx1".into(),
            block_height: Some(100), timestamp: ts,
        };
        let out = history_aggregate_dogecoin(DogecoinAggregateInput {
            own_addresses: vec!["Own1".into()],
            entries: vec![
                entry("receive", 10.0, "External", 1700000000.0, "confirmed"),
                entry("send", 3.0, "Own1", 1700000005.0, "confirmed"),
            ],
        });
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, "receive");
        assert!((out[0].amount - 7.0).abs() < 1e-9);
        assert_eq!(out[0].counterparty, "External");
        assert_eq!(out[0].created_at_unix, 1700000000.0);
    }

    #[test]
    fn evm_native_asset_lookup() {
        let eth = history_evm_native_asset("Ethereum".into()).unwrap();
        assert_eq!(eth.symbol, "ETH");
        let bnb = history_evm_native_asset("BNB Chain".into()).unwrap();
        assert_eq!(bnb.asset_name, "BNB");
        assert!(history_evm_native_asset("Bitcoin".into()).is_none());
    }

    #[test]
    fn chain_id_mapping() {
        assert_eq!(history_pagination_chain_id("Bitcoin".into()), Some(0));
        assert_eq!(history_pagination_chain_id("Hyperliquid".into()), Some(24));
        assert_eq!(history_pagination_chain_id("Nope".into()), None);
    }
}
