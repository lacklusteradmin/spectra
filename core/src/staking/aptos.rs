//! Aptos staking validator queries.

use crate::api::aptos_rest::AptosClient;
use crate::api::error::ApiError;
use crate::staking::StakingValidator;

/// Only active validators with an actual on-chain delegation pool.
pub(crate) async fn fetch_validators(
    rest_endpoints: Vec<String>,
) -> Result<Vec<StakingValidator>, ApiError> {
    use futures::{StreamExt, TryStreamExt, stream};
    let client = std::sync::Arc::new(AptosClient::new(std::sync::Arc::new(rest_endpoints)));
    let pool_addrs = client.fetch_staking_validators().await?;
    let validators = stream::iter(pool_addrs)
        .map(|validator| {
            let client = client.clone();
            async move {
                let Some(commission) = client.fetch_delegation_commission(&validator.addr).await?
                else {
                    return Ok::<_, ApiError>(None);
                };
                Ok(Some(StakingValidator {
                    display_name: format!("Delegation pool {}", super::short_id(&validator.addr)),
                    identifier: validator.addr,
                    commission: Some(commission as f64 / 10_000.0),
                    total_stake_smallest_unit: Some(validator.voting_power),
                    is_active: true,
                    tags: vec![],
                    min_delegation_smallest_unit: crate::registry::Chain::Aptos
                        .aptos_delegation_minimum()
                        .ok()
                        .map(|v| v.to_string()),
                    uptime_pct: None,
                    website: None,
                    description: None,
                }))
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect();

    Ok(validators)
}
