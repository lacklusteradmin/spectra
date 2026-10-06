//! Aptos staking validator queries.

use crate::api::aptos_rest::AptosClient;
use crate::staking::{StakingError, StakingValidator};

pub struct AptosStakingClient {
    rest_endpoints: Vec<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn short_id(id: &str) -> &str {
    if id.len() >= 10 { &id[..10] } else { id }
}

impl AptosStakingClient {
    pub fn new(rest_endpoints: Vec<String>) -> Self {
        Self { rest_endpoints }
    }

    /// Only active validators with an actual on-chain delegation pool.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        use futures::{StreamExt, TryStreamExt, stream};
        let client = std::sync::Arc::new(AptosClient::new(std::sync::Arc::new(
            self.rest_endpoints.clone(),
        )));
        let pool_addrs = client.fetch_staking_validators().await?;
        let validators = stream::iter(pool_addrs)
            .map(|validator| {
                let client = client.clone();
                async move {
                    let Some(commission) =
                        client.fetch_delegation_commission(&validator.addr).await?
                    else {
                        return Ok::<_, crate::api::error::ApiError>(None);
                    };
                    Ok(Some(StakingValidator {
                        display_name: format!("Delegation pool {}", short_id(&validator.addr)),
                        identifier: validator.addr,
                        commission: Some(commission as f64 / 10_000.0),
                        total_stake_smallest_unit: Some(validator.voting_power),
                        is_active: true,
                        tags: vec![],
                        min_delegation_smallest_unit: crate::registry::Chain::Aptos
                            .aptos_delegation_minimum()
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
}
