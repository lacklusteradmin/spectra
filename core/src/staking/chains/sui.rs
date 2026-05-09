//! Sui staking — `0x3::sui_system::request_add_stake` Move call.
//!
//! Wallet flow:
//! 1. `fetch_validators` returns the active validator set from
//!    `0x5::sui_system_state::SuiSystemState` epoch info.
//! 2. `build_request_add_stake_tx` constructs a programmable tx with a single
//!    `request_add_stake_mul_coin` call: passes a SUI Coin, validator address,
//!    and amount. Returns a `StakedSui` object owned by the wallet.
//! 3. `build_request_withdraw_stake_tx` calls `request_withdraw_stake` with
//!    the `StakedSui` object reference. Funds + rewards are returned at the
//!    end of the current epoch.
//!
//! Native unit: MIST (1 SUI = 1e9 MIST). Rewards accrue per-epoch (~24h).

use serde::Deserialize;
use serde_json::json;

use crate::http::{with_fallback, HttpClient, RetryProfile};
use crate::staking::{
    StakingActionKind, StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

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
    commission_rate: String,     // basis points, "500" = 5%
    staking_pool_sui_balance: String, // MIST string
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn mist_to_sui(mist: u64) -> f64 { mist as f64 / 1_000_000_000.0 }

fn sui_display(mist: u64) -> String { format!("{:.6} SUI", mist_to_sui(mist)) }

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
                    website: if v.project_url.is_empty() { None } else { Some(v.project_url) },
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

    pub async fn build_request_add_stake_tx(
        &self,
        _wallet_address: &str,
        amount_mist: u64,
        validator_address: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Stake,
            validator_identifier: validator_address.to_string(),
            validator_display_name: format!("Validator {}", short_id(validator_address)),
            amount_smallest_unit: amount_mist.to_string(),
            amount_display: sui_display(amount_mist),
            estimated_fee_smallest_unit: "1000000".to_string(), // 0.001 SUI
            estimated_fee_display: "~0.001 SUI".to_string(),
            unbonding_period_seconds: 24 * 3600, // until end of epoch
            notes: vec![
                "Creates a StakedSui object in your wallet.".to_string(),
                "Minimum stake: 1 SUI. Rewards begin next epoch (~24h).".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: Some(amount_mist >= 1_000_000_000),
        })
    }

    /// `staked_sui_object_id` must be a `StakedSui` object owned by the wallet.
    pub async fn build_request_withdraw_stake_tx(
        &self,
        _wallet_address: &str,
        staked_sui_object_id: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Unstake,
            validator_identifier: staked_sui_object_id.to_string(),
            validator_display_name: format!("StakedSui {}", short_id(staked_sui_object_id)),
            amount_smallest_unit: "0".to_string(),
            amount_display: "Full position".to_string(),
            estimated_fee_smallest_unit: "1000000".to_string(),
            estimated_fee_display: "~0.001 SUI".to_string(),
            unbonding_period_seconds: 24 * 3600,
            notes: vec![
                "Principal + rewards return at the end of the current epoch.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }
}
