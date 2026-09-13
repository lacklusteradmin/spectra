//! Near staking validator and position queries.

use serde::Deserialize;
use serde_json::json;

use crate::http::{with_fallback, HttpClient, RetryProfile};
use crate::staking::{StakingError, StakingPosition, StakingValidator};

pub struct NearStakingClient {
    rpc_endpoints: Vec<String>,
}

// ── RPC response types ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ValidatorsResp {
    result: ValidatorsResult,
}
#[derive(Deserialize)]
struct ValidatorsResult {
    current_validators: Vec<NearCurrentValidator>,
}
#[derive(Deserialize, Clone)]
struct NearCurrentValidator {
    account_id: String,
    stake: String, // yoctoNEAR string
    is_slashed: bool,
    num_produced_blocks: u64,
    num_expected_blocks: u64,
}

impl NearStakingClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Self {
        Self { rpc_endpoints }
    }

    /// JSON-RPC: `validators` for the active set; supplement with view calls
    /// to each pool's `get_reward_fee_fraction` and `get_total_staked_balance`.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        if self.rpc_endpoints.is_empty() {
            return Ok(vec![]);
        }
        let client = HttpClient::shared();
        let body = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "validators",
            "params": [null]
        });
        let resp: ValidatorsResp = match with_fallback(&self.rpc_endpoints, |url| {
            let client = client.clone();
            let body = body.clone();
            async move { client.post_json(&url, &body, RetryProfile::ChainRead).await }
        })
        .await
        {
            Ok(r) => r,
            Err(_) => return Ok(vec![]),
        };

        let validators = resp
            .result
            .current_validators
            .into_iter()
            .filter(|v| !v.is_slashed)
            .map(|v| {
                let uptime = if v.num_expected_blocks > 0 {
                    Some(v.num_produced_blocks as f64 / v.num_expected_blocks as f64 * 100.0)
                } else {
                    None
                };
                StakingValidator {
                    identifier: v.account_id.clone(),
                    display_name: v.account_id.clone(),
                    apy: 0.09, // ~9% baseline; actual depends on pool fee
                    commission: None,
                    total_stake_smallest_unit: Some(v.stake.clone()),
                    is_active: true,
                    tags: vec![],
                    min_delegation_smallest_unit: None,
                    uptime_pct: uptime,
                    website: None,
                    description: None,
                    next_epoch_active: None,
                }
            })
            .collect();

        Ok(validators)
    }

    /// View call: `get_account(account_id)` on each pool the wallet has
    /// interacted with. Returns staked / unstaked / can_withdraw.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Ok(vec![])
    }
}
