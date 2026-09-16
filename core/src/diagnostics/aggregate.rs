// Pure JSON-parsing helper for diagnostics: reduce a raw history payload to
// the two numbers a caller needs. Unit-tested here so the decoding shape stays
// stable across the chain clients that feed in.

use serde_json::Value;

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
    fn history_summary_combines() {
        let s = diagnostics_history_summary(r#"[{"txid":"AA"},{"txid":"bb"},{"other":1}]"#.into());
        assert_eq!(s.entry_count, 3);
        assert_eq!(s.confirmed_txids, vec!["aa".to_string(), "bb".to_string()]);
    }
}
