//! Sui staking validator queries.

use crate::api::error::ApiError;
use crate::api::sui_graphql::SuiClient;
use crate::staking::StakingValidator;

/// The current epoch's active validators.
pub(crate) async fn fetch_validators(
    rpc_endpoints: Vec<String>,
) -> Result<Vec<StakingValidator>, ApiError> {
    let validators = SuiClient::new(std::sync::Arc::new(rpc_endpoints))
        .fetch_staking_validators()
        .await?
        .into_iter()
        .map(|v| StakingValidator {
            identifier: v.sui_address.clone(),
            display_name: if v.name.is_empty() {
                format!("Validator {}", super::short_id(&v.sui_address))
            } else {
                v.name.clone()
            },
            commission: (v.commission_rate <= 10_000).then(|| v.commission_rate as f64 / 10_000.0),
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
        })
        .collect();

    Ok(validators)
}
