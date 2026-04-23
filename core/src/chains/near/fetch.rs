//! NEAR fetch paths: view_account balance, access-key nonce, latest block hash,
//! history (indexer), NEP-141 FT balance + metadata, and the UniFFI-exported
//! `near_parse_history_response` used by the Swift layer.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::http::RetryProfile;

use super::{
    NearBalance, NearClient, NearFtBalance, NearFtMetadata, NearHistoryEntry,
    NearHistoryParsedSnapshot,
};

impl NearClient {
    pub async fn fetch_balance(&self, account_id: &str) -> Result<NearBalance, String> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_account",
                    "finality": "final",
                    "account_id": account_id
                }),
            )
            .await?;
        let yocto = result
            .get("amount")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        let display = format_near(&yocto);
        Ok(NearBalance {
            yocto_near: yocto,
            near_display: display,
        })
    }

    pub async fn fetch_access_key_nonce(
        &self,
        account_id: &str,
        public_key_b58: &str,
    ) -> Result<u64, String> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_access_key",
                    "finality": "final",
                    "account_id": account_id,
                    "public_key": format!("ed25519:{public_key_b58}")
                }),
            )
            .await?;
        result
            .get("nonce")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "view_access_key: missing nonce".to_string())
    }

    pub async fn fetch_latest_block_hash(&self) -> Result<String, String> {
        let result = self.call("block", json!({"finality": "final"})).await?;
        result
            .pointer("/header/hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "block: missing hash".to_string())
    }

    /// Fetch transaction history via NEAR Explorer API (indexer).
    pub async fn fetch_history(
        &self,
        account_id: &str,
        indexer_base: &str,
    ) -> Result<Vec<NearHistoryEntry>, String> {
        let url = format!(
            "{}/accounts/{}/activity?limit=50",
            indexer_base.trim_end_matches('/'),
            account_id
        );
        let items: Vec<Value> = self
            .client
            .get_json(&url, RetryProfile::ChainRead)
            .await
            .unwrap_or_default();

        Ok(items
            .into_iter()
            .map(|item| {
                let txid = item
                    .get("transaction_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let timestamp_ns: u64 = item
                    .get("block_timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let signer_id = item
                    .get("signer_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let receiver_id = item
                    .get("receiver_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let amount_yocto = item
                    .pointer("/args/deposit")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                let is_incoming = receiver_id == account_id;
                NearHistoryEntry {
                    txid,
                    timestamp_ns,
                    signer_id,
                    receiver_id,
                    amount_yocto,
                    is_incoming,
                }
            })
            .collect())
    }

    // ----------------------------------------------------------------
    // NEP-141 (fungible token) support
    // ----------------------------------------------------------------

    /// Call a view function on `contract` and return its decoded bytes.
    /// `args` is JSON that will be serialized, base64-encoded, and sent as
    /// `args_base64` per the NEAR `call_function` query type.
    pub(super) async fn view_function(
        &self,
        contract: &str,
        method: &str,
        args: &Value,
    ) -> Result<Vec<u8>, String> {
        use base64::Engine;
        let args_str = serde_json::to_string(args).map_err(|e| format!("args serialize: {e}"))?;
        let args_b64 = base64::engine::general_purpose::STANDARD.encode(args_str.as_bytes());
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "call_function",
                    "finality": "final",
                    "account_id": contract,
                    "method_name": method,
                    "args_base64": args_b64,
                }),
            )
            .await?;
        // `result.result` is a u8 array.
        let bytes = result
            .get("result")
            .and_then(|v| v.as_array())
            .ok_or("view_function: missing result bytes")?
            .iter()
            .filter_map(|n| n.as_u64().map(|n| n as u8))
            .collect::<Vec<u8>>();
        Ok(bytes)
    }

    pub async fn fetch_ft_balance_of(
        &self,
        contract: &str,
        account_id: &str,
    ) -> Result<u128, String> {
        let bytes = self
            .view_function(contract, "ft_balance_of", &json!({ "account_id": account_id }))
            .await?;
        // Response body is a JSON string like `"1000000"`.
        let s: String =
            serde_json::from_slice(&bytes).map_err(|e| format!("ft_balance_of decode: {e}"))?;
        s.parse::<u128>()
            .map_err(|e| format!("ft_balance_of parse: {e}"))
    }

    pub async fn fetch_ft_metadata(&self, contract: &str) -> Result<NearFtMetadata, String> {
        let bytes = self.view_function(contract, "ft_metadata", &json!({})).await?;
        #[derive(Deserialize)]
        struct RawMeta {
            spec: String,
            name: String,
            symbol: String,
            decimals: u8,
        }
        let meta: RawMeta =
            serde_json::from_slice(&bytes).map_err(|e| format!("ft_metadata decode: {e}"))?;
        Ok(NearFtMetadata {
            spec: meta.spec,
            name: meta.name,
            symbol: meta.symbol,
            decimals: meta.decimals,
        })
    }

    pub async fn fetch_ft_balance(
        &self,
        contract: &str,
        holder: &str,
    ) -> Result<NearFtBalance, String> {
        let raw = self.fetch_ft_balance_of(contract, holder).await?;
        let meta = self.fetch_ft_metadata(contract).await?;
        Ok(NearFtBalance {
            contract: contract.to_string(),
            holder: holder.to_string(),
            balance_raw: raw.to_string(),
            balance_display: format_ft_amount(raw, meta.decimals),
            decimals: meta.decimals,
            symbol: meta.symbol,
        })
    }
}

// ----------------------------------------------------------------
// Formatting helpers (used by balance + FT balance)
// ----------------------------------------------------------------

fn format_near(yocto: &str) -> String {
    // yocto is a 25-digit decimal; divide by 10^24 for NEAR.
    let n: u128 = yocto.parse().unwrap_or(0);
    let divisor: u128 = 1_000_000_000_000_000_000_000_000; // 10^24
    let whole = n / divisor;
    let frac = n % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:024}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

/// Format a fungible-token raw amount using its `decimals`, up to 6
/// fractional digits of display precision.
fn format_ft_amount(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let divisor: u128 = 10u128.pow(decimals as u32);
    let whole = raw / divisor;
    let frac = raw % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0>width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}

// ---------------------------------------------------------------
// History response parser (Swift calls this directly via UniFFI to
// decode raw NEAR indexer JSON into typed snapshots).
// ---------------------------------------------------------------

fn history_rows(value: &Value) -> Vec<Value> {
    if let Some(arr) = value.as_array() {
        return arr.clone();
    }
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    for key in ["txns", "transactions", "data", "result"] {
        if let Some(Value::Array(arr)) = obj.get(key) {
            return arr.clone();
        }
    }
    Vec::new()
}

fn history_string_value(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if let Some(s) = v.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            if let Some(n) = v.as_i64() {
                return Some(n.to_string());
            }
            if let Some(n) = v.as_u64() {
                return Some(n.to_string());
            }
            if let Some(n) = v.as_f64() {
                return Some(n.to_string());
            }
        }
    }
    None
}

fn history_deposit_text(row: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(s) = history_string_value(row, &["deposit", "amount"]) {
        if !s.is_empty() {
            return Some(s);
        }
    }
    if let Some(Value::Object(agg)) = row.get("actions_agg") {
        if let Some(s) = history_string_value(agg, &["deposit", "total_deposit", "amount"]) {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    if let Some(Value::Array(actions)) = row.get("actions") {
        for action in actions {
            if let Some(action_obj) = action.as_object() {
                if let Some(s) = history_string_value(action_obj, &["deposit", "amount"]) {
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
                if let Some(Value::Object(args)) = action_obj.get("args") {
                    if let Some(s) = history_string_value(args, &["deposit", "amount"]) {
                        if !s.is_empty() {
                            return Some(s);
                        }
                    }
                }
            }
        }
    }
    None
}

fn history_numeric_timestamp(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if let Some(n) = v.as_f64() {
                return Some(n);
            }
            if let Some(s) = v.as_str() {
                if let Ok(n) = s.parse::<f64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

fn history_timestamp_seconds(row: &serde_json::Map<String, Value>) -> Option<f64> {
    let pick = |t: f64| -> Option<f64> {
        if t <= 0.0 {
            return None;
        }
        if t >= 1_000_000_000_000_000.0 {
            return Some(t / 1_000_000_000.0);
        }
        if t >= 1_000_000_000_000.0 {
            return Some(t / 1_000.0);
        }
        Some(t)
    };
    if let Some(t) = history_numeric_timestamp(
        row,
        &["block_timestamp", "timestamp", "included_in_block_timestamp"],
    ) {
        if let Some(s) = pick(t) {
            return Some(s);
        }
    }
    for nested_key in ["block", "receipt_block", "included_in_block", "receipt"] {
        if let Some(Value::Object(nested)) = row.get(nested_key) {
            if let Some(t) = history_numeric_timestamp(nested, &["block_timestamp", "timestamp"]) {
                if let Some(s) = pick(t) {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn yocto_to_near(yocto: &str) -> f64 {
    // NEAR has 24 decimals. Use string manipulation to avoid u128 overflow for
    // pathological inputs.
    let s = yocto.trim();
    if s.is_empty() {
        return 0.0;
    }
    if let Ok(v) = s.parse::<u128>() {
        return v as f64 / 1e24;
    }
    if s.len() <= 24 {
        let padded = format!("{:0>24}", s);
        let frac = format!("0.{}", padded);
        return frac.parse::<f64>().unwrap_or(0.0);
    }
    let (int_part, frac_part) = s.split_at(s.len() - 24);
    format!("{}.{}", int_part, frac_part).parse::<f64>().unwrap_or(0.0)
}

#[uniffi::export]
pub fn near_parse_history_response(
    json: String,
    owner_address: String,
) -> Vec<NearHistoryParsedSnapshot> {
    let Ok(root): Result<Value, _> = serde_json::from_str(&json) else {
        return Vec::new();
    };
    let owner = owner_address.trim().to_lowercase();
    history_rows(&root)
        .into_iter()
        .filter_map(|row| {
            let row_obj = row.as_object()?.clone();
            let hash =
                history_string_value(&row_obj, &["transaction_hash", "hash", "receipt_id"])?;
            if hash.is_empty() {
                return None;
            }
            let signer = history_string_value(
                &row_obj,
                &["signer_account_id", "predecessor_account_id", "signer_id", "signer"],
            )
            .unwrap_or_default()
            .trim()
            .to_lowercase();
            let receiver = history_string_value(
                &row_obj,
                &["receiver_account_id", "receiver_id", "receiver"],
            )
            .unwrap_or_default()
            .trim()
            .to_lowercase();
            let (kind, counterparty) = if signer == owner {
                ("send".to_string(), receiver)
            } else if receiver == owner {
                ("receive".to_string(), signer)
            } else if !signer.is_empty() {
                ("receive".to_string(), signer)
            } else {
                ("send".to_string(), receiver)
            };
            let yocto = history_deposit_text(&row_obj).unwrap_or_else(|| "0".to_string());
            let amount = yocto_to_near(&yocto);
            let created = history_timestamp_seconds(&row_obj).unwrap_or(0.0);
            Some(NearHistoryParsedSnapshot {
                transaction_hash: hash,
                kind,
                amount_near: amount,
                counterparty_address: counterparty,
                created_at_unix_seconds: created,
            })
        })
        .collect()
}

#[cfg(test)]
mod near_history_tests {
    use super::*;

    #[test]
    fn near_history_send_and_receive() {
        let json = r#"{"txns":[
            {"transaction_hash":"a","signer_id":"alice.near","receiver_id":"bob.near","deposit":"1000000000000000000000000","block_timestamp":"1700000000000000000"},
            {"transaction_hash":"b","signer_id":"bob.near","receiver_id":"alice.near","actions_agg":{"deposit":"2000000000000000000000000"},"block_timestamp":"1700000001000000000"}
        ]}"#;
        let out = near_parse_history_response(json.into(), "alice.near".into());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, "send");
        assert!((out[0].amount_near - 1.0).abs() < 1e-9);
        assert_eq!(out[0].counterparty_address, "bob.near");
        assert_eq!(out[1].kind, "receive");
        assert!((out[1].amount_near - 2.0).abs() < 1e-9);
    }

    #[test]
    fn near_history_empty_on_garbage() {
        assert!(near_parse_history_response("not json".into(), "alice".into()).is_empty());
        assert!(near_parse_history_response("{}".into(), "alice".into()).is_empty());
    }
}
