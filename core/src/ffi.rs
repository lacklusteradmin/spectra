use std::collections::HashMap;

use crate::SpectraBridgeError;
use super::addressing::{
    validate_address, validate_string_identifier, AddressValidationRequest,
    AddressValidationResult, StringValidationRequest, StringValidationResult,
};
use super::catalog::core_bootstrap;
use super::endpoint_reliability::{
    order_endpoints, record_attempt, EndpointAttemptRequest, EndpointOrderingRequest,
    ReliabilityCounter,
};
use super::fetch::{
    plan_balance_refresh_health, plan_dogecoin_refresh_targets, plan_evm_refresh_targets,
    plan_wallet_balance_refresh, BalanceRefreshHealthPlan, BalanceRefreshHealthRequest,
    DogecoinRefreshTargetsRequest, DogecoinRefreshWalletTarget, EvmRefreshPlan,
    EvmRefreshTargetsRequest, WalletBalanceRefreshPlan, WalletBalanceRefreshRequest,
};
use super::history::{
    merge_bitcoin_history_snapshots, normalize_history, CoreBitcoinHistorySnapshot,
    MergeBitcoinHistorySnapshotsRequest, NormalizeHistoryRequest, CoreNormalizedHistoryEntry,
};
use super::import::{
    plan_wallet_import, validate_wallet_import_draft, WalletImportDraftValidationRequest,
    WalletImportPlan, WalletImportRequest,
};
use super::localization::localization_catalog;
use super::refresh::{
    active_maintenance_plan, chain_plans, history_plans, should_run_background_maintenance,
    ActiveMaintenancePlan, ActiveMaintenancePlanRequest, BackgroundMaintenanceRequest,
    ChainRefreshPlan, ChainRefreshPlanRequest, HistoryRefreshPlanRequest,
};
use super::resources::{static_json_resource, static_text_resource};
use super::send::{
    plan_send_preview_routing, plan_send_submit_preflight, route_send_asset, SendAssetRoutingInput,
    SendAssetRoutingPlan, SendPreviewRoutingPlan, SendPreviewRoutingRequest,
    SendSubmitPreflightPlan, SendSubmitPreflightRequest,
};
use super::state::CoreAppState;
use super::store::{
    aggregate_owned_addresses, build_persisted_snapshot, build_persisted_snapshot_typed,
    persisted_snapshot_from_json, plan_receive_selection, plan_self_send_confirmation,
    plan_store_derived_state, wallet_secret_index, wallet_secret_index_from_observations,
    OwnedAddressAggregationRequest, PersistedAppSnapshotRequest, ReceiveSelectionPlan,
    ReceiveSelectionRequest, SelfSendConfirmationPlan, SelfSendConfirmationRequest,
    StoreDerivedStatePlan, StoreDerivedStateRequest, WalletSecretIndex, WalletSecretObservation,
};
use super::persistence::models::{
    CorePersistedAddressBookStore, CorePersistedPriceAlertStore, CorePersistedTransactionRecord,
};
use super::transactions::{merge_transactions, TransactionMergeRequest, CoreTransactionRecord};
use super::transfer::{
    plan_transfer_availability, TransferAvailabilityPlan, TransferAvailabilityRequest,
};
use super::utxo::{
    plan_utxo_preview, plan_utxo_spend, UtxoPreviewPlan, UtxoPreviewRequest, UtxoSpendPlan,
    UtxoSpendPlanRequest,
};
use serde::Serialize;

// ─── Inherent JSON functions (kept as-is) ────────────────────────────────────

#[uniffi::export]
pub fn core_bootstrap_json() -> Result<String, crate::SpectraBridgeError> {
    Ok(serialize_json(&core_bootstrap()?)?)
}

#[uniffi::export]
pub fn core_localization_document_json(
    resource_name: String,
    preferred_locales_json: String,
) -> Result<Vec<u8>, crate::SpectraBridgeError> {
    let preferred_locales = serde_json::from_str::<Vec<String>>(&preferred_locales_json)?;
    let document = localization_catalog()?
        .document_for(&preferred_locales, &resource_name)
        .cloned()
        .ok_or_else(|| format!("Missing localization document for table {resource_name}."))?;
    Ok(serialize_json(&document)?.into_bytes())
}

#[uniffi::export]
pub fn core_static_resource_json(
    resource_name: String,
) -> Result<Vec<u8>, crate::SpectraBridgeError> {
    let json = static_json_resource(&resource_name)
        .map(|value| value.to_string())
        .ok_or_else(|| format!("Missing static JSON resource {resource_name}."))?;
    Ok(json.into_bytes())
}

#[uniffi::export]
pub fn core_static_text_resource_utf8(
    resource_name: String,
) -> Result<String, crate::SpectraBridgeError> {
    static_text_resource(&resource_name)
        .map(|value| value.to_string())
        .ok_or_else(|| format!("Missing static text resource {resource_name}.").into())
}

// ─── Persistence JSON boundary (state is inherently JSON from disk) ───────────

#[uniffi::export]
pub fn core_build_persisted_snapshot_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<PersistedAppSnapshotRequest>(&request_json)?;
    Ok(serialize_json(&build_persisted_snapshot(request)?)?)
}

#[uniffi::export]
pub fn core_wallet_secret_index_json(
    snapshot_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let snapshot = persisted_snapshot_from_json(&snapshot_json)?;
    Ok(serialize_json(&wallet_secret_index(&snapshot))?)
}

// ─── Typed FFI functions ──────────────────────────────────────────────────────

#[uniffi::export]
pub fn core_build_persisted_snapshot(
    app_state: CoreAppState,
    secret_observations: Vec<WalletSecretObservation>,
) -> super::store::PersistedAppSnapshot {
    build_persisted_snapshot_typed(app_state, secret_observations)
}

#[uniffi::export]
pub fn core_wallet_secret_index(
    app_state: CoreAppState,
    secret_observations: Vec<WalletSecretObservation>,
) -> WalletSecretIndex {
    wallet_secret_index_from_observations(app_state, secret_observations)
}

#[uniffi::export]
pub fn core_plan_store_derived_state(
    request: StoreDerivedStateRequest,
) -> StoreDerivedStatePlan {
    plan_store_derived_state(request)
}

#[uniffi::export]
pub fn core_aggregate_owned_addresses(
    request: OwnedAddressAggregationRequest,
) -> Vec<String> {
    aggregate_owned_addresses(request)
}

#[uniffi::export]
pub fn core_plan_receive_selection(
    request: ReceiveSelectionRequest,
) -> ReceiveSelectionPlan {
    plan_receive_selection(request)
}

#[uniffi::export]
pub fn core_plan_self_send_confirmation(
    request: SelfSendConfirmationRequest,
) -> SelfSendConfirmationPlan {
    plan_self_send_confirmation(request)
}

#[uniffi::export]
pub fn core_plan_wallet_import(
    request: WalletImportRequest,
) -> Result<WalletImportPlan, crate::SpectraBridgeError> {
    Ok(plan_wallet_import(request)?)
}

#[uniffi::export]
pub fn core_validate_wallet_import_draft(
    request: WalletImportDraftValidationRequest,
) -> bool {
    validate_wallet_import_draft(request)
}

#[uniffi::export]
pub fn core_active_maintenance_plan(
    request: ActiveMaintenancePlanRequest,
) -> ActiveMaintenancePlan {
    active_maintenance_plan(request)
}

#[uniffi::export]
pub fn core_should_run_background_maintenance(
    request: BackgroundMaintenanceRequest,
) -> bool {
    should_run_background_maintenance(request)
}

#[uniffi::export]
pub fn core_chain_refresh_plans(
    request: ChainRefreshPlanRequest,
) -> Vec<ChainRefreshPlan> {
    chain_plans(request)
}

#[uniffi::export]
pub fn core_history_refresh_plans(
    request: HistoryRefreshPlanRequest,
) -> Vec<String> {
    history_plans(request)
}

#[uniffi::export]
pub fn core_normalize_history(
    request: NormalizeHistoryRequest,
) -> Vec<CoreNormalizedHistoryEntry> {
    normalize_history(request)
}

#[uniffi::export]
pub fn core_merge_bitcoin_history_snapshots(
    request: MergeBitcoinHistorySnapshotsRequest,
) -> Vec<CoreBitcoinHistorySnapshot> {
    merge_bitcoin_history_snapshots(request)
}

#[uniffi::export]
pub fn core_plan_evm_refresh_targets(
    request: EvmRefreshTargetsRequest,
) -> EvmRefreshPlan {
    plan_evm_refresh_targets(request)
}

#[uniffi::export]
pub fn core_plan_dogecoin_refresh_targets(
    request: DogecoinRefreshTargetsRequest,
) -> Vec<DogecoinRefreshWalletTarget> {
    plan_dogecoin_refresh_targets(request)
}

#[uniffi::export]
pub fn core_plan_wallet_balance_refresh(
    request: WalletBalanceRefreshRequest,
) -> WalletBalanceRefreshPlan {
    plan_wallet_balance_refresh(request)
}

#[uniffi::export]
pub fn core_plan_balance_refresh_health(
    request: BalanceRefreshHealthRequest,
) -> BalanceRefreshHealthPlan {
    plan_balance_refresh_health(request)
}

#[uniffi::export]
pub fn core_plan_transfer_availability(
    request: TransferAvailabilityRequest,
) -> TransferAvailabilityPlan {
    plan_transfer_availability(request)
}

#[uniffi::export]
pub fn core_route_send_asset(
    request: SendAssetRoutingInput,
) -> SendAssetRoutingPlan {
    route_send_asset(&request)
}

#[uniffi::export]
pub fn core_plan_send_preview_routing(
    request: SendPreviewRoutingRequest,
) -> SendPreviewRoutingPlan {
    plan_send_preview_routing(request)
}

#[uniffi::export]
pub fn core_plan_send_submit_preflight(
    request: SendSubmitPreflightRequest,
) -> Result<SendSubmitPreflightPlan, crate::SpectraBridgeError> {
    Ok(plan_send_submit_preflight(request)?)
}

#[uniffi::export]
pub fn core_plan_utxo_preview(
    request: UtxoPreviewRequest,
) -> Result<UtxoPreviewPlan, crate::SpectraBridgeError> {
    Ok(plan_utxo_preview(request)?)
}

#[uniffi::export]
pub fn core_plan_utxo_spend(
    request: UtxoSpendPlanRequest,
) -> Result<UtxoSpendPlan, crate::SpectraBridgeError> {
    Ok(plan_utxo_spend(request)?)
}

#[uniffi::export]
pub fn core_merge_transactions(
    request: TransactionMergeRequest,
) -> Vec<CoreTransactionRecord> {
    merge_transactions(request)
}

/// Typed input for `core_encode_history_records_json`. `payload_json` is the
/// full `PersistedTransactionRecord` JSON blob (unencoded); Rust base64-encodes it
/// into the resulting HistoryRecord payload.
#[derive(Debug, Clone, Serialize, serde::Deserialize, uniffi::Record)]
pub struct HistoryRecordEncodeInput {
    pub id: String,
    pub wallet_id: Option<String>,
    pub chain_name: String,
    pub tx_hash: Option<String>,
    pub created_at: f64,
    pub payload_json: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryRecordEncoded {
    id: String,
    wallet_id: Option<String>,
    chain_name: String,
    tx_hash: Option<String>,
    created_at: f64,
    payload: String,
}

#[uniffi::export]
pub fn core_encode_history_records_json(
    records: Vec<HistoryRecordEncodeInput>,
) -> Result<String, crate::SpectraBridgeError> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let encoded: Vec<HistoryRecordEncoded> = records
        .into_iter()
        .map(|r| HistoryRecordEncoded {
            id: r.id,
            wallet_id: r.wallet_id,
            chain_name: r.chain_name,
            tx_hash: r.tx_hash,
            created_at: r.created_at,
            payload: engine.encode(r.payload_json.as_bytes()),
        })
        .collect();
    Ok(serialize_json(&encoded)?)
}

#[uniffi::export]
pub fn core_validate_address(
    request: AddressValidationRequest,
) -> AddressValidationResult {
    validate_address(request)
}

#[uniffi::export]
pub fn core_validate_string_identifier(
    request: StringValidationRequest,
) -> StringValidationResult {
    validate_string_identifier(request)
}

#[uniffi::export]
pub fn core_order_endpoints_by_reliability(
    request: EndpointOrderingRequest,
) -> Vec<String> {
    order_endpoints(request)
}

#[uniffi::export]
pub fn core_record_endpoint_attempt(
    request: EndpointAttemptRequest,
) -> HashMap<String, ReliabilityCounter> {
    record_attempt(request)
}

// ─── High-risk send warning evaluation ──────────────────────────────────────

/// A chain_name + address pair used in the high-risk send evaluation.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HighRiskChainAddress {
    pub chain_name: String,
    pub address: String,
}

/// Typed input for high-risk send evaluation — replaces the JSON dict that
/// Swift previously assembled via `JSONSerialization`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HighRiskSendRequest {
    pub chain_name: String,
    pub symbol: String,
    pub amount: f64,
    pub holding_amount: f64,
    pub destination_address: String,
    pub destination_input: String,
    pub used_ens_resolution: bool,
    pub wallet_selected_chain: String,
    pub address_book_entries: Vec<HighRiskChainAddress>,
    pub tx_addresses: Vec<HighRiskChainAddress>,
}

/// A single high-risk warning with a code and optional metadata fields.
/// Swift maps these to localized user-facing strings.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HighRiskSendWarning {
    pub code: String,
    pub chain: Option<String>,
    pub name: Option<String>,
    pub address: Option<String>,
    pub percent: Option<u64>,
    pub symbol: Option<String>,
}

/// Typed high-risk send evaluation — replaces `core_evaluate_high_risk_send_reasons_json`.
#[uniffi::export]
pub fn core_evaluate_high_risk_send_reasons(
    request: HighRiskSendRequest,
) -> Vec<HighRiskSendWarning> {
    let chain_name = &request.chain_name;
    let mut warnings: Vec<HighRiskSendWarning> = Vec::new();

    let make = |code: &str| HighRiskSendWarning {
        code: code.to_string(), chain: None, name: None, address: None, percent: None, symbol: None,
    };

    // 1. Address format validation.
    if !hrsr_validate_address(chain_name, &request.destination_address) {
        warnings.push(HighRiskSendWarning { chain: Some(chain_name.clone()), ..make("invalid_format") });
    }

    // Normalize destination for case-insensitive comparison.
    let norm_dest = hrsr_normalize_address(&request.destination_address, chain_name).to_lowercase();

    // 2. New address detection.
    let has_address_book = request.address_book_entries.iter().any(|e| {
        e.chain_name == *chain_name
            && hrsr_normalize_address(&e.address, chain_name).to_lowercase() == norm_dest
    });
    let has_tx_history = request.tx_addresses.iter().any(|e| {
        e.chain_name == *chain_name
            && hrsr_normalize_address(&e.address, chain_name).to_lowercase() == norm_dest
    });
    if !has_address_book && !has_tx_history {
        warnings.push(make("new_address"));
    }

    // 3. ENS resolution warning.
    if request.used_ens_resolution {
        warnings.push(HighRiskSendWarning {
            name: Some(request.destination_input.clone()),
            address: Some(request.destination_address.clone()),
            ..make("ens_resolved")
        });
    }

    // 4. Large send percentage (≥25 % of holding balance).
    if request.holding_amount > 0.0 {
        let ratio = request.amount / request.holding_amount;
        if ratio >= 0.25 {
            let pct = (ratio * 100.0).round() as u64;
            warnings.push(HighRiskSendWarning {
                percent: Some(pct), symbol: Some(request.symbol.clone()), ..make("large_send")
            });
        }
    }

    // 5-10. Cross-chain prefix mismatch checks.
    let lowered = request.destination_input.to_lowercase();
    let is_evm = matches!(
        chain_name.as_str(),
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism"
            | "BNB Chain" | "Avalanche" | "Hyperliquid"
    );
    let is_l2 = matches!(
        chain_name.as_str(),
        "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche" | "Hyperliquid"
    );
    let is_ens_candidate = lowered.ends_with(".eth")
        && !lowered.contains(' ')
        && !lowered.starts_with("0x");

    if is_evm {
        let looks_non_evm = lowered.starts_with("bc1")
            || lowered.starts_with("tb1")
            || lowered.starts_with("ltc1")
            || lowered.starts_with("bnb1")
            || lowered.starts_with('t')
            || lowered.starts_with('d')
            || lowered.starts_with('a');
        if looks_non_evm {
            warnings.push(HighRiskSendWarning { chain: Some(chain_name.clone()), ..make("non_evm_on_evm") });
        }
        if is_l2 && is_ens_candidate {
            warnings.push(HighRiskSendWarning { chain: Some(chain_name.clone()), ..make("ens_on_l2") });
        }
    } else if matches!(chain_name.as_str(), "Bitcoin" | "Bitcoin Cash" | "Litecoin" | "Dogecoin") {
        if lowered.starts_with("0x") || is_ens_candidate {
            warnings.push(HighRiskSendWarning { chain: Some(chain_name.clone()), ..make("eth_on_utxo") });
        }
    } else if chain_name == "Tron" {
        if lowered.starts_with("0x") || lowered.starts_with("bc1") {
            warnings.push(make("non_tron"));
        }
    } else if chain_name == "Solana" {
        if lowered.starts_with("0x")
            || lowered.starts_with("bc1")
            || lowered.starts_with("ltc1")
            || lowered.starts_with('t')
        {
            warnings.push(make("non_solana"));
        }
    } else if chain_name == "XRP Ledger" {
        if lowered.starts_with("0x")
            || lowered.starts_with("bc1")
            || lowered.starts_with('t')
        {
            warnings.push(make("non_xrp"));
        }
    } else if chain_name == "Monero" {
        if lowered.starts_with("0x")
            || lowered.starts_with("bc1")
            || lowered.starts_with('r')
        {
            warnings.push(make("non_monero"));
        }
    }

    // 11. Wallet-chain context mismatch.
    if !request.wallet_selected_chain.is_empty() && request.wallet_selected_chain != *chain_name {
        warnings.push(make("chain_mismatch"));
    }

    warnings
}

fn hrsr_validate_address(chain_name: &str, address: &str) -> bool {
    let Some(kind) = super::send::flow::chain_kind(chain_name) else { return false };
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: address.to_string(),
        network_mode: None,
    })
    .is_valid
}

fn hrsr_normalize_address(address: &str, chain_name: &str) -> String {
    super::send::flow::normalize_address(chain_name, address)
}

fn serialize_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(display_error)
}

// ─── Persisted DTO JSON bridges ─────────────────────────────────────────────
// Swift `JSONEncoder` / `JSONDecoder` cannot produce the "omit-when-None" wire
// shape Rust uses for optional fields. Swift therefore round-trips these three
// DTOs through serde_json via these bridges to keep byte-exact on-disk JSON.

#[uniffi::export]
pub fn encode_persisted_price_alert_store_json(
    value: CorePersistedPriceAlertStore,
) -> Result<String, SpectraBridgeError> {
    Ok(serialize_json(&value)?)
}

#[uniffi::export]
pub fn decode_persisted_price_alert_store_json(
    json: String,
) -> Result<CorePersistedPriceAlertStore, SpectraBridgeError> {
    serde_json::from_str::<CorePersistedPriceAlertStore>(&json)
        .map_err(SpectraBridgeError::from)
}

#[uniffi::export]
pub fn encode_persisted_address_book_store_json(
    value: CorePersistedAddressBookStore,
) -> Result<String, SpectraBridgeError> {
    Ok(serialize_json(&value)?)
}

#[uniffi::export]
pub fn decode_persisted_address_book_store_json(
    json: String,
) -> Result<CorePersistedAddressBookStore, SpectraBridgeError> {
    serde_json::from_str::<CorePersistedAddressBookStore>(&json)
        .map_err(SpectraBridgeError::from)
}

#[uniffi::export]
pub fn encode_persisted_transaction_record_json(
    value: CorePersistedTransactionRecord,
) -> Result<String, SpectraBridgeError> {
    Ok(serialize_json(&value)?)
}

#[uniffi::export]
pub fn decode_persisted_transaction_record_json(
    json: String,
) -> Result<CorePersistedTransactionRecord, SpectraBridgeError> {
    serde_json::from_str::<CorePersistedTransactionRecord>(&json)
        .map_err(SpectraBridgeError::from)
}

// ─── Seed envelope encryption (Phase 1) ─────────────────────────────────────

/// Encrypt a seed phrase with AES-256-GCM. `master_key_bytes` must be exactly
/// 32 bytes. Returns the JSON envelope as `Data` (compatible with Swift's
/// existing `SeedMaterialEnvelope` keychain format).
#[uniffi::export]
pub fn encrypt_seed_envelope(
    plaintext: String,
    master_key_bytes: Vec<u8>,
) -> Result<Vec<u8>, SpectraBridgeError> {
    super::store::seed_envelope::encrypt(plaintext.as_bytes(), &master_key_bytes)
        .map_err(SpectraBridgeError::from)
}

/// Decrypt a seed envelope produced by [`encrypt_seed_envelope`] or by Swift's
/// `SeedMaterialEnvelope.encode`. Returns the plaintext seed phrase.
#[uniffi::export]
pub fn decrypt_seed_envelope(
    data: Vec<u8>,
    master_key_bytes: Vec<u8>,
) -> Result<String, SpectraBridgeError> {
    super::store::seed_envelope::decrypt(&data, &master_key_bytes)
        .map_err(SpectraBridgeError::from)
}

// ─── Password verifier (Phase 2) ────────────────────────────────────────────

/// Create a PBKDF2-HMAC-SHA256 password verifier envelope. Returns JSON `Data`
/// compatible with Swift's `SecureSeedPasswordStore` format.
#[uniffi::export]
pub fn create_password_verifier(
    password: String,
) -> Result<Vec<u8>, SpectraBridgeError> {
    super::store::password_verifier::create_verifier(&password)
        .map_err(SpectraBridgeError::from)
}

/// Verify a password against a PBKDF2 verifier envelope produced by
/// [`create_password_verifier`] or by Swift's `SecureSeedPasswordStore.save`.
#[uniffi::export]
pub fn verify_password_verifier(
    password: String,
    verifier_data: Vec<u8>,
) -> bool {
    super::store::password_verifier::verify(&password, &verifier_data)
}

// ── Derivation path parsing ──

pub use super::app_core::DerivationPathSegment;

#[uniffi::export]
pub fn core_parse_derivation_path(raw_path: String) -> Option<Vec<DerivationPathSegment>> {
    super::app_core::parse_derivation_path(&raw_path)
}

#[uniffi::export]
pub fn core_derivation_path_string(segments: Vec<DerivationPathSegment>) -> String {
    super::app_core::derivation_path_string(&segments)
}

#[uniffi::export]
pub fn core_normalize_derivation_path(raw_path: String, fallback: String) -> String {
    super::app_core::normalize_derivation_path(&raw_path, &fallback)
}

#[uniffi::export]
pub fn core_derivation_path_segment_value(path: String, index: u32) -> Option<u32> {
    super::app_core::derivation_path_segment_value(&path, index as usize)
}

#[uniffi::export]
pub fn core_derivation_path_replacing_last_two(
    raw_path: String,
    branch: u32,
    index: u32,
    fallback: String,
) -> String {
    let normalized = super::app_core::normalize_derivation_path(&raw_path, &fallback);
    let Some(mut segments) = super::app_core::parse_derivation_path(&normalized) else {
        return fallback;
    };
    if segments.len() < 2 {
        return fallback;
    }
    let len = segments.len();
    segments[len - 2] = DerivationPathSegment {
        value: branch,
        is_hardened: false,
    };
    segments[len - 1] = DerivationPathSegment {
        value: index,
        is_hardened: false,
    };
    super::app_core::derivation_path_string(&segments)
}

// ── Private key hex ──

#[uniffi::export]
pub fn core_private_key_hex_normalized(raw_value: String) -> String {
    let trimmed = raw_value.trim().to_lowercase();
    match trimmed.strip_prefix("0x") {
        Some(stripped) => stripped.to_string(),
        None => trimmed,
    }
}

#[uniffi::export]
pub fn core_private_key_hex_is_likely(raw_value: String) -> bool {
    let normalized = core_private_key_hex_normalized(raw_value);
    normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit())
}

// ── Endpoint role mask ──

#[uniffi::export]
pub fn core_endpoint_role_mask(roles: Vec<String>) -> u32 {
    roles
        .iter()
        .fold(0u32, |mask, role| mask | super::app_core::endpoint_role_bit(role))
}

// ── Large-movement threshold evaluation ──

#[derive(Debug, Clone, uniffi::Record)]
pub struct LargeMovementEvaluation {
    pub should_alert: bool,
    pub absolute_delta: f64,
    pub ratio: f64,
    pub direction_up: bool,
}

#[uniffi::export]
pub fn core_evaluate_large_movement(
    previous_total_usd: f64,
    current_total_usd: f64,
    usd_threshold: f64,
    percent_threshold: f64,
) -> LargeMovementEvaluation {
    if previous_total_usd <= 0.0 {
        return LargeMovementEvaluation {
            should_alert: false,
            absolute_delta: 0.0,
            ratio: 0.0,
            direction_up: true,
        };
    }
    let delta = current_total_usd - previous_total_usd;
    let absolute_delta = delta.abs();
    let ratio = absolute_delta / previous_total_usd;
    let should_alert =
        absolute_delta >= usd_threshold && ratio >= (percent_threshold / 100.0);
    LargeMovementEvaluation {
        should_alert,
        absolute_delta,
        ratio,
        direction_up: delta >= 0.0,
    }
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
