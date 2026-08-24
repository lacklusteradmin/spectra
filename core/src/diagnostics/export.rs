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
///
/// `label` stays out of the non-EVM JSON: `endpoint_row_value` does not emit it
/// and `evm_endpoint_row_value` does, so the bundle shape is unchanged.
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

fn evm_endpoint_row_value(row: &EndpointHealthRow) -> Value {
    json!({
        "label": row.label,
        "endpoint": row.endpoint,
        "reachable": row.reachable,
        "statusCode": row.status_code.unwrap_or(-1),
        "detail": row.detail,
    })
}

fn unix_or_zero(t: Option<f64>) -> f64 {
    t.unwrap_or(0.0)
}

// ---------- EVM ----------

pub fn diagnostics_build_evm_json(
    history: Vec<EvmHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history
        .iter()
        .map(|e| {
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
        })
        .collect();
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
    let Ok(v) = serde_json::from_str::<Value>(&json) else {
        return false;
    };
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.contains_key("history") && obj.contains_key("endpoints")
}

// ---------- UTXO (Bitcoin-shape) ----------

pub fn diagnostics_build_utxo_json(
    history: Vec<UtxoHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
    extra_network_mode: Option<String>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history
        .iter()
        .map(|item| {
            json!({
                "walletID": item.wallet_id,
                "identifier": item.identifier,
                "sourceUsed": item.source_used,
                "transactionCount": item.transaction_count,
                "nextCursor": item.next_cursor.clone().unwrap_or_default(),
                "error": item.error.clone().unwrap_or_default(),
            })
        })
        .collect();
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
    if let Some(mode) = extra_network_mode {
        payload.insert("networkMode".into(), Value::String(mode));
    }
    pretty_sanitized(Value::Object(payload))
}

// ---------- Simple address chains ----------

pub fn diagnostics_build_simple_address_json(
    history: Vec<SimpleAddressHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history
        .iter()
        .map(|item| {
            json!({
                "walletID": item.wallet_id,
                "address": item.address,
                "sourceUsed": item.source_used,
                "transactionCount": item.transaction_count,
                "error": item.error.clone().unwrap_or_default(),
            })
        })
        .collect();
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

pub fn diagnostics_build_tron_json(
    history: Vec<TronHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
    last_send_error_at_unix: Option<f64>,
    last_send_error_details: Option<String>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history
        .iter()
        .map(|e| {
            let d = &e.diagnostics;
            json!({
                "walletID": e.wallet_id,
                "address": d.address,
                "tronScanTxCount": d.tron_scan_tx_count,
                "tronScanTRC20Count": d.tron_scan_trc20_count,
                "sourceUsed": d.source_used,
                "error": d.error.clone().unwrap_or_default(),
            })
        })
        .collect();
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

pub fn diagnostics_build_solana_json(
    history: Vec<SolanaHistoryEntry>,
    endpoints: Vec<EndpointHealthRow>,
    history_last_updated_at_unix: Option<f64>,
    endpoints_last_updated_at_unix: Option<f64>,
) -> Option<String> {
    let history_dicts: Vec<Value> = history
        .iter()
        .map(|e| {
            let d = &e.diagnostics;
            json!({
                "walletID": e.wallet_id,
                "address": d.address,
                "rpcCount": d.rpc_count,
                "sourceUsed": d.source_used,
                "error": d.error.clone().unwrap_or_default(),
            })
        })
        .collect();
    let endpoint_dicts: Vec<Value> = endpoints.iter().map(endpoint_row_value).collect();
    let payload = json!({
        "historyLastUpdatedAt": unix_or_zero(history_last_updated_at_unix),
        "endpointsLastUpdatedAt": unix_or_zero(endpoints_last_updated_at_unix),
        "history": history_dicts,
        "endpoints": endpoint_dicts,
    });
    pretty_sanitized(payload)
}

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
            vec![EndpointHealthRow {
                label: "alchemy".into(),
                endpoint: "https://example".into(),
                reachable: true,
                status_code: Some(200),
                detail: "ok".into(),
            }],
            Some(1.0),
            Some(2.0),
        )
        .expect("builds");
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
        )
        .expect("builds");
        assert!(s.contains("\"networkMode\""));
        assert!(s.contains("mainnet"));
    }

    #[test]
    fn utxo_json_omits_network_mode_when_none() {
        let s = diagnostics_build_utxo_json(vec![], vec![], None, None, None).expect("builds");
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
                label: String::new(),
                endpoint: "u".into(),
                reachable: false,
                status_code: None,
                detail: "x".into(),
            }],
            Some(10.0),
            None,
        )
        .expect("builds");
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
        )
        .expect("builds");
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
        )
        .expect("builds");
        assert!(s.contains("\"rpcCount\""));
    }
}

/// The diagnostics document for one chain.
///
/// One export in place of five builders, and it takes no `history` argument
/// because core owns that now — the caller used to read core's registry, hand
/// the rows straight back across the FFI, and receive JSON built from them.
/// Which shape a chain reports is `Chain::diagnostics_shape`, a registry fact,
/// so this function does not match on chain names at all.
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
    use crate::registry::{Chain, DiagnosticsShape};

    let chain = Chain::from_display_name(&chain_name)?;
    match chain.diagnostics_shape() {
        DiagnosticsShape::Utxo => {
            let history = reg::diagnostics_all_utxo(chain_name)
                .into_values()
                // `UtxoHistoryEntry` is an alias for the record itself — it
                // already carries its wallet id.
                .collect();
            diagnostics_build_utxo_json(
                history,
                endpoints,
                history_last_updated_at_unix,
                endpoints_last_updated_at_unix,
                extra_network_mode,
            )
        }
        DiagnosticsShape::Evm => {
            let history = reg::diagnostics_all_evm(chain_name)
                .into_iter()
                .map(|(wallet_id, diagnostics)| EvmHistoryEntry {
                    wallet_id,
                    diagnostics,
                })
                .collect();
            diagnostics_build_evm_json(
                history,
                endpoints,
                history_last_updated_at_unix,
                endpoints_last_updated_at_unix,
            )
        }
        DiagnosticsShape::Simple => {
            let history = reg::diagnostics_all_simple(chain_name)
                .into_iter()
                .map(|(wallet_id, d)| SimpleAddressHistoryEntry {
                    wallet_id,
                    address: d.address,
                    source_used: d.source_used,
                    transaction_count: d.transaction_count,
                    error: d.error,
                })
                .collect();
            diagnostics_build_simple_address_json(
                history,
                endpoints,
                history_last_updated_at_unix,
                endpoints_last_updated_at_unix,
            )
        }
        DiagnosticsShape::Tron => {
            let history = reg::diagnostics_all_tron()
                .into_iter()
                .map(|(wallet_id, diagnostics)| TronHistoryEntry {
                    wallet_id,
                    diagnostics,
                })
                .collect();
            diagnostics_build_tron_json(
                history,
                endpoints,
                history_last_updated_at_unix,
                endpoints_last_updated_at_unix,
                last_send_error_at_unix,
                last_send_error_details,
            )
        }
        DiagnosticsShape::Solana => {
            let history = reg::diagnostics_all_solana()
                .into_iter()
                .map(|(wallet_id, diagnostics)| SolanaHistoryEntry {
                    wallet_id,
                    diagnostics,
                })
                .collect();
            diagnostics_build_solana_json(
                history,
                endpoints,
                history_last_updated_at_unix,
                endpoints_last_updated_at_unix,
            )
        }
    }
}

#[cfg(test)]
mod one_builder_tests {
    use super::*;
    use crate::registry::{Chain, DiagnosticsShape};

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

    #[test]
    fn shapes_cover_the_families_they_claim() {
        assert_eq!(Chain::Bitcoin.diagnostics_shape(), DiagnosticsShape::Utxo);
        assert_eq!(Chain::Dogecoin.diagnostics_shape(), DiagnosticsShape::Utxo);
        assert_eq!(Chain::Ethereum.diagnostics_shape(), DiagnosticsShape::Evm);
        assert_eq!(Chain::Arbitrum.diagnostics_shape(), DiagnosticsShape::Evm);
        assert_eq!(Chain::Tron.diagnostics_shape(), DiagnosticsShape::Tron);
        assert_eq!(Chain::Solana.diagnostics_shape(), DiagnosticsShape::Solana);
        assert_eq!(Chain::Xrp.diagnostics_shape(), DiagnosticsShape::Simple);
        // A testnet reports the same shape as its mainnet.
        assert_eq!(
            Chain::BitcoinTestnet.diagnostics_shape(),
            DiagnosticsShape::Utxo
        );
    }
}
