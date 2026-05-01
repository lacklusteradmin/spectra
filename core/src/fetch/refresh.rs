use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMaintenancePlanRequest {
    pub now_unix: f64,
    pub last_pending_transaction_refresh_at_unix: Option<f64>,
    pub last_live_price_refresh_at_unix: Option<f64>,
    pub has_pending_transaction_maintenance_work: bool,
    pub should_run_scheduled_price_refresh: bool,
    pub pending_refresh_interval: f64,
    pub price_refresh_interval: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMaintenancePlan {
    pub refresh_pending_transactions: bool,
    pub refresh_live_prices: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundMaintenanceRequest {
    pub now_unix: f64,
    pub is_network_reachable: bool,
    pub last_background_maintenance_at_unix: Option<f64>,
    pub interval: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainRefreshPlanRequest {
    pub chain_ids: Vec<String>,
    pub now_unix: f64,
    pub force_chain_refresh: bool,
    pub include_history_refreshes: bool,
    pub history_refresh_interval: f64,
    pub pending_transaction_maintenance_chain_ids: Vec<String>,
    pub degraded_chain_ids: Vec<String>,
    pub last_good_chain_sync_by_id: HashMap<String, f64>,
    pub last_history_refresh_at_by_chain_id: HashMap<String, f64>,
    pub automatic_chain_refresh_staleness_interval: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainRefreshPlan {
    pub chain_id: String,
    pub chain_name: String,
    pub refresh_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRefreshPlanRequest {
    pub chain_ids: Vec<String>,
    pub now_unix: f64,
    pub interval: f64,
    pub last_history_refresh_at_by_chain_id: HashMap<String, f64>,
}

pub fn active_maintenance_plan(request: ActiveMaintenancePlanRequest) -> ActiveMaintenancePlan {
    let refresh_pending_transactions = match request.last_pending_transaction_refresh_at_unix {
        Some(last_refresh_at) => {
            request.has_pending_transaction_maintenance_work
                && request.now_unix - last_refresh_at >= request.pending_refresh_interval
        }
        None => request.has_pending_transaction_maintenance_work,
    };

    let refresh_live_prices = match request.last_live_price_refresh_at_unix {
        Some(last_refresh_at) => {
            request.should_run_scheduled_price_refresh
                && request.now_unix - last_refresh_at >= request.price_refresh_interval
        }
        None => request.should_run_scheduled_price_refresh,
    };

    ActiveMaintenancePlan {
        refresh_pending_transactions,
        refresh_live_prices,
    }
}

pub fn should_run_background_maintenance(request: BackgroundMaintenanceRequest) -> bool {
    if !request.is_network_reachable {
        return false;
    }
    match request.last_background_maintenance_at_unix {
        Some(last_run_at) => request.now_unix - last_run_at >= request.interval,
        None => true,
    }
}

pub fn chain_plans(request: ChainRefreshPlanRequest) -> Vec<ChainRefreshPlan> {
    let pending_ids = request
        .pending_transaction_maintenance_chain_ids
        .into_iter()
        .collect::<BTreeSet<_>>();
    let degraded_ids = request
        .degraded_chain_ids
        .into_iter()
        .collect::<BTreeSet<_>>();

    let mut chain_ids = request.chain_ids;
    chain_ids
        .sort_by(|lhs, rhs| display_name_for_chain_id(lhs).cmp(display_name_for_chain_id(rhs)));

    chain_ids
        .into_iter()
        .filter(|chain_id| {
            if request.force_chain_refresh {
                return true;
            }
            if pending_ids.contains(chain_id) || degraded_ids.contains(chain_id) {
                return true;
            }
            match request.last_good_chain_sync_by_id.get(chain_id) {
                Some(last_sync_at) => {
                    request.now_unix - last_sync_at
                        >= request.automatic_chain_refresh_staleness_interval
                }
                None => true,
            }
        })
        .map(|chain_id| {
            let refresh_history = request.include_history_refreshes
                && should_refresh_history(
                    &chain_id,
                    request.now_unix,
                    request.history_refresh_interval,
                    &request.last_history_refresh_at_by_chain_id,
                );
            ChainRefreshPlan {
                chain_name: display_name_for_chain_id(&chain_id).to_string(),
                chain_id,
                refresh_history,
            }
        })
        .collect()
}

pub fn history_plans(request: HistoryRefreshPlanRequest) -> Vec<String> {
    let mut chain_ids = request.chain_ids;
    chain_ids
        .sort_by(|lhs, rhs| display_name_for_chain_id(lhs).cmp(display_name_for_chain_id(rhs)));
    chain_ids
        .into_iter()
        .filter(|chain_id| {
            should_refresh_history(
                chain_id,
                request.now_unix,
                request.interval,
                &request.last_history_refresh_at_by_chain_id,
            )
        })
        .collect()
}

fn should_refresh_history(
    chain_id: &str,
    now_unix: f64,
    interval: f64,
    last_history_refresh_at_by_chain_id: &HashMap<String, f64>,
) -> bool {
    match last_history_refresh_at_by_chain_id.get(chain_id) {
        Some(last_refresh_at) => now_unix - last_refresh_at >= interval,
        None => true,
    }
}

fn display_name_for_chain_id(chain_id: &str) -> &str {
    match chain_id {
        "bitcoin" => "Bitcoin",
        "bitcoincash" | "bitcoin-cash" => "Bitcoin Cash",
        "bitcoinsv" | "bitcoin-sv" => "Bitcoin SV",
        "litecoin" => "Litecoin",
        "dogecoin" => "Dogecoin",
        "ethereum" => "Ethereum",
        "ethereumclassic" | "ethereum-classic" => "Ethereum Classic",
        "arbitrum" => "Arbitrum",
        "optimism" => "Optimism",
        "bnb" => "BNB Chain",
        "avalanche" => "Avalanche",
        "hyperliquid" => "Hyperliquid",
        "tron" => "Tron",
        "solana" => "Solana",
        "cardano" => "Cardano",
        "xrp" => "XRP Ledger",
        "stellar" => "Stellar",
        "monero" => "Monero",
        "sui" => "Sui",
        "aptos" => "Aptos",
        "ton" => "TON",
        "icp" => "Internet Computer",
        "near" => "NEAR",
        "polkadot" => "Polkadot",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn computes_chain_plans() {
        let plans = chain_plans(ChainRefreshPlanRequest {
            chain_ids: vec![
                "ethereum".to_string(),
                "solana".to_string(),
                "bitcoin".to_string(),
            ],
            now_unix: 1_000.0,
            force_chain_refresh: false,
            include_history_refreshes: true,
            history_refresh_interval: 300.0,
            pending_transaction_maintenance_chain_ids: vec!["ethereum".to_string()],
            degraded_chain_ids: vec!["solana".to_string()],
            last_good_chain_sync_by_id: HashMap::from([
                ("ethereum".to_string(), 1_000.0),
                ("solana".to_string(), 1_000.0),
                ("bitcoin".to_string(), 1_000.0),
            ]),
            last_history_refresh_at_by_chain_id: HashMap::from([
                ("ethereum".to_string(), 200.0),
                ("solana".to_string(), 900.0),
            ]),
            automatic_chain_refresh_staleness_interval: 86_400.0,
        });

        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].chain_name, "Ethereum");
        assert!(plans[0].refresh_history);
        assert_eq!(plans[1].chain_name, "Solana");
        assert!(!plans[1].refresh_history);
    }
}

// ── FFI surface (relocated from ffi.rs) ──────────────────────────────────

#[uniffi::export]
pub fn core_active_maintenance_plan(
    request: ActiveMaintenancePlanRequest,
) -> ActiveMaintenancePlan {
    active_maintenance_plan(request)
}

#[uniffi::export]
pub fn core_should_run_background_maintenance(request: BackgroundMaintenanceRequest) -> bool {
    should_run_background_maintenance(request)
}

#[uniffi::export]
pub fn core_chain_refresh_plans(request: ChainRefreshPlanRequest) -> Vec<ChainRefreshPlan> {
    chain_plans(request)
}

#[uniffi::export]
pub fn core_history_refresh_plans(request: HistoryRefreshPlanRequest) -> Vec<String> {
    history_plans(request)
}
