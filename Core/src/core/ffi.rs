use super::catalog::core_bootstrap;
use super::fetch::{
    plan_dogecoin_refresh_targets, plan_evm_refresh_targets, plan_wallet_balance_refresh,
    DogecoinRefreshTargetsRequest, EvmRefreshTargetsRequest, WalletBalanceRefreshRequest,
};
use super::history::{normalize_history, NormalizeHistoryRequest};
use super::import::{plan_wallet_import, WalletImportRequest};
use super::localization::localization_catalog;
use super::migration::{core_state_to_legacy_wallet_store_json, legacy_wallet_store_to_core_state};
use super::refresh::{
    active_maintenance_plan, chain_plans, history_plans, should_run_background_maintenance,
    ActiveMaintenancePlanRequest, BackgroundMaintenanceRequest, ChainRefreshPlanRequest,
    HistoryRefreshPlanRequest,
};
use super::resources::{static_json_resource, static_text_resource, StaticResourceRequest};
use super::send::{
    plan_send_preview_routing, plan_send_submit_preflight, route_send_asset, SendAssetRoutingInput,
    SendPreviewRoutingRequest, SendSubmitPreflightRequest,
};
use super::state::{reduce_state, CoreAppState, StateCommand};
use super::store::{
    build_persisted_snapshot, persisted_snapshot_from_json, wallet_secret_index,
    PersistedAppSnapshotRequest,
};
use super::transactions::{merge_transactions, TransactionMergeRequest};
use super::transfer::{plan_transfer_availability, TransferAvailabilityRequest};
use super::utxo::{plan_utxo_preview, plan_utxo_spend, UtxoPreviewRequest, UtxoSpendPlanRequest};
use crate::derivation_runtime::SpectraBuffer;
use serde::Serialize;
use std::ptr;
use std::slice;

const STATUS_OK: i32 = 0;
const STATUS_ERROR: i32 = 1;

#[repr(C)]
pub struct SpectraJsonResponse {
    pub status_code: i32,
    pub payload_utf8: SpectraBuffer,
    pub error_message_utf8: SpectraBuffer,
}

impl SpectraJsonResponse {
    fn success(payload: String) -> *mut SpectraJsonResponse {
        Box::into_raw(Box::new(SpectraJsonResponse {
            status_code: STATUS_OK,
            payload_utf8: owned_buffer_from_string(payload),
            error_message_utf8: empty_buffer(),
        }))
    }

    fn error(message: impl Into<String>) -> *mut SpectraJsonResponse {
        Box::into_raw(Box::new(SpectraJsonResponse {
            status_code: STATUS_ERROR,
            payload_utf8: empty_buffer(),
            error_message_utf8: owned_buffer_from_string(message.into()),
        }))
    }
}

#[repr(C)]
pub struct SpectraCoreLocalizationDocumentRequest {
    pub resource_name_utf8: SpectraBuffer,
    pub preferred_locales_json_utf8: SpectraBuffer,
}

#[repr(C)]
pub struct SpectraCoreStateReduceRequest {
    pub state_json_utf8: SpectraBuffer,
    pub command_json_utf8: SpectraBuffer,
}

#[repr(C)]
pub struct SpectraCoreJSONRequest {
    pub json_utf8: SpectraBuffer,
}

#[no_mangle]
pub extern "C" fn spectra_core_bootstrap_json() -> *mut SpectraJsonResponse {
    json_response_from_result(core_bootstrap().and_then(|bootstrap| serialize_json(&bootstrap)))
}

#[no_mangle]
pub extern "C" fn spectra_core_localization_document_json(
    request: *const SpectraCoreLocalizationDocumentRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing localization document request.".to_string());
        }

        let request = unsafe { &*request };
        let resource_name = read_buffer_to_string(&request.resource_name_utf8)?;
        let preferred_locales =
            read_json_buffer::<Vec<String>>(&request.preferred_locales_json_utf8)?;
        let document = localization_catalog()?
            .document_for(&preferred_locales, &resource_name)
            .cloned()
            .ok_or_else(|| format!("Missing localization document for table {resource_name}."))?;
        serialize_json(&document)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_static_resource_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing static resource request.".to_string());
        }

        let request = unsafe { &*request };
        let request = read_json_buffer::<StaticResourceRequest>(&request.json_utf8)?;
        static_json_resource(&request.resource_name)
            .map(|json| json.to_string())
            .ok_or_else(|| format!("Missing static JSON resource {}.", request.resource_name))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_static_text_resource_utf8(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing static text resource request.".to_string());
        }

        let request = unsafe { &*request };
        let request = read_json_buffer::<StaticResourceRequest>(&request.json_utf8)?;
        static_text_resource(&request.resource_name)
            .map(|text| text.to_string())
            .ok_or_else(|| format!("Missing static text resource {}.", request.resource_name))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_reduce_state_json(
    request: *const SpectraCoreStateReduceRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing state reduction request.".to_string());
        }

        let request = unsafe { &*request };
        let state = read_json_buffer::<CoreAppState>(&request.state_json_utf8)?;
        let command = read_json_buffer::<StateCommand>(&request.command_json_utf8)?;
        let transition = reduce_state(state, command);
        serialize_json(&transition)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_migrate_legacy_wallet_store_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing legacy wallet store migration request.".to_string());
        }

        let request = unsafe { &*request };
        let json = read_buffer_to_string(&request.json_utf8)?;
        let state = legacy_wallet_store_to_core_state(&json)?;
        serialize_json(&state)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_export_legacy_wallet_store_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing legacy wallet store export request.".to_string());
        }

        let request = unsafe { &*request };
        let json = read_buffer_to_string(&request.json_utf8)?;
        let snapshot = persisted_snapshot_from_json(&json)?;
        core_state_to_legacy_wallet_store_json(&snapshot.app_state)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_build_persisted_snapshot_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing persisted snapshot build request.".to_string());
        }

        let request = unsafe { &*request };
        let request = read_json_buffer::<PersistedAppSnapshotRequest>(&request.json_utf8)?;
        let snapshot = build_persisted_snapshot(request)?;
        serialize_json(&snapshot)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_wallet_secret_index_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing wallet secret index request.".to_string());
        }

        let request = unsafe { &*request };
        let json = read_buffer_to_string(&request.json_utf8)?;
        let snapshot = persisted_snapshot_from_json(&json)?;
        serialize_json(&wallet_secret_index(&snapshot))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_wallet_import_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing wallet import planning request.".to_string());
        }

        let request = unsafe { &*request };
        let request = read_json_buffer::<WalletImportRequest>(&request.json_utf8)?;
        let plan = plan_wallet_import(request)?;
        serialize_json(&plan)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_active_maintenance_plan_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing active maintenance planning request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<ActiveMaintenancePlanRequest>(&request.json_utf8)?;
        serialize_json(&active_maintenance_plan(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_should_run_background_maintenance_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing background maintenance request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<BackgroundMaintenanceRequest>(&request.json_utf8)?;
        serialize_json(&should_run_background_maintenance(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_chain_refresh_plans_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing chain refresh planning request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<ChainRefreshPlanRequest>(&request.json_utf8)?;
        serialize_json(&chain_plans(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_history_refresh_plans_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing history refresh planning request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<HistoryRefreshPlanRequest>(&request.json_utf8)?;
        serialize_json(&history_plans(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_normalize_history_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing history normalization request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<NormalizeHistoryRequest>(&request.json_utf8)?;
        serialize_json(&normalize_history(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_evm_refresh_targets_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing EVM refresh target planning request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<EvmRefreshTargetsRequest>(&request.json_utf8)?;
        serialize_json(&plan_evm_refresh_targets(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_dogecoin_refresh_targets_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing Dogecoin refresh target planning request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<DogecoinRefreshTargetsRequest>(&request.json_utf8)?;
        serialize_json(&plan_dogecoin_refresh_targets(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_wallet_balance_refresh_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing wallet balance refresh planning request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<WalletBalanceRefreshRequest>(&request.json_utf8)?;
        serialize_json(&plan_wallet_balance_refresh(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_transfer_availability_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing transfer availability planning request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<TransferAvailabilityRequest>(&request.json_utf8)?;
        serialize_json(&plan_transfer_availability(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_route_send_asset_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing send asset routing request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<SendAssetRoutingInput>(&request.json_utf8)?;
        serialize_json(&route_send_asset(&request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_send_preview_routing_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing send preview routing request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<SendPreviewRoutingRequest>(&request.json_utf8)?;
        serialize_json(&plan_send_preview_routing(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_send_submit_preflight_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing send submit preflight request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<SendSubmitPreflightRequest>(&request.json_utf8)?;
        serialize_json(&plan_send_submit_preflight(request)?)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_utxo_preview_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing UTXO preview request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<UtxoPreviewRequest>(&request.json_utf8)?;
        serialize_json(&plan_utxo_preview(request)?)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_plan_utxo_spend_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing UTXO spend plan request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<UtxoSpendPlanRequest>(&request.json_utf8)?;
        serialize_json(&plan_utxo_spend(request)?)
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_merge_transactions_json(
    request: *const SpectraCoreJSONRequest,
) -> *mut SpectraJsonResponse {
    json_response_from_result((|| {
        if request.is_null() {
            return Err("Missing transaction merge request.".to_string());
        }
        let request = unsafe { &*request };
        let request = read_json_buffer::<TransactionMergeRequest>(&request.json_utf8)?;
        serialize_json(&merge_transactions(request))
    })())
}

#[no_mangle]
pub extern "C" fn spectra_core_json_response_free(response: *mut SpectraJsonResponse) {
    if response.is_null() {
        return;
    }

    unsafe {
        let response = Box::from_raw(response);
    free_buffer(response.payload_utf8);
    free_buffer(response.error_message_utf8);
}

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
    Ok(serialize_json(&legacy_wallet_store_to_core_state(&request_json)?)?)
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
}

fn json_response_from_result(result: Result<String, String>) -> *mut SpectraJsonResponse {
    match result {
        Ok(payload) => SpectraJsonResponse::success(payload),
        Err(message) => SpectraJsonResponse::error(message),
    }
}

fn serialize_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(display_error)
}

fn read_json_buffer<T: serde::de::DeserializeOwned>(buffer: &SpectraBuffer) -> Result<T, String> {
    serde_json::from_str(&read_buffer_to_string(buffer)?).map_err(display_error)
}

fn read_buffer_to_string(buffer: &SpectraBuffer) -> Result<String, String> {
    let bytes = read_buffer(buffer);
    std::str::from_utf8(bytes)
        .map(|value| value.to_string())
        .map_err(display_error)
}

fn read_buffer<'a>(buffer: &'a SpectraBuffer) -> &'a [u8] {
    if buffer.ptr.is_null() || buffer.len == 0 {
        return &[];
    }

    unsafe { slice::from_raw_parts(buffer.ptr.cast_const(), buffer.len) }
}

fn owned_buffer_from_string(value: String) -> SpectraBuffer {
    let mut bytes = value.into_bytes();
    let buffer = SpectraBuffer {
        ptr: bytes.as_mut_ptr(),
        len: bytes.len(),
    };
    std::mem::forget(bytes);
    buffer
}

fn empty_buffer() -> SpectraBuffer {
    SpectraBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    }
}

fn free_buffer(buffer: SpectraBuffer) {
    if buffer.ptr.is_null() || buffer.len == 0 {
        return;
    }

    unsafe {
        let _ = Vec::from_raw_parts(buffer.ptr, buffer.len, buffer.len);
    }
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
