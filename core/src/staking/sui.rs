//! Sui staking validator queries.

use crate::api::error::ApiError;
use crate::api::sui_json_rpc::SuiClient;
use crate::staking::StakingValidator;

/// RPC: `suix_getLatestSuiSystemState`. Validator list comes back with
/// pool_id, voting_power, commission_rate, next_epoch_stake.
pub(crate) async fn fetch_validators(
    rpc_endpoints: Vec<String>,
) -> Result<Vec<StakingValidator>, ApiError> {
    let resp = SuiClient::new(std::sync::Arc::new(rpc_endpoints))
        .fetch_staking_validators()
        .await?;

    let validators = resp
        .active_validators
        .into_iter()
        .map(|v| {
            let commission_bps: Option<f64> = v.commission_rate.parse().ok();
            StakingValidator {
                identifier: v.sui_address.clone(),
                display_name: if v.name.is_empty() {
                    format!("Validator {}", super::short_id(&v.sui_address))
                } else {
                    v.name.clone()
                },
                commission: commission_bps
                    .filter(|v| v.is_finite() && (0.0..=10_000.0).contains(v))
                    .map(|v| v / 10_000.0),
                total_stake_smallest_unit: Some(v.staking_pool_sui_balance),
                is_active: true,
                tags: vec![],
                min_delegation_smallest_unit: crate::registry::Chain::Sui
                    .sui_staking_minimum()
                    .ok()
                    .map(|v| v.to_string()),
                uptime_pct: None,
                website: if v.project_url.is_empty() {
                    None
                } else {
                    Some(v.project_url)
                },
                description: if v.description.is_empty() {
                    None
                } else {
                    Some(v.description)
                },
            }
        })
        .collect();

    Ok(validators)
}
