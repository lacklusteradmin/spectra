// NEAR history response parser (previously NearBalanceService.parseHistoryResponse).
// The old `balance_decoder_*_field` JSON peekers that used to live here were
// removed after `fetch_balance` became Rust-internal — the Swift layer now
// receives typed balance records and no longer needs generic JSON field
// extraction helpers.

use serde_json::Value;

// ---------------------------------------------------------------
// NEAR history response parser
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
