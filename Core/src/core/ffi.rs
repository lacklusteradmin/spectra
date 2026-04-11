use super::addressing::{
    validate_address, validate_string_identifier, AddressValidationRequest, StringValidationRequest,
};
use super::catalog::core_bootstrap;
use super::endpoint_reliability::{
    order_endpoints, record_attempt, EndpointAttemptRequest, EndpointOrderingRequest,
};
use super::fetch::{
    plan_balance_refresh_health, plan_dogecoin_refresh_targets, plan_evm_refresh_targets,
    plan_wallet_balance_refresh, BalanceRefreshHealthRequest, DogecoinRefreshTargetsRequest,
    EvmRefreshTargetsRequest, WalletBalanceRefreshRequest,
};
use super::history::{
    merge_bitcoin_history_snapshots, normalize_history, MergeBitcoinHistorySnapshotsRequest,
    NormalizeHistoryRequest,
};
use super::import::{plan_wallet_import, WalletImportRequest};
use super::localization::localization_catalog;
use super::migration::{core_state_to_legacy_wallet_store_json, legacy_wallet_store_to_core_state};
use super::refresh::{
    active_maintenance_plan, chain_plans, history_plans, should_run_background_maintenance,
    ActiveMaintenancePlanRequest, BackgroundMaintenanceRequest, ChainRefreshPlanRequest,
    HistoryRefreshPlanRequest,
};
use super::resources::{static_json_resource, static_text_resource};
use super::send::{
    plan_send_preview_routing, plan_send_submit_preflight, route_send_asset, SendAssetRoutingInput,
    SendPreviewRoutingRequest, SendSubmitPreflightRequest,
};
use super::state::{reduce_state, CoreAppState, StateCommand};
use super::store::{
    aggregate_owned_addresses, build_persisted_snapshot, persisted_snapshot_from_json,
    plan_receive_selection, plan_self_send_confirmation, plan_store_derived_state,
    wallet_secret_index, OwnedAddressAggregationRequest, PersistedAppSnapshotRequest,
    ReceiveSelectionRequest, SelfSendConfirmationRequest, StoreDerivedStateRequest,
};
use super::transactions::{merge_transactions, TransactionMergeRequest};
use super::transfer::{plan_transfer_availability, TransferAvailabilityRequest};
use super::utxo::{plan_utxo_preview, plan_utxo_spend, UtxoPreviewRequest, UtxoSpendPlanRequest};
use serde::Serialize;
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

#[uniffi::export]
pub fn core_reduce_state_json(
    state_json: String,
    command_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let state = serde_json::from_str::<CoreAppState>(&state_json)?;
    let command = serde_json::from_str::<StateCommand>(&command_json)?;
    Ok(serialize_json(&reduce_state(state, command))?)
}

#[uniffi::export]
pub fn core_migrate_legacy_wallet_store_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    Ok(serialize_json(&legacy_wallet_store_to_core_state(
        &request_json,
    )?)?)
}

#[uniffi::export]
pub fn core_export_legacy_wallet_store_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let state = serde_json::from_str::<CoreAppState>(&request_json)?;
    Ok(core_state_to_legacy_wallet_store_json(&state)?)
}

#[uniffi::export]
pub fn core_build_persisted_snapshot_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<PersistedAppSnapshotRequest>(&request_json)?;
    Ok(serialize_json(&build_persisted_snapshot(request)?)?)
}

#[uniffi::export]
pub fn core_wallet_secret_index_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let snapshot = persisted_snapshot_from_json(&request_json)?;
    Ok(serialize_json(&wallet_secret_index(&snapshot))?)
}

#[uniffi::export]
pub fn core_plan_store_derived_state_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<StoreDerivedStateRequest>(&request_json)?;
    Ok(serialize_json(&plan_store_derived_state(request))?)
}

#[uniffi::export]
pub fn core_aggregate_owned_addresses_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<OwnedAddressAggregationRequest>(&request_json)?;
    Ok(serialize_json(&aggregate_owned_addresses(request))?)
}

#[uniffi::export]
pub fn core_plan_receive_selection_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<ReceiveSelectionRequest>(&request_json)?;
    Ok(serialize_json(&plan_receive_selection(request))?)
}

#[uniffi::export]
pub fn core_plan_self_send_confirmation_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<SelfSendConfirmationRequest>(&request_json)?;
    Ok(serialize_json(&plan_self_send_confirmation(request))?)
}

#[uniffi::export]
pub fn core_plan_wallet_import_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<WalletImportRequest>(&request_json)?;
    Ok(serialize_json(&plan_wallet_import(request)?)?)
}

#[uniffi::export]
pub fn core_active_maintenance_plan_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<ActiveMaintenancePlanRequest>(&request_json)?;
    Ok(serialize_json(&active_maintenance_plan(request))?)
}

#[uniffi::export]
pub fn core_should_run_background_maintenance_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<BackgroundMaintenanceRequest>(&request_json)?;
    Ok(serialize_json(&should_run_background_maintenance(request))?)
}

#[uniffi::export]
pub fn core_chain_refresh_plans_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<ChainRefreshPlanRequest>(&request_json)?;
    Ok(serialize_json(&chain_plans(request))?)
}

#[uniffi::export]
pub fn core_history_refresh_plans_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<HistoryRefreshPlanRequest>(&request_json)?;
    Ok(serialize_json(&history_plans(request))?)
}

#[uniffi::export]
pub fn core_normalize_history_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<NormalizeHistoryRequest>(&request_json)?;
    Ok(serialize_json(&normalize_history(request))?)
}

#[uniffi::export]
pub fn core_merge_bitcoin_history_snapshots_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<MergeBitcoinHistorySnapshotsRequest>(&request_json)?;
    Ok(serialize_json(&merge_bitcoin_history_snapshots(request))?)
}

#[uniffi::export]
pub fn core_plan_evm_refresh_targets_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<EvmRefreshTargetsRequest>(&request_json)?;
    Ok(serialize_json(&plan_evm_refresh_targets(request))?)
}

#[uniffi::export]
pub fn core_plan_dogecoin_refresh_targets_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<DogecoinRefreshTargetsRequest>(&request_json)?;
    Ok(serialize_json(&plan_dogecoin_refresh_targets(request))?)
}

#[uniffi::export]
pub fn core_plan_wallet_balance_refresh_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<WalletBalanceRefreshRequest>(&request_json)?;
    Ok(serialize_json(&plan_wallet_balance_refresh(request))?)
}

#[uniffi::export]
pub fn core_plan_balance_refresh_health_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<BalanceRefreshHealthRequest>(&request_json)?;
    Ok(serialize_json(&plan_balance_refresh_health(request))?)
}

#[uniffi::export]
pub fn core_plan_transfer_availability_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<TransferAvailabilityRequest>(&request_json)?;
    Ok(serialize_json(&plan_transfer_availability(request))?)
}

#[uniffi::export]
pub fn core_route_send_asset_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<SendAssetRoutingInput>(&request_json)?;
    Ok(serialize_json(&route_send_asset(&request))?)
}

#[uniffi::export]
pub fn core_plan_send_preview_routing_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<SendPreviewRoutingRequest>(&request_json)?;
    Ok(serialize_json(&plan_send_preview_routing(request))?)
}

#[uniffi::export]
pub fn core_plan_send_submit_preflight_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<SendSubmitPreflightRequest>(&request_json)?;
    Ok(serialize_json(&plan_send_submit_preflight(request)?)?)
}

#[uniffi::export]
pub fn core_plan_utxo_preview_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<UtxoPreviewRequest>(&request_json)?;
    Ok(serialize_json(&plan_utxo_preview(request)?)?)
}

#[uniffi::export]
pub fn core_plan_utxo_spend_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<UtxoSpendPlanRequest>(&request_json)?;
    Ok(serialize_json(&plan_utxo_spend(request)?)?)
}

#[uniffi::export]
pub fn core_merge_transactions_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<TransactionMergeRequest>(&request_json)?;
    Ok(serialize_json(&merge_transactions(request))?)
}

#[uniffi::export]
pub fn core_validate_address_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<AddressValidationRequest>(&request_json)?;
    Ok(serialize_json(&validate_address(request))?)
}

#[uniffi::export]
pub fn core_validate_string_identifier_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<StringValidationRequest>(&request_json)?;
    Ok(serialize_json(&validate_string_identifier(request))?)
}

#[uniffi::export]
pub fn core_order_endpoints_by_reliability_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<EndpointOrderingRequest>(&request_json)?;
    Ok(serialize_json(&order_endpoints(request))?)
}

#[uniffi::export]
pub fn core_record_endpoint_attempt_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let request = serde_json::from_str::<EndpointAttemptRequest>(&request_json)?;
    Ok(serialize_json(&record_attempt(request))?)
}

fn serialize_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(display_error)
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
