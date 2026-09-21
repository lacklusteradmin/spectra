// Core-owned transaction payload stored in SQLite.

use serde::{Deserialize, Serialize};

use crate::store::wallet_domain::{CoreTransactionKind, CoreTransactionStatus};

/// Seconds between Unix time and the transaction payload epoch (2001-01-01).
pub(crate) const SWIFT_REFERENCE_EPOCH_OFFSET_SECS: f64 = 978_307_200.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CorePersistedTransactionRecord {
    /// Known for local sends; provider history may omit protocol identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_id: Option<String>,
    /// Swift `TransactionKind`: `"send"` or `"receive"`.
    pub kind: CoreTransactionKind,
    /// Whether the transaction is pending, confirmed or failed.
    ///
    /// Not optional: a record with no status used to mean "legacy row, decide
    /// by kind at the read site", and every read site that forgot got a
    /// different answer — the app read it one way and core another.
    pub status: CoreTransactionStatus,
    pub wallet_name: String,
    pub asset_display_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub amount: f64,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_block_number: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_gas_used: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_effective_gas_price_gwei: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_network_fee: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_priority_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_rate_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed_network_fee: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_fee_rate_per_kb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_change_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_derivation_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_derivation_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_transaction_payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_transaction_payload_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_history_source: Option<String>,
    /// Seconds since Swift reference date (2001-01-01T00:00:00Z).
    pub created_at: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_record_roundtrip_omits_none_fields() {
        // Minimal encoded shape for a received record: no null fields, and
        // createdAt as seconds since 2001-01-01 UTC. `status` is one of the
        // required fields — it was optional, and absence meant "decide by
        // kind at the read site", which the app and core decided differently.
        let json = r#"{"id":"A1B2C3D4-E5F6-7890-ABCD-EF1234567890","kind":"receive","status":"pending","walletName":"Main","assetDisplayName":"Bitcoin","symbol":"BTC","chainName":"Bitcoin","amount":0.5,"address":"bc1qreceive","createdAt":745200000.0}"#;
        let decoded: CorePersistedTransactionRecord = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.kind, CoreTransactionKind::Receive);
        assert_eq!(decoded.status, CoreTransactionStatus::Pending);
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
            deployment_id: None,
            id: "11111111-2222-3333-4444-555555555555".to_string(),
            wallet_id: None,
            kind: CoreTransactionKind::Receive,
            status: CoreTransactionStatus::Pending,
            wallet_name: "Main".to_string(),
            asset_display_name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            chain_name: "Bitcoin".to_string(),
            amount: 0.0,
            address: "".to_string(),
            transaction_hash: None,
            nonce: None,
            receipt_block_number: None,
            receipt_gas_used: None,
            receipt_effective_gas_price_gwei: None,
            receipt_network_fee: None,
            fee_priority_raw: None,
            fee_rate_description: None,
            confirmation_count: None,
            confirmed_network_fee: None,
            estimated_fee_rate_per_kb: None,
            used_change_output: None,
            source_derivation_path: None,
            change_derivation_path: None,
            source_address: None,
            change_address: None,
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
            status: CoreTransactionStatus::Confirmed,
            asset_display_name: "Ethereum".to_string(),
            symbol: "ETH".to_string(),
            chain_name: "Ethereum".to_string(),
            amount: 1.25,
            address: "0xrecipient".to_string(),
            transaction_hash: Some("0xhash".to_string()),
            nonce: Some(7),
            receipt_block_number: Some(20_000_000),
            receipt_gas_used: Some("21000".to_string()),
            receipt_effective_gas_price_gwei: Some(25.5),
            receipt_network_fee: Some(0.000535),
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
