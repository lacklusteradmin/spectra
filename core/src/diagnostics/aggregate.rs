// Pure aggregation + JSON-parsing helpers for diagnostics: count rows,
// extract status maps, and build diagnostic records from raw history /
// RPC responses. Unit-tested in one place so the decoding shape stays
// stable across the chain clients that feed in.

use serde_json::Value;

use super::types::{HistoryDiagnostics, HistoryDiagnosticsSource};

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
        .and_then(|v| {
            v.get("native")
                .and_then(Value::as_array)
                .map(|a| a.len() as u32)
        })
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

/// The four backends an EVM history run asks, in the order it asks them.
const EVM_HISTORY_SOURCES: &[&str] = &["rpc", "blockscout", "etherscan", "ethplorer"];

fn evm_record(
    wallet_id: String,
    address: String,
    source_used: &str,
    counts: &[i32; 4],
    errors: [Option<String>; 4],
    scanned: Option<i32>,
    decoded: i32,
) -> HistoryDiagnostics {
    HistoryDiagnostics {
        wallet_id,
        identifier: normalize_evm_address(&address),
        source_used: source_used.to_string(),
        transaction_count: decoded,
        scanned_count: scanned,
        next_cursor: None,
        error: errors.iter().flatten().next().cloned(),
        per_source: EVM_HISTORY_SOURCES
            .iter()
            .zip(counts)
            .zip(errors)
            .map(|((name, count), error)| HistoryDiagnosticsSource {
                name: (*name).to_string(),
                count: *count,
                error,
            })
            .collect(),
    }
}

/// The placeholder shown while a refresh is in flight.
#[uniffi::export]
pub fn diagnostics_make_evm_running(wallet_id: String, address: String) -> HistoryDiagnostics {
    evm_record(
        wallet_id,
        address,
        "running",
        &[0; 4],
        [Some("Running...".into()), None, None, None],
        None,
        0,
    )
}

/// Seeded when a refresh failed. `error_description` is the message the caller
/// would otherwise surface.
pub fn diagnostics_make_evm_error(
    wallet_id: String,
    address: String,
    error_description: String,
) -> HistoryDiagnostics {
    evm_record(
        wallet_id,
        address,
        "none",
        &[0; 4],
        [Some(error_description), None, None, None],
        None,
        0,
    )
}

/// Built from a decoded history page.
///
/// The count goes in `transaction_count` — what the run ended up with — as well
/// as against the backend that produced it. The old record put it in
/// `etherscan_transfer_count` alone and left `decoded_transfer_count` at zero,
/// so every successful EVM refresh reported "0 decoded" and a decoding
/// completeness of 0%.
pub fn diagnostics_make_evm_success_record(
    wallet_id: String,
    address: String,
    page: &crate::fetch::history_decode::EvmHistoryPageDecoded,
) -> HistoryDiagnostics {
    let decoded = page.native.len() as i32;
    evm_record(
        wallet_id,
        address,
        "rust",
        &[0, 0, decoded, 0],
        [None, None, None, None],
        Some(decoded),
        decoded,
    )
}

/// Outcome of a JSON-RPC reachability probe: whether the endpoint answered,
/// and a human-readable detail line for the diagnostics screen.
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
    let detail = error_message.unwrap_or_else(|| format!("HTTP {}", status_code.unwrap_or(-1)));
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
        assert_eq!(
            diagnostics_history_entry_count(r#"{"foo":"bar"}"#.into()),
            0
        );
        assert_eq!(diagnostics_history_entry_count("[]".into()), 0);
    }

    #[test]
    fn evm_native_count() {
        assert_eq!(
            diagnostics_evm_history_native_count(r#"{"native":[1,2,3],"tokens":[]}"#.into()),
            3
        );
        assert_eq!(
            diagnostics_evm_history_native_count(r#"{"tokens":[]}"#.into()),
            0
        );
        assert_eq!(diagnostics_evm_history_native_count("bogus".into()), 0);
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
        let r = diagnostics_make_evm_running("w".into(), "0xAbCDef".into());
        assert_eq!(r.identifier, "0xabcdef");
        assert_eq!(r.source_used, "running");
        assert_eq!(r.error.as_deref(), Some("Running..."));
        assert_eq!(r.per_source.len(), 4, "one entry per backend asked");

        let e = diagnostics_make_evm_error("w".into(), "0xAA".into(), "boom".into());
        assert_eq!(e.identifier, "0xaa");
        assert_eq!(e.source_used, "none");
        assert_eq!(e.error.as_deref(), Some("boom"));
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
        let s = diagnostics_make_evm_success_record("w".into(), "0xAB".into(), &page);
        assert_eq!(s.source_used, "rust");
        assert_eq!(s.identifier, "0xab");
        // The count is what the run ended up with, not only a per-backend
        // number: the old record left `decoded_transfer_count` at zero here,
        // so a successful refresh reported nothing decoded.
        assert_eq!(s.transaction_count, 3);
        assert_eq!(s.decoding_completeness(), 1.0);
        assert_eq!(
            s.per_source.iter().find(|p| p.name == "etherscan").map(|p| p.count),
            Some(3)
        );
    }

    #[test]
    fn jsonrpc_probe_classifies_result() {
        let ok =
            diagnostics_parse_jsonrpc_probe(Some(200), r#"{"jsonrpc":"2.0","result":{}}"#.into());
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
        let s = diagnostics_history_summary(r#"[{"txid":"AA"},{"txid":"bb"},{"other":1}]"#.into());
        assert_eq!(s.entry_count, 3);
        assert_eq!(s.confirmed_txids, vec!["aa".to_string(), "bb".to_string()]);
    }
}
