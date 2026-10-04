//! Sui staking validator queries.

use crate::api::sui_json_rpc::SuiClient;
use crate::staking::{StakingError, StakingValidator};

pub struct SuiStakingClient {
    rpc_endpoints: Vec<String>,
}

// ── RPC response types ────────────────────────────────────────────────────────

// ── Helpers ───────────────────────────────────────────────────────────────────

fn short_id(id: &str) -> &str {
    if id.len() >= 10 { &id[..10] } else { id }
}

impl SuiStakingClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Self {
        Self { rpc_endpoints }
    }

    /// RPC: `suix_getLatestSuiSystemState`. Validator list comes back with
    /// pool_id, voting_power, commission_rate, next_epoch_stake.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        let resp = SuiClient::new(std::sync::Arc::new(self.rpc_endpoints.clone()))
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
                        format!("Validator {}", short_id(&v.sui_address))
                    } else {
                        v.name.clone()
                    },
                    apy: None,
                    commission: commission_bps
                        .filter(|v| v.is_finite() && (0.0..=10_000.0).contains(v))
                        .map(|v| v / 10_000.0),
                    total_stake_smallest_unit: Some(v.staking_pool_sui_balance),
                    is_active: true,
                    tags: vec![],
                    min_delegation_smallest_unit: crate::registry::Chain::Sui
                        .sui_staking_minimum()
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
                    next_epoch_active: None,
                }
            })
            .collect();

        Ok(validators)
    }
}
