//! Near staking validator queries.

use crate::api::near_json_rpc::NearClient;
use crate::staking::{StakingError, StakingValidator};

pub struct NearStakingClient {
    rpc_endpoints: Vec<String>,
}

// ── RPC response types ────────────────────────────────────────────────────────

impl NearStakingClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Self {
        Self { rpc_endpoints }
    }

    /// Active validators with their stake and observed block production.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        use futures::{StreamExt, TryStreamExt, stream};
        let client = std::sync::Arc::new(NearClient::new(std::sync::Arc::new(
            self.rpc_endpoints.clone(),
        )));
        let resp = client.fetch_staking_validators().await?;
        let validators = stream::iter(
            resp.current_validators
                .into_iter()
                .filter(|v| !v.is_slashed),
        )
        .map(|v| {
            let client = client.clone();
            async move {
                if !client
                    .staking_pool_approved(crate::registry::Chain::Near, &v.account_id)
                    .await?
                {
                    return Ok::<_, crate::api::error::ApiError>(None);
                };
                let pool = client
                    .fetch_staking_pool(crate::registry::Chain::Near, &v.account_id, &v.account_id)
                    .await?;
                let uptime = (v.num_expected_blocks > 0)
                    .then(|| v.num_produced_blocks as f64 / v.num_expected_blocks as f64 * 100.0);
                Ok(Some(StakingValidator {
                    identifier: v.account_id.clone(),
                    display_name: v.account_id,
                    commission: Some(pool.commission),
                    total_stake_smallest_unit: Some(v.stake),
                    is_active: !pool.paused,
                    tags: vec![],
                    min_delegation_smallest_unit: None,
                    uptime_pct: uptime,
                    website: None,
                    description: Some(format!("Pool owner: {}", pool.owner_id)),
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
