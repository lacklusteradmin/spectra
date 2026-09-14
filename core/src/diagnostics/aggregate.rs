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
    fn evm_success_counts_native() {
        use crate::fetch::history_decode::{EvmHistoryPageDecoded, EvmNativeTransferItem};
        let page = EvmHistoryPageDecoded {
            tokens: vec![],
            native: (0..3)
                .map(|_| EvmNativeTransferItem {
                    status: "confirmed".into(),
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
            s.per_source
                .iter()
                .find(|p| p.name == "etherscan")
                .map(|p| p.count),
            Some(3)
        );
    }

    #[test]
    fn history_summary_combines() {
        let s = diagnostics_history_summary(r#"[{"txid":"AA"},{"txid":"bb"},{"other":1}]"#.into());
        assert_eq!(s.entry_count, 3);
        assert_eq!(s.confirmed_txids, vec!["aa".to_string(), "bb".to_string()]);
    }
}
