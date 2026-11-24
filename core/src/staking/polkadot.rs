//! A live directory of Asset Hub nomination pools; no static validator list.
use super::StakingValidator;
use crate::api::error::ApiError;
use crate::api::substrate_json_rpc::SubstrateClient;
use std::sync::Arc;

pub(crate) async fn fetch_validators(
    endpoints: Vec<String>,
) -> Result<Vec<StakingValidator>, ApiError> {
    let pools = SubstrateClient::new(Arc::new(endpoints))
        .fetch_nomination_pools()
        .await?;
    Ok(pools
        .into_iter()
        .map(|pool| StakingValidator {
            identifier: pool.id.to_string(),
            display_name: pool.name,
            commission: pool.commission,
            total_stake_smallest_unit: Some(pool.active_balance.to_string()),
            is_active: pool.is_open,
            tags: vec!["nomination pool".into()],
            min_delegation_smallest_unit: Some(pool.minimum_join.to_string()),
            uptime_pct: None,
            website: None,
            description: None,
        })
        .collect())
}
