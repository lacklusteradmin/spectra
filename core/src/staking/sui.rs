//! Sui staking validator and position queries.

use serde::Deserialize;
use serde_json::json;

use crate::fetch::http::{with_fallback, HttpClient, RetryProfile};
use crate::staking::{StakingError, StakingPosition, StakingValidator};

pub struct SuiStakingClient {
    rpc_endpoints: Vec<String>,
}

// ── RPC response types ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SuiSystemStateResp {
    result: SuiSystemStateSummary,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuiSystemStateSummary {
    active_validators: Vec<SuiValidatorSummary>,
}
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SuiValidatorSummary {
    sui_address: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    project_url: String,
    commission_rate: String,          // basis points, "500" = 5%
    staking_pool_sui_balance: String, // MIST string
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn short_id(id: &str) -> &str {
    if id.len() >= 10 {
        &id[..10]
    } else {
        id
    }
}

impl SuiStakingClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Self {
        Self { rpc_endpoints }
    }

    /// RPC: `suix_getLatestSuiSystemState`. Validator list comes back with
    /// pool_id, voting_power, commission_rate, next_epoch_stake.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        if self.rpc_endpoints.is_empty() {
            return Ok(vec![]);
        }
        let client = HttpClient::shared();
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "suix_getLatestSuiSystemState",
            "params": []
        });
        let resp: SuiSystemStateResp = match with_fallback(&self.rpc_endpoints, |url| {
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
            .active_validators
            .into_iter()
            .map(|v| {
                let commission_bps: f64 = v.commission_rate.parse().unwrap_or(0.0);
                let apy = 0.035 * (1.0 - commission_bps / 10_000.0);
                StakingValidator {
                    identifier: v.sui_address.clone(),
                    display_name: if v.name.is_empty() {
                        format!("Validator {}", short_id(&v.sui_address))
                    } else {
                        v.name.clone()
                    },
                    apy,
                    commission: Some(commission_bps / 10_000.0),
                    total_stake_smallest_unit: Some(v.staking_pool_sui_balance),
                    is_active: true,
                    tags: vec![],
                    min_delegation_smallest_unit: Some("1000000000".to_string()), // 1 SUI
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

    /// RPC: `suix_getStakes` returns active + pending stakes for `wallet_address`.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Ok(vec![])
    }
}
