// Phase C: pure aggregation / JSON-parsing helpers lifted from
// Swift `AppState+DiagnosticsEndpoints.swift`.
//
// These helpers compute counts, extract status maps, and build
// diagnostic records from raw history JSON / RPC responses. Keeping
// them in Rust drops hand-rolled `JSONSerialization.jsonObject(with:)`
// blocks from Swift and lets us unit-test the decoding shape once.

use serde_json::Value;

use super::types::EthereumTokenTransferHistoryDiagnostics;

/// EVM address normalization used by the diagnostics layer. Mirrors
/// `normalizeEVMAddress` in `Send/SendPreviewTypes.swift`: lowercase
/// and trim whitespace. Kept here so Rust-side constructors produce
/// identical values.
fn normalize_evm_address(address: &str) -> String {
    address.trim().to_lowercase()
}

/// Count top-level entries in a Rust-shaped history JSON payload
/// (an array of `{ "txid": "...", ... }` records). Returns 0 for
/// any parse failure or non-array payload — matching Swift's
/// `(try? JSONSerialization...).map { ... } ?? 0` semantics.
pub fn diagnostics_history_entry_count(json: String) -> u32 {
    serde_json::from_str::<Value>(&json)
        .ok()
        .and_then(|v| v.as_array().map(|a| a.len() as u32))
        .unwrap_or(0)
}

/// Count entries in the `native` array of an EVM history-page JSON response.
/// Returns 0 when the key is missing or the payload is malformed.
pub fn diagnostics_evm_history_native_count(json: String) -> u32 {
    serde_json::from_str::<Value>(&json)
        .ok()
        .and_then(|v| v.get("native").and_then(Value::as_array).map(|a| a.len() as u32))
        .unwrap_or(0)
}

/// Return the set of confirmed `txid`s from a Rust-shaped history
/// JSON payload, lowercased and trimmed. Used by
/// `refreshPendingRustHistoryChainTransactions` to mark known-confirmed
/// transactions.
pub fn diagnostics_history_confirmed_txids(json: String) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(&json) else {
        return Vec::new();
    };
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| entry.get("txid").and_then(Value::as_str))
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// EVM diagnostics record seeded to the "running" placeholder shown
/// while a refresh is in flight.
#[uniffi::export]
pub fn diagnostics_make_evm_running(address: String) -> EthereumTokenTransferHistoryDiagnostics {
    EthereumTokenTransferHistoryDiagnostics {
        address: normalize_evm_address(&address),
        rpc_transfer_count: 0,
        rpc_error: Some("Running...".into()),
        blockscout_transfer_count: 0,
        blockscout_error: None,
        etherscan_transfer_count: 0,
        etherscan_error: None,
        ethplorer_transfer_count: 0,
        ethplorer_error: None,
        source_used: "running".into(),
        transfer_scan_count: 0,
        decoded_transfer_count: 0,
        unsupported_transfer_drop_count: 0,
        decoding_completeness_ratio: 0.0,
    }
}

/// EVM diagnostics record seeded when a refresh failed. `error_description`
/// is the message the caller would normally surface (e.g. `error.localizedDescription`).
pub fn diagnostics_make_evm_error(
    address: String,
    error_description: String,
) -> EthereumTokenTransferHistoryDiagnostics {
    EthereumTokenTransferHistoryDiagnostics {
        address: normalize_evm_address(&address),
        rpc_transfer_count: 0,
        rpc_error: Some(error_description),
        blockscout_transfer_count: 0,
        blockscout_error: None,
        etherscan_transfer_count: 0,
        etherscan_error: None,
        ethplorer_transfer_count: 0,
        ethplorer_error: None,
        source_used: "none".into(),
        transfer_scan_count: 0,
        decoded_transfer_count: 0,
        unsupported_transfer_drop_count: 0,
        decoding_completeness_ratio: 0.0,
    }
}

/// Build an EVM diagnostics record from a decoded history page.
/// Used by `WalletService::fetch_evm_history_diagnostics`.
pub fn diagnostics_make_evm_success_record(
    address: String,
    page: &crate::fetch::history_decode::EvmHistoryPageDecoded,
) -> EthereumTokenTransferHistoryDiagnostics {
    EthereumTokenTransferHistoryDiagnostics {
        address: normalize_evm_address(&address),
        rpc_transfer_count: 0,
        rpc_error: None,
        blockscout_transfer_count: 0,
        blockscout_error: None,
        etherscan_transfer_count: page.native.len() as i32,
        etherscan_error: None,
        ethplorer_transfer_count: 0,
        ethplorer_error: None,
        source_used: "rust".into(),
        transfer_scan_count: 0,
        decoded_transfer_count: 0,
        unsupported_transfer_drop_count: 0,
        decoding_completeness_ratio: 0.0,
    }
}

/// Outcome of a JSON-RPC reachability probe. Mirrors the ad-hoc
/// `(reachable, detail)` tuple that Swift used to compute inline
/// while probing Near / Polkadot RPC endpoints.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct JsonRpcProbeOutcome {
    pub reachable: bool,
    pub detail: String,
}

/// Decide the reachable/detail outcome of a JSON-RPC probe given the
/// HTTP status code and raw response body. A probe is considered
/// reachable iff:
///   * the HTTP status is 2xx, **and**
///   * the JSON body decodes and contains a top-level `result` key.
///
/// When unreachable the detail prefers the JSON-RPC `error.message`
/// (if present) and falls back to `HTTP <code>`.
#[uniffi::export]
pub fn diagnostics_parse_jsonrpc_probe(
    status_code: Option<i32>,
    body_utf8: String,
) -> JsonRpcProbeOutcome {
    let http_ok = matches!(status_code, Some(c) if (200..=299).contains(&c));
    let json: Option<Value> = serde_json::from_str(&body_utf8).ok();
    let has_result = json
        .as_ref()
        .and_then(|v| v.get("result"))
        .map(|_| true)
        .unwrap_or(false);
    let reachable = http_ok && has_result;
    if reachable {
        return JsonRpcProbeOutcome {
            reachable: true,
            detail: "OK".into(),
        };
    }
    let error_message = json
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let detail = error_message
        .unwrap_or_else(|| format!("HTTP {}", status_code.unwrap_or(-1)));
    JsonRpcProbeOutcome {
        reachable: false,
        detail,
    }
}

/// Convenience for Swift call sites: partition a history JSON payload
/// into (entry_count, confirmed_txids) in one FFI hop. Useful where
/// callers need both (e.g. UTXO diagnostics + pending-refresh).
pub fn diagnostics_history_summary(json: String) -> HistorySummary {
    let entries: Vec<Value> = serde_json::from_str::<Value>(&json)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let count = entries.len() as u32;
    let confirmed = entries
        .iter()
        .filter_map(|e| e.get("txid").and_then(Value::as_str))
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    HistorySummary {
        entry_count: count,
        confirmed_txids: confirmed,
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct HistorySummary {
    pub entry_count: u32,
    pub confirmed_txids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_count_handles_array_and_garbage() {
        assert_eq!(
            diagnostics_history_entry_count(r#"[{"txid":"a"},{"txid":"b"}]"#.into()),
            2
        );
        assert_eq!(diagnostics_history_entry_count("not-json".into()), 0);
        assert_eq!(diagnostics_history_entry_count(r#"{"foo":"bar"}"#.into()), 0);
        assert_eq!(diagnostics_history_entry_count("[]".into()), 0);
    }

    #[test]
    fn evm_native_count() {
        assert_eq!(
            diagnostics_evm_history_native_count(
                r#"{"native":[1,2,3],"tokens":[]}"#.into()
            ),
            3
        );
        assert_eq!(
            diagnostics_evm_history_native_count(r#"{"tokens":[]}"#.into()),
            0
        );
        assert_eq!(
            diagnostics_evm_history_native_count("bogus".into()),
            0
        );
    }

    #[test]
    fn confirmed_txids_lowercased_and_trimmed() {
        let out = diagnostics_history_confirmed_txids(
            r#"[{"txid":"  ABC "},{"txid":""},{"txid":"def"},{"other":"x"}]"#.into(),
        );
        assert_eq!(out, vec!["abc".to_string(), "def".to_string()]);
    }

    #[test]
    fn evm_running_and_error_records() {
        let r = diagnostics_make_evm_running("0xAbCDef".into());
        assert_eq!(r.address, "0xabcdef");
        assert_eq!(r.source_used, "running");
        assert_eq!(r.rpc_error.as_deref(), Some("Running..."));

        let e = diagnostics_make_evm_error("0xAA".into(), "boom".into());
        assert_eq!(e.address, "0xaa");
        assert_eq!(e.source_used, "none");
        assert_eq!(e.rpc_error.as_deref(), Some("boom"));
    }

    #[test]
    fn evm_success_counts_native() {
        use crate::fetch::history_decode::{EvmHistoryPageDecoded, EvmNativeTransferItem};
        let page = EvmHistoryPageDecoded {
            tokens: vec![],
            native: (0..3)
                .map(|_| EvmNativeTransferItem {
                    from_address: String::new(),
                    to_address: String::new(),
                    amount_decimal: "0".into(),
                    transaction_hash: String::new(),
                    block_number: 0,
                    timestamp: 0.0,
                })
                .collect(),
        };
        let s = diagnostics_make_evm_success_record("0xAB".into(), &page);
        assert_eq!(s.etherscan_transfer_count, 3);
        assert_eq!(s.source_used, "rust");
        assert_eq!(s.address, "0xab");
    }

    #[test]
    fn jsonrpc_probe_classifies_result() {
        let ok = diagnostics_parse_jsonrpc_probe(
            Some(200),
            r#"{"jsonrpc":"2.0","result":{}}"#.into(),
        );
        assert!(ok.reachable);
        assert_eq!(ok.detail, "OK");

        let err = diagnostics_parse_jsonrpc_probe(
            Some(200),
            r#"{"error":{"message":"bad method"}}"#.into(),
        );
        assert!(!err.reachable);
        assert_eq!(err.detail, "bad method");

        let http_err = diagnostics_parse_jsonrpc_probe(Some(503), "<html/>".into());
        assert!(!http_err.reachable);
        assert_eq!(http_err.detail, "HTTP 503");

        let no_code = diagnostics_parse_jsonrpc_probe(None, "".into());
        assert!(!no_code.reachable);
        assert_eq!(no_code.detail, "HTTP -1");
    }

    #[test]
    fn history_summary_combines() {
        let s = diagnostics_history_summary(
            r#"[{"txid":"AA"},{"txid":"bb"},{"other":1}]"#.into(),
        );
        assert_eq!(s.entry_count, 3);
        assert_eq!(s.confirmed_txids, vec!["aa".to_string(), "bb".to_string()]);
    }
}
