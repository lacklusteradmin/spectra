use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Wire/merge form of a transaction record.
///
/// Distinct from [`crate::store::persistence_models::CorePersistedTransactionRecord`]
/// in two specific ways — they look almost identical but the differences are
/// load-bearing:
///   1. **Timestamps**: this type uses unix-epoch seconds (`created_at_unix`)
///      because merge ordering compares against incoming RPC payloads that
///      carry unix timestamps. The persisted form uses *Swift reference time*
///      (seconds since 2001-01-01) for backward-compat with already-stored
///      Foundation `Date` values.
///   2. **Enum typing**: `kind` and `status` are plain strings so inbound
///      records that carry unfamiliar values don't fail to deserialize before
///      the merge logic can decide what to do with them. The persisted form
///      uses strongly-typed `CoreTransactionKind` / `CoreTransactionStatus`
///      enums so storage rejects malformed data at write time.
///
/// Conversion between the two forms happens at the Swift boundary
/// (`TransactionRecord.rustBridgeRecord` / `.persistedSnapshot`), where the
/// time-base shift and type tightening can both be performed in one place.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreTransactionRecord {
    pub id: String,
    pub wallet_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub wallet_name: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub amount: f64,
    pub address: String,
    pub transaction_hash: Option<String>,
    pub ethereum_nonce: Option<i64>,
    pub receipt_block_number: Option<i64>,
    pub receipt_gas_used: Option<String>,
    pub receipt_effective_gas_price_gwei: Option<f64>,
    pub receipt_network_fee_eth: Option<f64>,
    pub fee_priority_raw: Option<String>,
    pub fee_rate_description: Option<String>,
    pub confirmation_count: Option<i64>,
    pub dogecoin_confirmed_network_fee_doge: Option<f64>,
    pub dogecoin_estimated_fee_rate_doge_per_kb: Option<f64>,
    pub used_change_output: Option<bool>,
    pub source_derivation_path: Option<String>,
    pub change_derivation_path: Option<String>,
    pub source_address: Option<String>,
    pub change_address: Option<String>,
    pub signed_transaction_payload: Option<String>,
    pub signed_transaction_payload_format: Option<String>,
    pub failure_reason: Option<String>,
    pub transaction_history_source: Option<String>,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum TransactionMergeStrategy {
    StandardUtxo,
    Dogecoin,
    AccountBased,
    Evm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionMergeRequest {
    pub existing_transactions: Vec<CoreTransactionRecord>,
    pub incoming_transactions: Vec<CoreTransactionRecord>,
    pub strategy: TransactionMergeStrategy,
    pub chain_name: String,
    #[serde(default)]
    pub include_symbol_in_identity: bool,
    pub preserve_created_at_sentinel_unix: Option<f64>,
}

// ── Wire ↔ persisted conversion ──────────────────────────────────────────────
//
// The two shapes hold the same 30 fields and differ in three ways: `kind` and
// `status` are strings on the wire and enums in storage, and the timestamp is
// Unix on the wire and Swift reference time in storage. Both platforms used to
// own this conversion; it belongs here, next to the merge that needs it.

use crate::store::persistence_models::CorePersistedTransactionRecord;
use crate::store::wallet_domain::{CoreTransactionKind, CoreTransactionStatus};

/// Seconds between the Unix epoch and Swift's reference date (2001-01-01 UTC).
const SWIFT_REFERENCE_EPOCH_OFFSET_SECS: f64 = 978_307_200.0;

fn kind_from_raw(raw: &str) -> CoreTransactionKind {
    match raw {
        "send" => CoreTransactionKind::Send,
        _ => CoreTransactionKind::Receive,
    }
}

fn kind_to_raw(kind: CoreTransactionKind) -> &'static str {
    match kind {
        CoreTransactionKind::Send => "send",
        CoreTransactionKind::Receive => "receive",
    }
}

fn status_from_raw(raw: &str) -> Option<CoreTransactionStatus> {
    match raw {
        "pending" => Some(CoreTransactionStatus::Pending),
        "confirmed" => Some(CoreTransactionStatus::Confirmed),
        "failed" => Some(CoreTransactionStatus::Failed),
        _ => None,
    }
}

/// A stored record with no status is legacy data: receives were pending and
/// sends were confirmed. Applying that here means no read site has to remember.
fn status_to_raw(status: Option<CoreTransactionStatus>, kind: CoreTransactionKind) -> String {
    match status {
        Some(CoreTransactionStatus::Pending) => "pending",
        Some(CoreTransactionStatus::Confirmed) => "confirmed",
        Some(CoreTransactionStatus::Failed) => "failed",
        None => match kind {
            CoreTransactionKind::Send => "confirmed",
            CoreTransactionKind::Receive => "pending",
        },
    }
    .to_string()
}

impl From<CorePersistedTransactionRecord> for CoreTransactionRecord {
    fn from(stored: CorePersistedTransactionRecord) -> Self {
        Self {
            id: stored.id,
            wallet_id: stored.wallet_id,
            kind: kind_to_raw(stored.kind).to_string(),
            status: status_to_raw(stored.status, stored.kind),
            wallet_name: stored.wallet_name,
            asset_name: stored.asset_name,
            symbol: stored.symbol,
            chain_name: stored.chain_name,
            amount: stored.amount,
            address: stored.address,
            transaction_hash: stored.transaction_hash,
            ethereum_nonce: stored.ethereum_nonce,
            receipt_block_number: stored.receipt_block_number,
            receipt_gas_used: stored.receipt_gas_used,
            receipt_effective_gas_price_gwei: stored.receipt_effective_gas_price_gwei,
            receipt_network_fee_eth: stored.receipt_network_fee_eth,
            fee_priority_raw: stored.fee_priority_raw,
            fee_rate_description: stored.fee_rate_description,
            confirmation_count: stored.confirmation_count,
            dogecoin_confirmed_network_fee_doge: stored.dogecoin_confirmed_network_fee_doge,
            dogecoin_estimated_fee_rate_doge_per_kb: stored.dogecoin_estimated_fee_rate_doge_per_kb,
            used_change_output: stored.used_change_output,
            source_derivation_path: stored.source_derivation_path,
            change_derivation_path: stored.change_derivation_path,
            source_address: stored.source_address,
            change_address: stored.change_address,
            signed_transaction_payload: stored.signed_transaction_payload,
            signed_transaction_payload_format: stored.signed_transaction_payload_format,
            failure_reason: stored.failure_reason,
            transaction_history_source: stored.transaction_history_source,
            created_at_unix: stored.created_at + SWIFT_REFERENCE_EPOCH_OFFSET_SECS,
        }
    }
}

impl From<CoreTransactionRecord> for CorePersistedTransactionRecord {
    fn from(wire: CoreTransactionRecord) -> Self {
        Self {
            id: wire.id,
            wallet_id: wire.wallet_id,
            kind: kind_from_raw(&wire.kind),
            status: status_from_raw(&wire.status),
            wallet_name: wire.wallet_name,
            asset_name: wire.asset_name,
            symbol: wire.symbol,
            chain_name: wire.chain_name,
            amount: wire.amount,
            address: wire.address,
            transaction_hash: wire.transaction_hash,
            ethereum_nonce: wire.ethereum_nonce,
            receipt_block_number: wire.receipt_block_number,
            receipt_gas_used: wire.receipt_gas_used,
            receipt_effective_gas_price_gwei: wire.receipt_effective_gas_price_gwei,
            receipt_network_fee_eth: wire.receipt_network_fee_eth,
            fee_priority_raw: wire.fee_priority_raw,
            fee_rate_description: wire.fee_rate_description,
            confirmation_count: wire.confirmation_count,
            dogecoin_confirmed_network_fee_doge: wire.dogecoin_confirmed_network_fee_doge,
            dogecoin_estimated_fee_rate_doge_per_kb: wire.dogecoin_estimated_fee_rate_doge_per_kb,
            used_change_output: wire.used_change_output,
            source_derivation_path: wire.source_derivation_path,
            change_derivation_path: wire.change_derivation_path,
            source_address: wire.source_address,
            change_address: wire.change_address,
            signed_transaction_payload: wire.signed_transaction_payload,
            signed_transaction_payload_format: wire.signed_transaction_payload_format,
            failure_reason: wire.failure_reason,
            transaction_history_source: wire.transaction_history_source,
            created_at: wire.created_at_unix - SWIFT_REFERENCE_EPOCH_OFFSET_SECS,
        }
    }
}

pub fn merge_transactions(request: TransactionMergeRequest) -> Vec<CoreTransactionRecord> {
    let TransactionMergeRequest {
        existing_transactions,
        incoming_transactions,
        strategy,
        chain_name,
        include_symbol_in_identity,
        preserve_created_at_sentinel_unix,
    } = request;
    let mut merged_transactions = existing_transactions;

    for incoming in incoming_transactions {
        if !incoming_is_relevant(&incoming, &strategy, &chain_name) {
            continue;
        }

        if let Some(existing_index) = merged_transactions.iter().position(|existing| {
            matches_identity(
                existing,
                &incoming,
                &strategy,
                &chain_name,
                include_symbol_in_identity,
            )
        }) {
            let existing = merged_transactions[existing_index].clone();
            merged_transactions[existing_index] = merge_record(
                existing,
                incoming,
                &strategy,
                preserve_created_at_sentinel_unix,
            );
        } else {
            merged_transactions.push(incoming);
        }
    }

    merged_transactions.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(Ordering::Equal)
    });
    merged_transactions
}

fn incoming_is_relevant(
    incoming: &CoreTransactionRecord,
    strategy: &TransactionMergeStrategy,
    chain_name: &str,
) -> bool {
    if incoming.chain_name != chain_name || incoming.transaction_hash.is_none() {
        return false;
    }

    match strategy {
        TransactionMergeStrategy::StandardUtxo => true,
        TransactionMergeStrategy::Dogecoin
        | TransactionMergeStrategy::AccountBased
        | TransactionMergeStrategy::Evm => incoming.wallet_id.is_some(),
    }
}

fn matches_identity(
    existing: &CoreTransactionRecord,
    incoming: &CoreTransactionRecord,
    strategy: &TransactionMergeStrategy,
    chain_name: &str,
    include_symbol_in_identity: bool,
) -> bool {
    if existing.chain_name != chain_name
        || existing.transaction_hash != incoming.transaction_hash
        || existing.kind != incoming.kind
    {
        return false;
    }

    match strategy {
        TransactionMergeStrategy::StandardUtxo => existing.wallet_id == incoming.wallet_id,
        TransactionMergeStrategy::Dogecoin => {
            existing.wallet_id == incoming.wallet_id && incoming.wallet_id.is_some()
        }
        TransactionMergeStrategy::AccountBased => {
            existing.wallet_id == incoming.wallet_id
                && incoming.wallet_id.is_some()
                && (!include_symbol_in_identity || existing.symbol == incoming.symbol)
        }
        TransactionMergeStrategy::Evm => {
            existing.wallet_id == incoming.wallet_id
                && incoming.wallet_id.is_some()
                && existing.symbol == incoming.symbol
                && normalize_evm_address(&existing.address)
                    == normalize_evm_address(&incoming.address)
                && approximately_equal(existing.amount, incoming.amount)
        }
    }
}

fn merge_record(
    existing: CoreTransactionRecord,
    incoming: CoreTransactionRecord,
    strategy: &TransactionMergeStrategy,
    preserve_created_at_sentinel_unix: Option<f64>,
) -> CoreTransactionRecord {
    match strategy {
        TransactionMergeStrategy::StandardUtxo => merge_standard_utxo(existing, incoming),
        TransactionMergeStrategy::Dogecoin => merge_dogecoin(existing, incoming),
        TransactionMergeStrategy::AccountBased => {
            merge_account_based(existing, incoming, preserve_created_at_sentinel_unix)
        }
        TransactionMergeStrategy::Evm => {
            merge_evm(existing, incoming, preserve_created_at_sentinel_unix)
        }
    }
}

fn merge_standard_utxo(
    existing: CoreTransactionRecord,
    incoming: CoreTransactionRecord,
) -> CoreTransactionRecord {
    CoreTransactionRecord {
        id: existing.id,
        wallet_id: incoming.wallet_id.or(existing.wallet_id),
        kind: incoming.kind,
        status: incoming.status,
        wallet_name: incoming.wallet_name,
        asset_name: incoming.asset_name,
        symbol: incoming.symbol,
        chain_name: incoming.chain_name,
        amount: incoming.amount,
        address: incoming.address,
        transaction_hash: incoming.transaction_hash,
        ethereum_nonce: incoming.ethereum_nonce.or(existing.ethereum_nonce),
        receipt_block_number: incoming
            .receipt_block_number
            .or(existing.receipt_block_number),
        receipt_gas_used: existing.receipt_gas_used,
        receipt_effective_gas_price_gwei: existing.receipt_effective_gas_price_gwei,
        receipt_network_fee_eth: existing.receipt_network_fee_eth,
        fee_priority_raw: incoming.fee_priority_raw.or(existing.fee_priority_raw),
        fee_rate_description: incoming
            .fee_rate_description
            .or(existing.fee_rate_description),
        confirmation_count: incoming.confirmation_count.or(existing.confirmation_count),
        dogecoin_confirmed_network_fee_doge: existing.dogecoin_confirmed_network_fee_doge,
        dogecoin_estimated_fee_rate_doge_per_kb: existing.dogecoin_estimated_fee_rate_doge_per_kb,
        used_change_output: incoming.used_change_output.or(existing.used_change_output),
        source_derivation_path: existing.source_derivation_path,
        change_derivation_path: existing.change_derivation_path,
        source_address: incoming.source_address.or(existing.source_address),
        change_address: incoming.change_address.or(existing.change_address),
        signed_transaction_payload: incoming
            .signed_transaction_payload
            .or(existing.signed_transaction_payload),
        signed_transaction_payload_format: incoming
            .signed_transaction_payload_format
            .or(existing.signed_transaction_payload_format),
        failure_reason: existing.failure_reason,
        transaction_history_source: incoming
            .transaction_history_source
            .or(existing.transaction_history_source),
        created_at_unix: incoming.created_at_unix,
    }
}

fn merge_dogecoin(
    existing: CoreTransactionRecord,
    incoming: CoreTransactionRecord,
) -> CoreTransactionRecord {
    CoreTransactionRecord {
        id: existing.id,
        wallet_id: incoming.wallet_id.or(existing.wallet_id),
        kind: incoming.kind,
        status: incoming.status,
        wallet_name: incoming.wallet_name,
        asset_name: incoming.asset_name,
        symbol: incoming.symbol,
        chain_name: incoming.chain_name,
        amount: incoming.amount,
        address: incoming.address,
        transaction_hash: incoming.transaction_hash,
        ethereum_nonce: incoming.ethereum_nonce.or(existing.ethereum_nonce),
        receipt_block_number: incoming
            .receipt_block_number
            .or(existing.receipt_block_number),
        receipt_gas_used: existing.receipt_gas_used,
        receipt_effective_gas_price_gwei: existing.receipt_effective_gas_price_gwei,
        receipt_network_fee_eth: existing.receipt_network_fee_eth,
        fee_priority_raw: incoming.fee_priority_raw.or(existing.fee_priority_raw),
        fee_rate_description: incoming
            .fee_rate_description
            .or(existing.fee_rate_description),
        confirmation_count: incoming.confirmation_count.or(existing.confirmation_count),
        dogecoin_confirmed_network_fee_doge: incoming
            .dogecoin_confirmed_network_fee_doge
            .or(existing.dogecoin_confirmed_network_fee_doge),
        dogecoin_estimated_fee_rate_doge_per_kb: incoming
            .dogecoin_estimated_fee_rate_doge_per_kb
            .or(existing.dogecoin_estimated_fee_rate_doge_per_kb),
        used_change_output: incoming.used_change_output.or(existing.used_change_output),
        source_derivation_path: incoming
            .source_derivation_path
            .or(existing.source_derivation_path),
        change_derivation_path: incoming
            .change_derivation_path
            .or(existing.change_derivation_path),
        source_address: incoming.source_address.or(existing.source_address),
        change_address: incoming.change_address.or(existing.change_address),
        signed_transaction_payload: incoming
            .signed_transaction_payload
            .or(existing.signed_transaction_payload),
        signed_transaction_payload_format: incoming
            .signed_transaction_payload_format
            .or(existing.signed_transaction_payload_format),
        failure_reason: incoming.failure_reason.or(existing.failure_reason),
        transaction_history_source: incoming
            .transaction_history_source
            .or(existing.transaction_history_source),
        created_at_unix: incoming.created_at_unix,
    }
}

fn merge_account_based(
    existing: CoreTransactionRecord,
    incoming: CoreTransactionRecord,
    preserve_created_at_sentinel_unix: Option<f64>,
) -> CoreTransactionRecord {
    CoreTransactionRecord {
        id: existing.id,
        wallet_id: incoming.wallet_id.or(existing.wallet_id),
        kind: incoming.kind,
        status: incoming.status,
        wallet_name: incoming.wallet_name,
        asset_name: incoming.asset_name,
        symbol: incoming.symbol,
        chain_name: incoming.chain_name,
        amount: incoming.amount,
        address: incoming.address,
        transaction_hash: incoming.transaction_hash,
        ethereum_nonce: existing.ethereum_nonce,
        receipt_block_number: incoming
            .receipt_block_number
            .or(existing.receipt_block_number),
        receipt_gas_used: existing.receipt_gas_used,
        receipt_effective_gas_price_gwei: existing.receipt_effective_gas_price_gwei,
        receipt_network_fee_eth: existing.receipt_network_fee_eth,
        fee_priority_raw: incoming.fee_priority_raw.or(existing.fee_priority_raw),
        fee_rate_description: incoming
            .fee_rate_description
            .or(existing.fee_rate_description),
        confirmation_count: incoming.confirmation_count.or(existing.confirmation_count),
        dogecoin_confirmed_network_fee_doge: existing.dogecoin_confirmed_network_fee_doge,
        dogecoin_estimated_fee_rate_doge_per_kb: existing.dogecoin_estimated_fee_rate_doge_per_kb,
        used_change_output: incoming.used_change_output.or(existing.used_change_output),
        source_derivation_path: existing.source_derivation_path,
        change_derivation_path: existing.change_derivation_path,
        source_address: incoming.source_address.or(existing.source_address),
        change_address: incoming.change_address.or(existing.change_address),
        signed_transaction_payload: incoming
            .signed_transaction_payload
            .or(existing.signed_transaction_payload),
        signed_transaction_payload_format: incoming
            .signed_transaction_payload_format
            .or(existing.signed_transaction_payload_format),
        failure_reason: incoming.failure_reason.or(existing.failure_reason),
        transaction_history_source: incoming
            .transaction_history_source
            .or(existing.transaction_history_source),
        created_at_unix: resolve_created_at(
            existing.created_at_unix,
            incoming.created_at_unix,
            preserve_created_at_sentinel_unix,
        ),
    }
}

fn merge_evm(
    existing: CoreTransactionRecord,
    incoming: CoreTransactionRecord,
    preserve_created_at_sentinel_unix: Option<f64>,
) -> CoreTransactionRecord {
    CoreTransactionRecord {
        id: existing.id,
        wallet_id: incoming.wallet_id.or(existing.wallet_id),
        kind: incoming.kind,
        status: incoming.status,
        wallet_name: incoming.wallet_name,
        asset_name: incoming.asset_name,
        symbol: incoming.symbol,
        chain_name: incoming.chain_name,
        amount: incoming.amount,
        address: incoming.address,
        transaction_hash: incoming.transaction_hash,
        ethereum_nonce: incoming.ethereum_nonce.or(existing.ethereum_nonce),
        receipt_block_number: incoming
            .receipt_block_number
            .or(existing.receipt_block_number),
        receipt_gas_used: incoming.receipt_gas_used.or(existing.receipt_gas_used),
        receipt_effective_gas_price_gwei: incoming
            .receipt_effective_gas_price_gwei
            .or(existing.receipt_effective_gas_price_gwei),
        receipt_network_fee_eth: incoming
            .receipt_network_fee_eth
            .or(existing.receipt_network_fee_eth),
        fee_priority_raw: incoming.fee_priority_raw.or(existing.fee_priority_raw),
        fee_rate_description: incoming
            .fee_rate_description
            .or(existing.fee_rate_description),
        confirmation_count: incoming.confirmation_count.or(existing.confirmation_count),
        dogecoin_confirmed_network_fee_doge: existing.dogecoin_confirmed_network_fee_doge,
        dogecoin_estimated_fee_rate_doge_per_kb: existing.dogecoin_estimated_fee_rate_doge_per_kb,
        used_change_output: incoming.used_change_output.or(existing.used_change_output),
        source_derivation_path: existing.source_derivation_path,
        change_derivation_path: existing.change_derivation_path,
        source_address: existing.source_address,
        change_address: existing.change_address,
        signed_transaction_payload: incoming
            .signed_transaction_payload
            .or(existing.signed_transaction_payload),
        signed_transaction_payload_format: incoming
            .signed_transaction_payload_format
            .or(existing.signed_transaction_payload_format),
        failure_reason: incoming.failure_reason.or(existing.failure_reason),
        transaction_history_source: incoming
            .transaction_history_source
            .or(existing.transaction_history_source),
        created_at_unix: resolve_created_at(
            existing.created_at_unix,
            incoming.created_at_unix,
            preserve_created_at_sentinel_unix,
        ),
    }
}

fn resolve_created_at(
    existing_created_at_unix: f64,
    incoming_created_at_unix: f64,
    preserve_created_at_sentinel_unix: Option<f64>,
) -> f64 {
    if let Some(sentinel) = preserve_created_at_sentinel_unix {
        if approximately_equal(incoming_created_at_unix, sentinel) {
            return existing_created_at_unix;
        }
    }
    incoming_created_at_unix
}

fn normalize_evm_address(address: &str) -> String {
    address.trim().to_lowercase()
}

fn approximately_equal(lhs: f64, rhs: f64) -> bool {
    (lhs - rhs).abs() < 0.0000000001
}

#[cfg(test)]
mod tests {
    use super::{
        merge_transactions, CoreTransactionRecord, TransactionMergeRequest,
        TransactionMergeStrategy,
    };

    fn sample_transaction(chain_name: &str) -> CoreTransactionRecord {
        CoreTransactionRecord {
            id: "tx-1".to_string(),
            wallet_id: Some("wallet-1".to_string()),
            kind: "receive".to_string(),
            status: "pending".to_string(),
            wallet_name: "Wallet".to_string(),
            asset_name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            chain_name: chain_name.to_string(),
            amount: 1.25,
            address: "0xAbC".to_string(),
            transaction_hash: Some("hash-1".to_string()),
            ethereum_nonce: Some(7),
            receipt_block_number: Some(10),
            receipt_gas_used: Some("100".to_string()),
            receipt_effective_gas_price_gwei: Some(2.5),
            receipt_network_fee_eth: Some(0.01),
            fee_priority_raw: Some("normal".to_string()),
            fee_rate_description: Some("normal".to_string()),
            confirmation_count: Some(2),
            dogecoin_confirmed_network_fee_doge: Some(1.0),
            dogecoin_estimated_fee_rate_doge_per_kb: Some(3.0),
            used_change_output: Some(true),
            source_derivation_path: Some("m/0/0".to_string()),
            change_derivation_path: Some("m/1/0".to_string()),
            source_address: Some("source-old".to_string()),
            change_address: Some("change-old".to_string()),
            signed_transaction_payload: Some("payload-old".to_string()),
            signed_transaction_payload_format: Some("hex".to_string()),
            failure_reason: Some("old-failure".to_string()),
            transaction_history_source: Some("rpc".to_string()),
            created_at_unix: 250.0,
        }
    }

    #[test]
    fn merges_standard_utxo_transactions_with_swift_compatible_precedence() {
        let existing = sample_transaction("Bitcoin");
        let mut incoming = sample_transaction("Bitcoin");
        incoming.id = "tx-2".to_string();
        incoming.status = "confirmed".to_string();
        incoming.amount = 2.0;
        incoming.address = "incoming-address".to_string();
        incoming.receipt_gas_used = None;
        incoming.receipt_effective_gas_price_gwei = None;
        incoming.receipt_network_fee_eth = None;
        incoming.confirmation_count = Some(12);
        incoming.used_change_output = Some(false);
        incoming.source_address = Some("source-new".to_string());
        incoming.change_address = Some("change-new".to_string());
        incoming.failure_reason = None;
        incoming.transaction_history_source = Some("esplora".to_string());
        incoming.created_at_unix = 500.0;

        let merged = merge_transactions(TransactionMergeRequest {
            existing_transactions: vec![existing],
            incoming_transactions: vec![incoming],
            strategy: TransactionMergeStrategy::StandardUtxo,
            chain_name: "Bitcoin".to_string(),
            include_symbol_in_identity: false,
            preserve_created_at_sentinel_unix: None,
        });

        assert_eq!(merged.len(), 1);
        let record = &merged[0];
        assert_eq!(record.id, "tx-1");
        assert_eq!(record.status, "confirmed");
        assert_eq!(record.amount, 2.0);
        assert_eq!(record.confirmation_count, Some(12));
        assert_eq!(record.receipt_gas_used.as_deref(), Some("100"));
        assert_eq!(record.receipt_effective_gas_price_gwei, Some(2.5));
        assert_eq!(record.used_change_output, Some(false));
        assert_eq!(record.source_derivation_path.as_deref(), Some("m/0/0"));
        assert_eq!(record.source_address.as_deref(), Some("source-new"));
        assert_eq!(record.failure_reason.as_deref(), Some("old-failure"));
        assert_eq!(
            record.transaction_history_source.as_deref(),
            Some("esplora")
        );
        assert_eq!(record.created_at_unix, 500.0);
    }

    #[test]
    fn merges_evm_transactions_and_preserves_existing_created_at_for_sentinel_values() {
        let mut existing = sample_transaction("Ethereum");
        existing.symbol = "USDC".to_string();
        existing.amount = 3.5;
        existing.address = "0xABCDEF".to_string();
        existing.source_address = Some("keep-source".to_string());
        existing.created_at_unix = 900.0;

        let mut incoming = sample_transaction("Ethereum");
        incoming.id = "tx-2".to_string();
        incoming.symbol = "USDC".to_string();
        incoming.amount = 3.5;
        incoming.address = " 0xabcdef ".to_string();
        incoming.receipt_gas_used = Some("222".to_string());
        incoming.receipt_effective_gas_price_gwei = Some(4.0);
        incoming.receipt_network_fee_eth = Some(0.02);
        incoming.failure_reason = Some("new-failure".to_string());
        incoming.created_at_unix = -999_999.0;

        let merged = merge_transactions(TransactionMergeRequest {
            existing_transactions: vec![existing],
            incoming_transactions: vec![incoming],
            strategy: TransactionMergeStrategy::Evm,
            chain_name: "Ethereum".to_string(),
            include_symbol_in_identity: false,
            preserve_created_at_sentinel_unix: Some(-999_999.0),
        });

        assert_eq!(merged.len(), 1);
        let record = &merged[0];
        assert_eq!(record.id, "tx-1");
        assert_eq!(record.receipt_gas_used.as_deref(), Some("222"));
        assert_eq!(record.receipt_effective_gas_price_gwei, Some(4.0));
        assert_eq!(record.receipt_network_fee_eth, Some(0.02));
        assert_eq!(record.source_address.as_deref(), Some("keep-source"));
        assert_eq!(record.failure_reason.as_deref(), Some("new-failure"));
        assert_eq!(record.created_at_unix, 900.0);
    }
}

// ── FFI surface ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod wire_persisted_conversion {
    use super::*;

    /// Every field populated with a distinguishable value, so a field dropped
    /// or crossed in the conversion shows up as an inequality rather than
    /// passing on defaults.
    fn populated_wire() -> CoreTransactionRecord {
        CoreTransactionRecord {
            id: "TX-1".to_string(),
            wallet_id: Some("w1".to_string()),
            kind: "send".to_string(),
            status: "confirmed".to_string(),
            wallet_name: "Wallet".to_string(),
            asset_name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            chain_name: "Bitcoin".to_string(),
            amount: 1.25,
            address: "bc1qexample".to_string(),
            transaction_hash: Some("0xhash".to_string()),
            ethereum_nonce: Some(7),
            receipt_block_number: Some(1234),
            receipt_gas_used: Some("21000".to_string()),
            receipt_effective_gas_price_gwei: Some(12.5),
            receipt_network_fee_eth: Some(0.00042),
            fee_priority_raw: Some("priority".to_string()),
            fee_rate_description: Some("12 sat/vB".to_string()),
            confirmation_count: Some(6),
            dogecoin_confirmed_network_fee_doge: Some(1.5),
            dogecoin_estimated_fee_rate_doge_per_kb: Some(0.01),
            used_change_output: Some(true),
            source_derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
            change_derivation_path: Some("m/84'/0'/0'/1/0".to_string()),
            source_address: Some("bc1qsource".to_string()),
            change_address: Some("bc1qchange".to_string()),
            signed_transaction_payload: Some("deadbeef".to_string()),
            signed_transaction_payload_format: Some("hex".to_string()),
            failure_reason: Some("none".to_string()),
            transaction_history_source: Some("esplora".to_string()),
            created_at_unix: 1_700_000_000.0,
        }
    }

    #[test]
    fn wire_to_stored_and_back_is_lossless() {
        let original = populated_wire();
        let stored: CorePersistedTransactionRecord = original.clone().into();
        let round_tripped: CoreTransactionRecord = stored.into();
        assert_eq!(round_tripped, original);
    }

    #[test]
    fn the_timestamp_changes_epoch_and_returns() {
        let stored: CorePersistedTransactionRecord = populated_wire().into();
        // Same instant, expressed from 2001-01-01 instead of 1970-01-01.
        assert_eq!(stored.created_at, 1_700_000_000.0 - 978_307_200.0);
        let back: CoreTransactionRecord = stored.into();
        assert_eq!(back.created_at_unix, 1_700_000_000.0);
    }

    /// A stored record with no status is legacy data. Reading it applies the
    /// documented fallback rather than inventing a status.
    #[test]
    fn a_missing_stored_status_falls_back_by_kind() {
        let mut stored: CorePersistedTransactionRecord = populated_wire().into();
        stored.status = None;

        stored.kind = CoreTransactionKind::Send;
        let as_send: CoreTransactionRecord = stored.clone().into();
        assert_eq!(as_send.status, "confirmed");

        stored.kind = CoreTransactionKind::Receive;
        let as_receive: CoreTransactionRecord = stored.into();
        assert_eq!(as_receive.status, "pending");
    }

    /// An unrecognised status string does not silently become "pending" on the
    /// way in — it stores as absent, and the fallback above then applies.
    #[test]
    fn an_unknown_status_string_stores_as_absent() {
        let mut wire = populated_wire();
        wire.status = "who-knows".to_string();
        let stored: CorePersistedTransactionRecord = wire.into();
        assert_eq!(stored.status, None);
    }
}
