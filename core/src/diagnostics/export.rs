//! Diagnostics documents: one per chain, and the bundle that carries them.
//!
//! Core builds both from what it recorded. The JSON shape is the exported
//! bundle's contract; every string is sanitized before it leaves.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::registry::RecordedChainDiagnostics;
use super::types::*;
use crate::diagnostics::sanitizer::sanitize_diagnostics_string;
use crate::service::EndpointProbe;

fn pretty_sanitized(value: Value) -> Option<String> {
    let bytes = serde_json::to_vec_pretty(&value).ok()?;
    let s = String::from_utf8(bytes).ok()?;
    Some(sanitize_diagnostics_string(&s))
}

/// One endpoint row. `checked` false means nothing knows how to probe it,
/// which is not a pass.
fn endpoint_row_value(row: &EndpointProbe) -> Value {
    json!({
        "endpoint": row.endpoint,
        "checked": row.checked,
        "reachable": row.reachable,
        "detail": row.detail,
    })
}

// ---------- Chain document ----------

/// One family's diagnostics document: its history rows, and the endpoint
/// check of the network it is on. A time is `null` until its run happens.
pub(crate) fn chain_diagnostics_document(
    family_id: &str,
    network_id: &str,
    recorded: &RecordedChainDiagnostics,
) -> Option<String> {
    let mut payload = Map::new();
    payload.insert("chainId".into(), json!(family_id));
    payload.insert("network".into(), json!(network_id));
    payload.insert(
        "historyLastUpdatedAt".into(),
        json!(recorded.history_run_at_unix),
    );
    payload.insert(
        "endpointsLastUpdatedAt".into(),
        json!(recorded.endpoints_checked_at_unix),
    );
    payload.insert(
        "history".into(),
        Value::Array(recorded.history.iter().map(history_row_value).collect()),
    );
    payload.insert(
        "endpoints".into(),
        Value::Array(recorded.endpoints.iter().map(endpoint_row_value).collect()),
    );
    pretty_sanitized(Value::Object(payload))
}

fn history_row_value(row: &HistoryDiagnostics) -> Value {
    let mut out = Map::new();
    out.insert("walletID".into(), json!(row.wallet_id));
    out.insert("identifier".into(), json!(row.identifier));
    out.insert("sourceUsed".into(), json!(row.source_used));
    out.insert("transactionCount".into(), json!(row.transaction_count));
    out.insert("error".into(), json!(row.error.clone().unwrap_or_default()));
    if let Some(cursor) = &row.next_cursor {
        out.insert("nextCursor".into(), json!(cursor));
    }
    // Present only where the chain decodes and can see more than it can use.
    // The other two numbers are derived rather than stored, so they cannot
    // disagree with the two they come from.
    if let Some(scanned) = row.scanned_count {
        out.insert("scannedCount".into(), json!(scanned));
        out.insert("undecodedCount".into(), json!(row.undecoded_count()));
        out.insert(
            "decodingCompleteness".into(),
            json!(row.decoding_completeness()),
        );
    }
    if !row.per_source.is_empty() {
        out.insert(
            "perSource".into(),
            Value::Array(
                row.per_source
                    .iter()
                    .map(|s| {
                        json!({
                            "name": s.name,
                            "count": s.count,
                            "error": s.error.clone().unwrap_or_default(),
                        })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(out)
}

// ---------- Full diagnostics bundle ----------

/// Complete diagnostics bundle, assembled by `WalletService::diagnostics_bundle`.
/// `generated_at` is a Unix timestamp (f64) so it round-trips losslessly
/// across FFI without depending on Swift date-encoding strategy.
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsBundlePayload {
    pub schema_version: i32,
    pub generated_at: f64,
    pub environment: DiagnosticsEnvironmentMetadata,
    pub chain_degraded: HashMap<String, crate::service::ChainDegradation>,
    /// `Chain::str_id()` of every mainnet → that family's diagnostics document.
    /// Keyed rather than one field per chain: the bundle is
    /// written for human inspection and nothing reads individual chains, so a
    /// map costs nothing and adding a chain stops being a schema change.
    pub chain_diagnostics_json: HashMap<String, String>,
}

/// Serialize a bundle payload to pretty-printed, sanitized JSON. Returns `None`
/// only on the extremely unlikely serialization failure path.
pub fn diagnostics_bundle_to_json(payload: DiagnosticsBundlePayload) -> Option<String> {
    // Redact inside each string value rather than over the rendered text: a
    // redaction that ran across quotes could consume JSON structure.
    fn sanitize(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(s) => *s = sanitize_diagnostics_string(s),
            serde_json::Value::Array(items) => items.iter_mut().for_each(sanitize),
            serde_json::Value::Object(map) => map.values_mut().for_each(sanitize),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(&payload).ok()?;
    sanitize(&mut value);
    serde_json::to_string_pretty(&value).ok()
}

/// Parse a bundle JSON string back into a `DiagnosticsBundlePayload`. Returns
/// `None` if the JSON is malformed or missing required fields.
#[uniffi::export]
pub fn diagnostics_bundle_from_json(json: String) -> Option<DiagnosticsBundlePayload> {
    serde_json::from_str(&json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> HistoryDiagnostics {
        HistoryDiagnostics {
            wallet_id: id.into(),
            identifier: "addr".into(),
            source_used: "rust".into(),
            transaction_count: 5,
            scanned_count: None,
            next_cursor: None,
            error: None,
            per_source: Vec::new(),
        }
    }

    fn recorded(history: Vec<HistoryDiagnostics>) -> RecordedChainDiagnostics {
        RecordedChainDiagnostics {
            history,
            history_run_at_unix: None,
            endpoints: Vec::new(),
            endpoints_checked_at_unix: None,
        }
    }

    #[test]
    fn a_document_names_its_network_and_carries_history_and_endpoints() {
        let s =
            chain_diagnostics_document("bitcoin", "bitcoin-testnet", &recorded(vec![row("w1")]))
                .expect("builds");
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["chainId"], "bitcoin");
        assert_eq!(v["network"], "bitcoin-testnet");
        assert!(
            v["historyLastUpdatedAt"].is_null(),
            "no run is not a run at 1970"
        );
        assert!(v["endpoints"].as_array().unwrap().is_empty());
        assert_eq!(v["history"][0]["walletID"], "w1");
        assert_eq!(v["history"][0]["transactionCount"], 5);
    }

    /// Optional keys are absent rather than empty, so a reader can tell "this
    /// chain has no such thing" from "it has one and it is empty".
    #[test]
    fn optional_keys_are_absent_when_the_chain_has_none() {
        let plain = chain_diagnostics_document("bitcoin", "bitcoin", &recorded(vec![row("w1")]))
            .expect("builds");
        assert!(!plain.contains("nextCursor"));
        assert!(!plain.contains("scannedCount"));
        assert!(!plain.contains("perSource"));

        let mut full = row("w1");
        full.next_cursor = Some("c".into());
        full.scanned_count = Some(10);
        full.transaction_count = 9;
        full.per_source = vec![HistoryDiagnosticsSource {
            name: "rpc".into(),
            count: 1,
            error: None,
        }];
        let s = chain_diagnostics_document("bitcoin", "bitcoin", &recorded(vec![full]))
            .expect("builds");
        assert!(s.contains("\"nextCursor\""));
        assert!(s.contains("\"perSource\""));
        // Derived, not stored.
        assert!(s.contains("\"undecodedCount\": 1"));
        assert!(s.contains("\"decodingCompleteness\""));
    }
}
