//! Per-chain diagnostics JSON builders.
//!
//! The JSON output shape is part of the exported diagnostics bundle contract —
//! keep field names stable across migrations.
//!
//! Each builder takes an already-normalized list of diagnostics records and
//! returns a pretty-printed, sanitized JSON string. `Option<String>` return
//! type mirrors the Swift helpers that return `String?` on serialization failure.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::types::*;
use crate::diagnostics::sanitizer::sanitize_diagnostics_string;

/// One endpoint's reachability, for every chain.
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct EndpointHealthRow {
    /// Human-readable name for the endpoint. Empty for chains whose
    /// diagnostics list endpoints without one.
    #[serde(default)]
    pub label: String,
    pub endpoint: String,
    pub reachable: bool,
    pub status_code: Option<i32>,
    pub detail: String,
}

// ---------- shared helpers ----------

fn pretty_sanitized(value: Value) -> Option<String> {
    let bytes = serde_json::to_vec_pretty(&value).ok()?;
    let s = String::from_utf8(bytes).ok()?;
    Some(sanitize_diagnostics_string(&s))
}

/// One endpoint's row.
///
/// There were two of these — one emitting `label` and one not — chosen by which
/// chain family was being built. A row that has a label carries it; one that
/// does not omits the key rather than printing an empty string.
fn endpoint_row_value(row: &EndpointHealthRow) -> Value {
    let mut out = Map::new();
    if !row.label.is_empty() {
        out.insert("label".into(), json!(row.label));
    }
    out.insert("endpoint".into(), json!(row.endpoint));
    out.insert("reachable".into(), json!(row.reachable));
    out.insert("statusCode".into(), json!(row.status_code.unwrap_or(-1)));
    out.insert("detail".into(), json!(row.detail));
    Value::Object(out)
}

fn unix_or_zero(t: Option<f64>) -> f64 {
    t.unwrap_or(0.0)
}

// ---------- History JSON ----------

/// One chain's diagnostics document.
///
/// There were five of these, one per record shape, and they built the same
/// payload — `historyLastUpdatedAt`, `endpointsLastUpdatedAt`, `history[]`,
/// `endpoints[]` — differing only in the field names inside a row. Rows are
/// uniform now, so the document is one function and adding a chain is not a
/// new builder.
pub fn diagnostics_build_history_json(
    history: Vec<HistoryDiagnostics>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
    extra_network_mode: Option<String>,
    last_send_error_at_unix: Option<f64>,
    last_send_error_details: Option<String>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history.iter().map(history_row_value).collect();
    let endpoint_dicts: Vec<Value> = endpoints.iter().map(endpoint_row_value).collect();
    let mut payload = Map::new();
    payload.insert(
        "historyLastUpdatedAt".into(),
        json!(unix_or_zero(history_last_updated_at_unix)),
    );
    payload.insert(
        "endpointsLastUpdatedAt".into(),
        json!(unix_or_zero(endpoints_last_updated_at_unix)),
    );
    payload.insert("history".into(), Value::Array(history_dicts));
    payload.insert("endpoints".into(), Value::Array(endpoint_dicts));
    // Only two chains have anything else to say, and saying nothing is not the
    // same as saying "none" — an absent key reads as "this chain has no such
    // thing", a present empty one as "it has one and it is empty".
    if let Some(mode) = extra_network_mode {
        payload.insert("networkMode".into(), Value::String(mode));
    }
    if last_send_error_at_unix.is_some() || last_send_error_details.is_some() {
        payload.insert(
            "lastSendErrorAt".into(),
            json!(unix_or_zero(last_send_error_at_unix)),
        );
        payload.insert(
            "lastSendErrorDetails".into(),
            Value::String(last_send_error_details.unwrap_or_default()),
        );
    }
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

/// True iff the JSON parses as an object carrying the top-level `history` and
/// `endpoints` keys `diagnostics_build_history_json` produces. Used by the
/// self-test to check the bundle shape without parsing JSON in Swift.
#[uniffi::export]
pub fn core_diagnostics_json_shape_ok(json: String) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(&json) else {
        return false;
    };
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.contains_key("history") && obj.contains_key("endpoints")
}

// ---------- UTXO (Bitcoin-shape) ----------

// ---------- Full diagnostics bundle ----------

/// Complete diagnostics bundle. All chain JSON fields are non-optional — callers
/// supply `"{}"` as a fallback for chains with no data. `generated_at` is a
/// Unix timestamp (f64) so it round-trips losslessly across FFI without
/// depending on Swift date-encoding strategy.
#[derive(uniffi::Record, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsBundlePayload {
    pub schema_version: i32,
    pub generated_at: f64,
    pub environment: DiagnosticsEnvironmentMetadata,
    pub chain_degraded_messages: HashMap<String, String>,
    /// `Chain::str_id()` → that chain's diagnostics JSON blob (`"{}"` when the
    /// chain has no data). Keyed rather than one field per chain: the bundle is
    /// written for human inspection and nothing reads individual chains, so a
    /// map costs nothing and adding a chain stops being a schema change.
    pub chain_diagnostics_json: HashMap<String, String>,
}

/// Serialize a bundle payload to pretty-printed, sanitized JSON. Returns `None`
/// only on the extremely unlikely serialization failure path.
#[uniffi::export]
pub fn diagnostics_bundle_to_json(payload: DiagnosticsBundlePayload) -> Option<String> {
    let bytes = serde_json::to_vec_pretty(&payload).ok()?;
    let s = String::from_utf8(bytes).ok()?;
    Some(sanitize_diagnostics_string(&s))
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

    #[test]
    fn a_document_carries_history_and_endpoints() {
        let s = diagnostics_build_history_json(vec![row("w1")], vec![], None, None, None, None, None)
            .expect("builds");
        assert!(core_diagnostics_json_shape_ok(s.clone()));
        assert!(s.contains("\"walletID\""));
        assert!(s.contains("\"identifier\""));
        assert!(s.contains("\"transactionCount\""));
    }

    /// Optional keys are absent rather than empty, so a reader can tell "this
    /// chain has no such thing" from "it has one and it is empty".
    #[test]
    fn optional_keys_are_absent_when_the_chain_has_none() {
        let plain =
            diagnostics_build_history_json(vec![row("w1")], vec![], None, None, None, None, None)
                .expect("builds");
        assert!(!plain.contains("nextCursor"));
        assert!(!plain.contains("scannedCount"));
        assert!(!plain.contains("perSource"));
        assert!(!plain.contains("networkMode"));
        assert!(!plain.contains("lastSendErrorAt"));

        let mut full = row("w1");
        full.next_cursor = Some("c".into());
        full.scanned_count = Some(10);
        full.transaction_count = 9;
        full.per_source = vec![HistoryDiagnosticsSource {
            name: "rpc".into(),
            count: 1,
            error: None,
        }];
        let s = diagnostics_build_history_json(
            vec![full],
            vec![],
            None,
            None,
            Some("testnet".into()),
            Some(42.0),
            Some("details".into()),
        )
        .expect("builds");
        assert!(s.contains("\"nextCursor\""));
        assert!(s.contains("\"perSource\""));
        assert!(s.contains("\"networkMode\"") && s.contains("testnet"));
        assert!(s.contains("\"lastSendErrorDetails\"") && s.contains("details"));
        // Derived, not stored.
        assert!(s.contains("\"undecodedCount\": 1"));
        assert!(s.contains("\"decodingCompleteness\""));
    }
}

/// The diagnostics document for one chain.
///
/// One export in place of five builders, and it takes no `history` argument
/// because core owns that now — the caller used to read core's registry, hand
/// the rows straight back across the FFI, and receive JSON built from them.
/// It does not match on chain at all: every chain records the same row.
#[uniffi::export]
pub fn core_diagnostics_json(
    chain_name: String,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
    extra_network_mode: Option<String>,
    last_send_error_at_unix: Option<f64>,
    last_send_error_details: Option<String>,
) -> Option<String> {
    use crate::diagnostics::registry as reg;

    // No shape to dispatch on: every chain records the same row, so the
    // document is the same document.
    crate::registry::Chain::from_display_name(&chain_name)?;
    let history: Vec<HistoryDiagnostics> = reg::diagnostics_all(chain_name).into_values().collect();
    diagnostics_build_history_json(
        history,
        endpoints,
        history_last_updated_at_unix,
        endpoints_last_updated_at_unix,
        extra_network_mode,
        last_send_error_at_unix,
        last_send_error_details,
    )
}

#[cfg(test)]
mod one_builder_tests {
    use super::*;
    use crate::registry::Chain;

    /// Every chain the diagnostics bundle reports on produces a document.
    ///
    /// Five separate builders could not state this; a chain simply had no
    /// builder and nothing said so.
    #[test]
    fn every_chain_produces_a_document() {
        for chain in Chain::all().filter(|c| !c.is_testnet()) {
            let json = core_diagnostics_json(
                chain.chain_display_name().to_string(),
                Vec::new(),
                None,
                None,
                None,
                None,
                None,
            );
            assert!(
                json.is_some(),
                "{} produced no diagnostics document",
                chain.chain_display_name()
            );
        }
    }

}
