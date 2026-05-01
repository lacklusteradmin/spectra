// Rust-owned mirrors of Swift Persisted* types from PersistenceModels.swift.
//
// Every struct's JSON representation MUST round-trip with the matching Swift
// Codable type byte-for-byte. If you add or rename a field, update the Swift
// side and the roundtrip tests below in the same change.

use serde::{Deserialize, Serialize};

use crate::store::wallet_domain::{CorePriceAlertCondition, CoreTransactionKind, CoreTransactionStatus};

/// Matches Swift `PersistedPriceAlertRule`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CorePersistedPriceAlertRule {
    /// UUID encoded as uppercase string (Swift default).
    pub id: String,
    pub holding_key: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub target_price: f64,
    /// One of `"Above"` / `"Below"` (see Swift `PriceAlertCondition`).
    pub condition: CorePriceAlertCondition,
    pub is_enabled: bool,
    pub has_triggered: bool,
}

/// Matches Swift `PersistedPriceAlertStore`. Current version constant: 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CorePersistedPriceAlertStore {
    pub version: i32,
    pub alerts: Vec<CorePersistedPriceAlertRule>,
}

impl CorePersistedPriceAlertStore {
    pub const CURRENT_VERSION: i32 = 1;
}

/// Matches Swift `PersistedAddressBookEntry`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CorePersistedAddressBookEntry {
    /// UUID encoded as uppercase string (Swift default).
    pub id: String,
    pub name: String,
    pub chain_name: String,
    pub address: String,
    pub note: String,
}

/// Matches Swift `PersistedAddressBookStore`. Current version constant: 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CorePersistedAddressBookStore {
    pub version: i32,
    pub entries: Vec<CorePersistedAddressBookEntry>,
}

impl CorePersistedAddressBookStore {
    pub const CURRENT_VERSION: i32 = 1;
}

/// SQLite-storage form of a transaction record. Matches Swift
/// `PersistedTransactionRecord`.
///
/// Sibling type: [`crate::fetch::transactions::CoreTransactionRecord`] is the
/// wire/merge form. Their fields look nearly identical but the two are
/// deliberately separate — see that type's doc comment for the timestamp
/// and enum-typing differences.
///
/// `created_at` is a Double (seconds since the Swift reference date,
/// 2001-01-01T00:00:00Z) — emitted by a vanilla `JSONEncoder()` which is what
/// `persistCodableToSQLite` uses on the Swift side.
///
/// All optional fields are emitted when present and omitted (not encoded as
/// `null`) when absent — Swift `decodeIfPresent` accepts both, and `serde`
/// with `skip_serializing_if` preserves the "omit when none" shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CorePersistedTransactionRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_id: Option<String>,
    /// Swift `TransactionKind`: `"send"` or `"receive"`.
    pub kind: CoreTransactionKind,
    /// Swift `TransactionStatus`: `"pending"` / `"confirmed"` / `"failed"`.
    /// Legacy records without a status default to `"pending"` for receives and
    /// `"confirmed"` for sends — replicate that fallback at the read site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<CoreTransactionStatus>,
    pub wallet_name: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub amount: f64,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ethereum_nonce: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_block_number: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_gas_used: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_effective_gas_price_gwei: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_network_fee_eth: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_priority_raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_rate_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dogecoin_confirmed_network_fee_doge: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dogecoin_confirmations: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dogecoin_fee_priority_raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dogecoin_estimated_fee_rate_doge_per_kb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_change_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dogecoin_used_change_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_derivation_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_derivation_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dogecoin_raw_transaction_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_transaction_payload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_transaction_payload_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_history_source: Option<String>,
    /// Seconds since Swift reference date (2001-01-01T00:00:00Z).
    pub created_at: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_alert_store_roundtrip() {
        // Sample payload in the exact byte shape Swift would emit.
        let json = r#"{"version":1,"alerts":[{"id":"3F2504E0-4F89-11D3-9A0C-0305E82C3301","holdingKey":"ethereum:ETH","assetName":"Ethereum","symbol":"ETH","chainName":"Ethereum","targetPrice":3500.0,"condition":"Above","isEnabled":true,"hasTriggered":false}]}"#;
        let decoded: CorePersistedPriceAlertStore = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.version, CorePersistedPriceAlertStore::CURRENT_VERSION);
        assert_eq!(decoded.alerts.len(), 1);
        assert_eq!(decoded.alerts[0].condition, CorePriceAlertCondition::Above);
        let reencoded = serde_json::to_string(&decoded).unwrap();
        assert_eq!(reencoded, json);
    }

    #[test]
    fn address_book_store_roundtrip() {
        let json = r#"{"version":1,"entries":[{"id":"550E8400-E29B-41D4-A716-446655440000","name":"Cold Wallet","chainName":"Bitcoin","address":"bc1qexample","note":"primary"}]}"#;
        let decoded: CorePersistedAddressBookStore = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.version, CorePersistedAddressBookStore::CURRENT_VERSION);
        assert_eq!(decoded.entries[0].name, "Cold Wallet");
        let reencoded = serde_json::to_string(&decoded).unwrap();
        assert_eq!(reencoded, json);
    }

    #[test]
    fn transaction_record_roundtrip_omits_none_fields() {
        // Minimal encoded shape for a received record — mirrors what Swift's
        // vanilla JSONEncoder produces (no null fields, createdAt as seconds
        // since 2001-01-01 UTC).
        let json = r#"{"id":"A1B2C3D4-E5F6-7890-ABCD-EF1234567890","kind":"receive","walletName":"Main","assetName":"Bitcoin","symbol":"BTC","chainName":"Bitcoin","amount":0.5,"address":"bc1qreceive","createdAt":745200000.0}"#;
        let decoded: CorePersistedTransactionRecord = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.kind, CoreTransactionKind::Receive);
        assert!(decoded.status.is_none());
        assert_eq!(decoded.created_at, 745200000.0);
        let reencoded = serde_json::to_string(&decoded).unwrap();
        assert_eq!(reencoded, json);
    }

    /// Minimal record for tests: an unconfirmed receive with no receipt or
    /// chain-specific extras. Tests start from this and mutate the specific
    /// fields they exercise so the assertion focus is on what changed,
    /// not a wall of `None`s.
    fn minimal_record() -> CorePersistedTransactionRecord {
        CorePersistedTransactionRecord {
            id: "11111111-2222-3333-4444-555555555555".to_string(),
            wallet_id: None,
            kind: CoreTransactionKind::Receive,
            status: None,
            wallet_name: "Main".to_string(),
            asset_name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            chain_name: "Bitcoin".to_string(),
            amount: 0.0,
            address: "".to_string(),
            transaction_hash: None,
            ethereum_nonce: None,
            receipt_block_number: None,
            receipt_gas_used: None,
            receipt_effective_gas_price_gwei: None,
            receipt_network_fee_eth: None,
            fee_priority_raw: None,
            fee_rate_description: None,
            confirmation_count: None,
            dogecoin_confirmed_network_fee_doge: None,
            dogecoin_confirmations: None,
            dogecoin_fee_priority_raw: None,
            dogecoin_estimated_fee_rate_doge_per_kb: None,
            used_change_output: None,
            dogecoin_used_change_output: None,
            source_derivation_path: None,
            change_derivation_path: None,
            source_address: None,
            change_address: None,
            dogecoin_raw_transaction_hex: None,
            signed_transaction_payload: None,
            signed_transaction_payload_format: None,
            failure_reason: None,
            transaction_history_source: None,
            created_at: 0.0,
        }
    }

    #[test]
    fn transaction_record_roundtrip_with_receipt_fields() {
        let original = CorePersistedTransactionRecord {
            wallet_id: Some("wallet-1".to_string()),
            kind: CoreTransactionKind::Send,
            status: Some(CoreTransactionStatus::Confirmed),
            asset_name: "Ethereum".to_string(),
            symbol: "ETH".to_string(),
            chain_name: "Ethereum".to_string(),
            amount: 1.25,
            address: "0xrecipient".to_string(),
            transaction_hash: Some("0xhash".to_string()),
            ethereum_nonce: Some(7),
            receipt_block_number: Some(20_000_000),
            receipt_gas_used: Some("21000".to_string()),
            receipt_effective_gas_price_gwei: Some(25.5),
            receipt_network_fee_eth: Some(0.000535),
            fee_priority_raw: Some("standard".to_string()),
            confirmation_count: Some(12),
            used_change_output: Some(true),
            transaction_history_source: Some("rpc".to_string()),
            created_at: 750000000.5,
            ..minimal_record()
        };
        let json = serde_json::to_string(&original).unwrap();
        // None-valued optional fields must be omitted, not serialized as null.
        assert!(!json.contains("null"), "unexpected null in {json}");
        assert!(!json.contains("feeRateDescription"));
        let decoded: CorePersistedTransactionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }
}
