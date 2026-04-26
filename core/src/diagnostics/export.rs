// per-chain diagnostics JSON builders.
//
// Lifted from Swift `swift/shell/StoreDiagnosticsExport.swift`. The JSON
// output shape is part of the exported diagnostics bundle contract — keep
// field names stable across migrations.
//
// Each builder takes an already-normalized list of diagnostics records
// (the Swift values of the `*HistoryDiagnosticsByWallet` dictionaries plus
// their matching endpoint-health arrays) and returns a pretty-printed,
// key-sorted, sanitized JSON string. Returning `Option<String>` mirrors
// the Swift helpers that return `String?` on serialization failure.

use serde_json::{json, Map, Value};

use super::types::*;
use crate::diagnostics::sanitizer::sanitize_diagnostics_string;

/// Generic endpoint-health row (matches Swift `BitcoinEndpointHealthResult`
/// and the UTXO/non-EVM chains that reuse its shape).
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct EndpointHealthRow {
    pub endpoint: String,
    pub reachable: bool,
    pub status_code: Option<i32>,
    pub detail: String,
}

/// EVM endpoint-health row (adds a human-readable `label`).
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct EvmEndpointHealthRow {
    pub label: String,
    pub endpoint: String,
    pub reachable: bool,
    pub status_code: Option<i32>,
    pub detail: String,
}

/// EVM history entry keyed by wallet id (so Swift can pass the dictionary
/// values through without collapsing the wallet mapping).
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct EvmHistoryEntry {
    pub wallet_id: String,
    pub diagnostics: EthereumTokenTransferHistoryDiagnostics,
}

/// UTXO history entry. `wallet_id` is carried by
/// `BitcoinHistoryDiagnostics.wallet_id`, so we just pass the value.
pub type UtxoHistoryEntry = BitcoinHistoryDiagnostics;

/// Simple (address/source/count/error) entry paired with the wallet id.
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct SimpleAddressHistoryEntry {
    pub wallet_id: String,
    pub address: String,
    pub source_used: String,
    pub transaction_count: i32,
    pub error: Option<String>,
}

/// Tron history entry with wallet id.
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct TronHistoryEntry {
    pub wallet_id: String,
    pub diagnostics: TronHistoryDiagnostics,
}

/// Solana history entry with wallet id.
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct SolanaHistoryEntry {
    pub wallet_id: String,
    pub diagnostics: SolanaHistoryDiagnostics,
}

// ---------- shared helpers ----------

fn pretty_sanitized(value: Value) -> Option<String> {
    let bytes = serde_json::to_vec_pretty(&value).ok()?;
    let s = String::from_utf8(bytes).ok()?;
    Some(sanitize_diagnostics_string(&s))
}

fn endpoint_row_value(row: &EndpointHealthRow) -> Value {
    json!({
        "endpoint": row.endpoint,
        "reachable": row.reachable,
        "statusCode": row.status_code.unwrap_or(-1),
        "detail": row.detail,
    })
}

fn evm_endpoint_row_value(row: &EvmEndpointHealthRow) -> Value {
    json!({
        "label": row.label,
        "endpoint": row.endpoint,
        "reachable": row.reachable,
        "statusCode": row.status_code.unwrap_or(-1),
        "detail": row.detail,
    })
}

fn unix_or_zero(t: Option<f64>) -> f64 { t.unwrap_or(0.0) }

// ---------- EVM ----------

#[uniffi::export]
pub fn diagnostics_build_evm_json(
    history: Vec<EvmHistoryEntry>,
    endpoints: Vec<EvmEndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history.iter().map(|e| {
        let d = &e.diagnostics;
        json!({
            "walletID": e.wallet_id,
            "address": d.address,
            "rpcTransferCount": d.rpc_transfer_count,
            "rpcError": d.rpc_error.clone().unwrap_or_default(),
            "blockscoutTransferCount": d.blockscout_transfer_count,
            "blockscoutError": d.blockscout_error.clone().unwrap_or_default(),
            "etherscanTransferCount": d.etherscan_transfer_count,
            "etherscanError": d.etherscan_error.clone().unwrap_or_default(),
            "ethplorerTransferCount": d.ethplorer_transfer_count,
            "ethplorerError": d.ethplorer_error.clone().unwrap_or_default(),
            "sourceUsed": d.source_used,
            "transferScanCount": d.transfer_scan_count,
            "decodedTransferCount": d.decoded_transfer_count,
            "unsupportedTransferDropCount": d.unsupported_transfer_drop_count,
            "decodingCompletenessRatio": d.decoding_completeness_ratio,
        })
    }).collect();
    let endpoint_dicts: Vec<Value> = endpoints.iter().map(evm_endpoint_row_value).collect();
    let payload = json!({
        "historyLastUpdatedAt": unix_or_zero(history_last_updated_at_unix),
        "endpointsLastUpdatedAt": unix_or_zero(endpoints_last_updated_at_unix),
        "history": history_dicts,
        "endpoints": endpoint_dicts,
    });
    pretty_sanitized(payload)
}

/// Returns true iff the given diagnostics JSON string parses as an object that
/// contains the top-level `history` and `endpoints` keys produced by
/// `diagnostics_build_evm_json`. Used by the Swift self-test to verify the
/// bundle shape without doing any JSON parsing on the Swift side.
#[uniffi::export]
pub fn core_diagnostics_evm_json_shape_ok(json: String) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(&json) else { return false };
    let Some(obj) = v.as_object() else { return false };
    obj.contains_key("history") && obj.contains_key("endpoints")
}

// ---------- UTXO (Bitcoin-shape) ----------

#[uniffi::export]
pub fn diagnostics_build_utxo_json(
    history: Vec<UtxoHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
    extra_network_mode: Option<String>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history.iter().map(|item| {
        json!({
            "walletID": item.wallet_id,
            "identifier": item.identifier,
            "sourceUsed": item.source_used,
            "transactionCount": item.transaction_count,
            "nextCursor": item.next_cursor.clone().unwrap_or_default(),
            "error": item.error.clone().unwrap_or_default(),
        })
    }).collect();
    let endpoint_dicts: Vec<Value> = endpoints.iter().map(endpoint_row_value).collect();
    let mut payload = Map::new();
    payload.insert("historyLastUpdatedAt".into(), json!(unix_or_zero(history_last_updated_at_unix)));
    payload.insert("endpointsLastUpdatedAt".into(), json!(unix_or_zero(endpoints_last_updated_at_unix)));
    payload.insert("history".into(), Value::Array(history_dicts));
    payload.insert("endpoints".into(), Value::Array(endpoint_dicts));
    if let Some(mode) = extra_network_mode {
        payload.insert("networkMode".into(), Value::String(mode));
    }
    pretty_sanitized(Value::Object(payload))
}

// ---------- Simple address chains ----------

#[uniffi::export]
pub fn diagnostics_build_simple_address_json(
    history: Vec<SimpleAddressHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history.iter().map(|item| {
        json!({
            "walletID": item.wallet_id,
            "address": item.address,
            "sourceUsed": item.source_used,
            "transactionCount": item.transaction_count,
            "error": item.error.clone().unwrap_or_default(),
        })
    }).collect();
    let endpoint_dicts: Vec<Value> = endpoints.iter().map(endpoint_row_value).collect();
    let payload = json!({
        "historyLastUpdatedAt": unix_or_zero(history_last_updated_at_unix),
        "endpointsLastUpdatedAt": unix_or_zero(endpoints_last_updated_at_unix),
        "history": history_dicts,
        "endpoints": endpoint_dicts,
    });
    pretty_sanitized(payload)
}

// ---------- Tron ----------

#[uniffi::export]
pub fn diagnostics_build_tron_json(
    history: Vec<TronHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
    last_send_error_at_unix: Option<f64>,
    last_send_error_details: Option<String>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history.iter().map(|e| {
        let d = &e.diagnostics;
        json!({
            "walletID": e.wallet_id,
            "address": d.address,
            "tronScanTxCount": d.tron_scan_tx_count,
            "tronScanTRC20Count": d.tron_scan_trc20_count,
            "sourceUsed": d.source_used,
            "error": d.error.clone().unwrap_or_default(),
        })
    }).collect();
    let endpoint_dicts: Vec<Value> = endpoints.iter().map(endpoint_row_value).collect();
    let payload = json!({
        "historyLastUpdatedAt": unix_or_zero(history_last_updated_at_unix),
        "endpointsLastUpdatedAt": unix_or_zero(endpoints_last_updated_at_unix),
        "lastSendErrorAt": unix_or_zero(last_send_error_at_unix),
        "lastSendErrorDetails": last_send_error_details.unwrap_or_default(),
        "history": history_dicts,
        "endpoints": endpoint_dicts,
    });
    pretty_sanitized(payload)
}

// ---------- Solana ----------

#[uniffi::export]
pub fn diagnostics_build_solana_json(
    history: Vec<SolanaHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history.iter().map(|e| {
        let d = &e.diagnostics;
        json!({
            "walletID": e.wallet_id,
            "address": d.address,
            "rpcCount": d.rpc_count,
            "sourceUsed": d.source_used,
            "error": d.error.clone().unwrap_or_default(),
        })
    }).collect();
    let endpoint_dicts: Vec<Value> = endpoints.iter().map(endpoint_row_value).collect();
    let payload = json!({
        "historyLastUpdatedAt": unix_or_zero(history_last_updated_at_unix),
        "endpointsLastUpdatedAt": unix_or_zero(endpoints_last_updated_at_unix),
        "history": history_dicts,
        "endpoints": endpoint_dicts,
    });
    pretty_sanitized(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_json_contains_expected_shape() {
        let s = diagnostics_build_evm_json(
            vec![EvmHistoryEntry {
                wallet_id: "w1".into(),
                diagnostics: EthereumTokenTransferHistoryDiagnostics {
                    address: "0xabc".into(),
                    rpc_transfer_count: 1,
                    rpc_error: None,
                    blockscout_transfer_count: 2,
                    blockscout_error: Some("boom".into()),
                    etherscan_transfer_count: 3,
                    etherscan_error: None,
                    ethplorer_transfer_count: 4,
                    ethplorer_error: None,
                    source_used: "rust".into(),
                    transfer_scan_count: 10,
                    decoded_transfer_count: 9,
                    unsupported_transfer_drop_count: 1,
                    decoding_completeness_ratio: 0.9,
                },
            }],
            vec![EvmEndpointHealthRow {
                label: "alchemy".into(),
                endpoint: "https://example".into(),
                reachable: true,
                status_code: Some(200),
                detail: "ok".into(),
            }],
            Some(1.0),
            Some(2.0),
        ).expect("builds");
        assert!(s.contains("\"walletID\""));
        assert!(s.contains("\"rpcTransferCount\""));
        assert!(s.contains("\"historyLastUpdatedAt\""));
        assert!(s.contains("\"label\""));
    }

    #[test]
    fn utxo_json_includes_network_mode_when_set() {
        let s = diagnostics_build_utxo_json(
            vec![UtxoHistoryEntry {
                wallet_id: "w1".into(),
                identifier: "addr".into(),
                source_used: "rust".into(),
                transaction_count: 3,
                next_cursor: Some("c".into()),
                error: None,
            }],
            vec![],
            None,
            None,
            Some("mainnet".into()),
        ).expect("builds");
        assert!(s.contains("\"networkMode\""));
        assert!(s.contains("mainnet"));
    }

    #[test]
    fn utxo_json_omits_network_mode_when_none() {
        let s = diagnostics_build_utxo_json(
            vec![],
            vec![],
            None,
            None,
            None,
        ).expect("builds");
        assert!(!s.contains("networkMode"));
    }

    #[test]
    fn simple_address_round_trip_fields() {
        let s = diagnostics_build_simple_address_json(
            vec![SimpleAddressHistoryEntry {
                wallet_id: "w1".into(),
                address: "addr".into(),
                source_used: "rust".into(),
                transaction_count: 7,
                error: Some("err".into()),
            }],
            vec![EndpointHealthRow {
                endpoint: "u".into(),
                reachable: false,
                status_code: None,
                detail: "x".into(),
            }],
            Some(10.0),
            None,
        ).expect("builds");
        assert!(s.contains("\"walletID\""));
        assert!(s.contains("\"transactionCount\""));
        assert!(s.contains("-1"));
    }

    #[test]
    fn tron_json_includes_send_error_fields() {
        let s = diagnostics_build_tron_json(
            vec![],
            vec![],
            None,
            None,
            Some(42.0),
            Some("details".into()),
        ).expect("builds");
        assert!(s.contains("\"lastSendErrorAt\""));
        assert!(s.contains("\"lastSendErrorDetails\""));
        assert!(s.contains("details"));
    }

    #[test]
    fn solana_json_has_rpc_count() {
        let s = diagnostics_build_solana_json(
            vec![SolanaHistoryEntry {
                wallet_id: "w1".into(),
                diagnostics: SolanaHistoryDiagnostics {
                    address: "S".into(),
                    rpc_count: 9,
                    source_used: "rpc".into(),
                    error: None,
                },
            }],
            vec![],
            None,
            None,
        ).expect("builds");
        assert!(s.contains("\"rpcCount\""));
    }
}
