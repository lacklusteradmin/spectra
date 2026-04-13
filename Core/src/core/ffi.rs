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

/// Evaluate high-risk warning reasons for a pending send transaction.
///
/// Input JSON fields:
///   chain_name, symbol, amount, holding_amount,
///   destination_address, destination_input, used_ens_resolution,
///   wallet_selected_chain,
///   address_book: [{ chain_name, address }],
///   tx_addresses: [{ chain_name, address }]
///
/// Output: JSON array of warning objects, e.g.:
///   [{"code":"invalid_format","chain":"Ethereum"}, {"code":"new_address"}, ...]
#[uniffi::export]
pub fn core_evaluate_high_risk_send_reasons_json(
    request_json: String,
) -> Result<String, crate::SpectraBridgeError> {
    let req: serde_json::Value = serde_json::from_str(&request_json)
        .map_err(|e| crate::SpectraBridgeError::from(e.to_string()))?;

    let chain_name = req["chain_name"].as_str().unwrap_or("");
    let symbol = req["symbol"].as_str().unwrap_or("");
    let amount = req["amount"].as_f64().unwrap_or(0.0);
    let holding_amount = req["holding_amount"].as_f64().unwrap_or(0.0);
    let destination_address = req["destination_address"].as_str().unwrap_or("");
    let destination_input = req["destination_input"].as_str().unwrap_or("");
    let used_ens_resolution = req["used_ens_resolution"].as_bool().unwrap_or(false);
    let wallet_selected_chain = req["wallet_selected_chain"].as_str().unwrap_or("");

    let mut warnings: Vec<serde_json::Value> = Vec::new();

    // 1. Address format validation (already in Rust — no extra cost).
    if !hrsr_validate_address(chain_name, destination_address) {
        warnings.push(serde_json::json!({ "code": "invalid_format", "chain": chain_name }));
    }

    // Normalize destination for case-insensitive comparison.
    let norm_dest = hrsr_normalize_address(destination_address, chain_name).to_lowercase();

    // 2. New address detection.
    let empty_arr: Vec<serde_json::Value> = Vec::new();
    let address_book = req["address_book"].as_array().unwrap_or(&empty_arr);
    let has_address_book = address_book.iter().any(|e| {
        e["chain_name"].as_str() == Some(chain_name)
            && hrsr_normalize_address(e["address"].as_str().unwrap_or(""), chain_name)
                .to_lowercase()
                == norm_dest
    });
    let tx_addresses = req["tx_addresses"].as_array().unwrap_or(&empty_arr);
    let has_tx_history = tx_addresses.iter().any(|e| {
        e["chain_name"].as_str() == Some(chain_name)
            && hrsr_normalize_address(e["address"].as_str().unwrap_or(""), chain_name)
                .to_lowercase()
                == norm_dest
    });
    if !has_address_book && !has_tx_history {
        warnings.push(serde_json::json!({ "code": "new_address" }));
    }

    // 3. ENS resolution warning.
    if used_ens_resolution {
        warnings.push(serde_json::json!({
            "code": "ens_resolved",
            "name": destination_input,
            "address": destination_address
        }));
    }

    // 4. Large send percentage (≥25 % of holding balance).
    if holding_amount > 0.0 {
        let ratio = amount / holding_amount;
        if ratio >= 0.25 {
            let pct = (ratio * 100.0).round() as u64;
            warnings.push(serde_json::json!({
                "code": "large_send", "percent": pct, "symbol": symbol
            }));
        }
    }

    // 5-10. Cross-chain prefix mismatch checks.
    let lowered = destination_input.to_lowercase();
    let is_evm = matches!(
        chain_name,
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism"
            | "BNB Chain" | "Avalanche" | "Hyperliquid"
    );
    let is_l2 = matches!(
        chain_name,
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
            warnings.push(serde_json::json!({ "code": "non_evm_on_evm", "chain": chain_name }));
        }
        if is_l2 && is_ens_candidate {
            warnings.push(serde_json::json!({ "code": "ens_on_l2", "chain": chain_name }));
        }
    } else if matches!(chain_name, "Bitcoin" | "Bitcoin Cash" | "Litecoin" | "Dogecoin") {
        if lowered.starts_with("0x") || is_ens_candidate {
            warnings.push(serde_json::json!({ "code": "eth_on_utxo", "chain": chain_name }));
        }
    } else if chain_name == "Tron" {
        if lowered.starts_with("0x") || lowered.starts_with("bc1") {
            warnings.push(serde_json::json!({ "code": "non_tron" }));
        }
    } else if chain_name == "Solana" {
        if lowered.starts_with("0x")
            || lowered.starts_with("bc1")
            || lowered.starts_with("ltc1")
            || lowered.starts_with('t')
        {
            warnings.push(serde_json::json!({ "code": "non_solana" }));
        }
    } else if chain_name == "XRP Ledger" {
        if lowered.starts_with("0x")
            || lowered.starts_with("bc1")
            || lowered.starts_with('t')
        {
            warnings.push(serde_json::json!({ "code": "non_xrp" }));
        }
    } else if chain_name == "Monero" {
        if lowered.starts_with("0x")
            || lowered.starts_with("bc1")
            || lowered.starts_with('r')
        {
            warnings.push(serde_json::json!({ "code": "non_monero" }));
        }
    }

    // 11. Wallet-chain context mismatch.
    if !wallet_selected_chain.is_empty() && wallet_selected_chain != chain_name {
        warnings.push(serde_json::json!({ "code": "chain_mismatch" }));
    }

    Ok(serde_json::to_string(&warnings)
        .map_err(|e| crate::SpectraBridgeError::from(e.to_string()))?)
}

/// Validate a destination address for the given chain name using the Rust
/// address-validation logic already used by `core_validate_address_json`.
fn hrsr_validate_address(chain_name: &str, address: &str) -> bool {
    let kind = match chain_name {
        "Bitcoin" => "bitcoin",
        "Bitcoin Cash" => "bitcoinCash",
        "Bitcoin SV" => "bitcoinSV",
        "Litecoin" => "litecoin",
        "Dogecoin" => "dogecoin",
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism"
        | "BNB Chain" | "Avalanche" | "Hyperliquid" => "evm",
        "Tron" => "tron",
        "Solana" => "solana",
        "Cardano" => "cardano",
        "XRP Ledger" => "xrp",
        "Stellar" => "stellar",
        "Monero" => "monero",
        "Sui" => "sui",
        "Aptos" => "aptos",
        "TON" => "ton",
        "Internet Computer" => "internetComputer",
        "NEAR" => "near",
        "Polkadot" => "polkadot",
        _ => return false,
    };
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: address.to_string(),
        network_mode: None,
    })
    .is_valid
}

/// Normalize an address for case-insensitive comparison, mirroring the Swift
/// `normalizedAddress(_:for:)` logic in `WalletStore+SendFlow.swift`.
fn hrsr_normalize_address(address: &str, chain_name: &str) -> String {
    let t = address.trim().to_string();
    let is_evm = matches!(
        chain_name,
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism"
            | "BNB Chain" | "Avalanche" | "Hyperliquid"
    );
    if is_evm {
        return t.to_lowercase();
    }
    if matches!(chain_name, "Sui" | "Aptos") {
        let l = t.to_lowercase();
        return if l.starts_with("0x") { l } else { format!("0x{l}") };
    }
    if matches!(chain_name, "Internet Computer" | "NEAR") {
        return t.to_lowercase();
    }
    t
}

fn serialize_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(display_error)
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
