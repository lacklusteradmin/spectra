// Lightweight JSON balance decoders used by the Swift layer when it
// receives backend JSON payloads it wants to interpret without owning a
// decoder. Mirrors the old Swift `RustBalanceDecoder` enum and the
// NEAR history-response parser that previously lived in ChainTypes.swift.

use serde_json::Value;

fn parse_object(json: &str) -> Option<serde_json::Map<String, Value>> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.as_object().cloned()
}

fn field_u64(obj: &serde_json::Map<String, Value>, field: &str) -> Option<u64> {
    let v = obj.get(field)?;
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        return s.parse::<u64>().ok();
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 0.0 {
            return Some(f as u64);
        }
    }
    None
}

fn field_i64(obj: &serde_json::Map<String, Value>, field: &str) -> Option<i64> {
    let v = obj.get(field)?;
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        return s.parse::<i64>().ok();
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() {
            return Some(f as i64);
        }
    }
    None
}

fn field_f64(obj: &serde_json::Map<String, Value>, field: &str) -> Option<f64> {
    let v = obj.get(field)?;
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    if let Some(s) = v.as_str() {
        return s.parse::<f64>().ok();
    }
    None
}

#[uniffi::export]
pub fn balance_decoder_u64_field(field: String, json: String) -> Option<u64> {
    let obj = parse_object(&json)?;
    field_u64(&obj, &field)
}

#[uniffi::export]
pub fn balance_decoder_i64_field(field: String, json: String) -> Option<i64> {
    let obj = parse_object(&json)?;
    field_i64(&obj, &field)
}

#[uniffi::export]
pub fn balance_decoder_u128_string_field_as_f64(field: String, json: String) -> Option<f64> {
    let obj = parse_object(&json)?;
    field_f64(&obj, &field)
}

#[uniffi::export]
pub fn balance_decoder_f64_field(field: String, json: String) -> Option<f64> {
    let obj = parse_object(&json)?;
    field_f64(&obj, &field)
}

#[uniffi::export]
pub fn balance_decoder_string_field(field: String, json: String) -> Option<String> {
    let obj = parse_object(&json)?;
    obj.get(&field)?.as_str().map(|s| s.to_string())
}

#[uniffi::export]
pub fn balance_decoder_has_field(field: String, json: String) -> bool {
    parse_object(&json).map_or(false, |obj| obj.contains_key(&field))
}

#[uniffi::export]
pub fn balance_decoder_first_element_string_field(field: String, json: String) -> Option<String> {
    let v: Value = serde_json::from_str(&json).ok()?;
    let arr = v.as_array()?;
    let obj = arr.first()?.as_object()?;
    obj.get(&field)?.as_str().map(|s| s.to_string())
}

#[uniffi::export]
pub fn balance_decoder_json_array_is_non_empty(json: String) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(&json) else { return false };
    v.as_array().map_or(false, |a| !a.is_empty())
}

#[uniffi::export]
pub fn balance_decoder_evm_native_balance(json: String) -> Option<f64> {
    let obj = parse_object(&json)?;
    if let Some(display) = obj.get("balance_display").and_then(|v| v.as_str()) {
        if let Ok(v) = display.parse::<f64>() {
            return Some(v);
        }
    }
    let wei = field_f64(&obj, "balance_wei")?;
    Some(wei / 1e18)
}

#[uniffi::export]
pub fn balance_decoder_yocto_near_to_double(json: String) -> Option<f64> {
    let obj = parse_object(&json)?;
    if let Some(display) = obj.get("near_display").and_then(|v| v.as_str()) {
        if let Ok(v) = display.parse::<f64>() {
            return Some(v);
        }
    }
    let yocto = field_f64(&obj, "yocto_near")?;
    Some(yocto / 1e24)
}

// ---------------------------------------------------------------
// NEAR history response parser (previously NearBalanceService.parseHistoryResponse)
// ---------------------------------------------------------------

#[derive(Debug, Clone, uniffi::Record)]
pub struct NearHistoryParsedSnapshot {
    pub transaction_hash: String,
    /// "send" or "receive"
    pub kind: String,
    pub amount_near: f64,
    pub counterparty_address: String,
    /// Unix seconds (0 = fall back to "now" on the Swift side).
    pub created_at_unix_seconds: f64,
}

fn history_rows(value: &Value) -> Vec<Value> {
    if let Some(arr) = value.as_array() {
        return arr.clone();
    }
    let Some(obj) = value.as_object() else { return Vec::new(); };
    for key in ["txns", "transactions", "data", "result"] {
        if let Some(Value::Array(arr)) = obj.get(key) {
            return arr.clone();
        }
    }
    Vec::new()
}

fn string_value(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if let Some(s) = v.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            if let Some(n) = v.as_i64() { return Some(n.to_string()); }
            if let Some(n) = v.as_u64() { return Some(n.to_string()); }
            if let Some(n) = v.as_f64() { return Some(n.to_string()); }
        }
    }
    None
}

fn deposit_text(row: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(s) = string_value(row, &["deposit", "amount"]) {
        if !s.is_empty() { return Some(s); }
    }
    if let Some(Value::Object(agg)) = row.get("actions_agg") {
        if let Some(s) = string_value(agg, &["deposit", "total_deposit", "amount"]) {
            if !s.is_empty() { return Some(s); }
        }
    }
    if let Some(Value::Array(actions)) = row.get("actions") {
        for action in actions {
            if let Some(action_obj) = action.as_object() {
                if let Some(s) = string_value(action_obj, &["deposit", "amount"]) {
                    if !s.is_empty() { return Some(s); }
                }
                if let Some(Value::Object(args)) = action_obj.get("args") {
                    if let Some(s) = string_value(args, &["deposit", "amount"]) {
                        if !s.is_empty() { return Some(s); }
                    }
                }
            }
        }
    }
    None
}

fn numeric_timestamp(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if let Some(n) = v.as_f64() { return Some(n); }
            if let Some(s) = v.as_str() { if let Ok(n) = s.parse::<f64>() { return Some(n); } }
        }
    }
    None
}

fn timestamp_seconds(row: &serde_json::Map<String, Value>) -> Option<f64> {
    let pick = |t: f64| -> Option<f64> {
        if t <= 0.0 { return None; }
        if t >= 1_000_000_000_000_000.0 { return Some(t / 1_000_000_000.0); }
        if t >= 1_000_000_000_000.0 { return Some(t / 1_000.0); }
        Some(t)
    };
    if let Some(t) = numeric_timestamp(row, &["block_timestamp", "timestamp", "included_in_block_timestamp"]) {
        if let Some(s) = pick(t) { return Some(s); }
    }
    for nested_key in ["block", "receipt_block", "included_in_block", "receipt"] {
        if let Some(Value::Object(nested)) = row.get(nested_key) {
            if let Some(t) = numeric_timestamp(nested, &["block_timestamp", "timestamp"]) {
                if let Some(s) = pick(t) { return Some(s); }
            }
        }
    }
    None
}

fn yocto_to_near(yocto: &str) -> f64 {
    // Parse big-int decimal; NEAR has 24 decimals. Use string manipulation
    // to avoid u128 overflow for pathological inputs.
    let s = yocto.trim();
    if s.is_empty() { return 0.0; }
    // Fast path: parse as u128.
    if let Ok(v) = s.parse::<u128>() {
        return v as f64 / 1e24;
    }
    // Fallback: string divide — drop last 24 digits.
    if s.len() <= 24 {
        // fractional
        let padded = format!("{:0>24}", s);
        let frac = format!("0.{}", padded);
        return frac.parse::<f64>().unwrap_or(0.0);
    }
    let (int_part, frac_part) = s.split_at(s.len() - 24);
    format!("{}.{}", int_part, frac_part).parse::<f64>().unwrap_or(0.0)
}

#[uniffi::export]
pub fn near_parse_history_response(json: String, owner_address: String) -> Vec<NearHistoryParsedSnapshot> {
    let Ok(root): Result<Value, _> = serde_json::from_str(&json) else { return Vec::new(); };
    let owner = owner_address.trim().to_lowercase();
    history_rows(&root)
        .into_iter()
        .filter_map(|row| {
            let row_obj = row.as_object()?.clone();
            let hash = string_value(&row_obj, &["transaction_hash", "hash", "receipt_id"])?;
            if hash.is_empty() { return None; }
            let signer = string_value(&row_obj, &["signer_account_id", "predecessor_account_id", "signer_id", "signer"])
                .unwrap_or_default().trim().to_lowercase();
            let receiver = string_value(&row_obj, &["receiver_account_id", "receiver_id", "receiver"])
                .unwrap_or_default().trim().to_lowercase();
            let (kind, counterparty) = if signer == owner {
                ("send".to_string(), receiver)
            } else if receiver == owner {
                ("receive".to_string(), signer)
            } else if !signer.is_empty() {
                ("receive".to_string(), signer)
            } else {
                ("send".to_string(), receiver)
            };
            let yocto = deposit_text(&row_obj).unwrap_or_else(|| "0".to_string());
            let amount = yocto_to_near(&yocto);
            let created = timestamp_seconds(&row_obj).unwrap_or(0.0);
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
mod tests {
    use super::*;

    #[test]
    fn u64_field_from_number_and_string() {
        assert_eq!(balance_decoder_u64_field("n".into(), r#"{"n":42}"#.into()), Some(42));
        assert_eq!(balance_decoder_u64_field("n".into(), r#"{"n":"99"}"#.into()), Some(99));
        assert_eq!(balance_decoder_u64_field("x".into(), r#"{"n":"99"}"#.into()), None);
    }

    #[test]
    fn evm_native_balance_prefers_display() {
        let j = r#"{"balance_display":"1.5","balance_wei":"123"}"#;
        assert_eq!(balance_decoder_evm_native_balance(j.into()), Some(1.5));
    }

    #[test]
    fn evm_native_balance_falls_back_to_wei() {
        let j = r#"{"balance_wei":"1000000000000000000"}"#;
        assert_eq!(balance_decoder_evm_native_balance(j.into()), Some(1.0));
    }

    #[test]
    fn yocto_near_fallback() {
        let j = r#"{"yocto_near":"1000000000000000000000000"}"#;
        assert_eq!(balance_decoder_yocto_near_to_double(j.into()), Some(1.0));
    }

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
