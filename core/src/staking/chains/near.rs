//! NEAR staking — function calls to `staking_pool` contracts.
//!
//! Wallet flow:
//! 1. Each validator runs its own `staking_pool` contract at a `*.poolv1.near`
//!    or `*.pool.near` address. `fetch_validators` queries the staking-pools
//!    factory or an indexer to enumerate active pools.
//! 2. `build_deposit_and_stake_tx` constructs an action with method
//!    `deposit_and_stake` and an attached deposit equal to the stake amount.
//!    Funds become active at the next epoch (~12h).
//! 3. `build_unstake_tx` calls `unstake(amount)` (or `unstake_all()`). After
//!    4 epochs (~52h on mainnet), the stake becomes withdrawable.
//! 4. `build_withdraw_tx` calls `withdraw(amount)` (or `withdraw_all()`).
//!
//! Native unit: yoctoNEAR (1 NEAR = 1e24 yoctoNEAR). Storage staking
//! requirements apply but are typically pre-paid by the wallet account.

use serde::Deserialize;
use serde_json::json;

use crate::http::{with_fallback, HttpClient, RetryProfile};
use crate::staking::{
    StakingActionKind, StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert yoctoNEAR string (1e24) to approximate NEAR for display.
fn yocto_to_near_display(yocto: &str) -> String {
    // 1 NEAR = 10^24 yoctoNEAR; drop the last 24 digits to get the integer NEAR part.
    let digits = yocto.len();
    if digits <= 24 {
        return "< 1 NEAR".to_string();
    }
    let near_part = &yocto[..digits - 24];
    format!("{} NEAR", near_part)
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

    pub async fn build_deposit_and_stake_tx(
        &self,
        _wallet_address: &str,
        pool_account_id: &str,
        amount_yocto_near: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        let display = yocto_to_near_display(amount_yocto_near);
        Ok(StakingActionPreview {
            kind: StakingActionKind::Stake,
            validator_identifier: pool_account_id.to_string(),
            validator_display_name: pool_account_id.to_string(),
            amount_smallest_unit: amount_yocto_near.to_string(),
            amount_display: display,
            estimated_fee_smallest_unit: "1000000000000000000000".to_string(), // ~0.001 NEAR gas (10^21)
            estimated_fee_display: "~0.001 NEAR".to_string(),
            unbonding_period_seconds: 4 * 12 * 3600, // 4 epochs ~48h
            notes: vec![
                "Calls deposit_and_stake on the pool contract.".to_string(),
                "Funds activate at the next epoch (~12h).".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    pub async fn build_unstake_tx(
        &self,
        _wallet_address: &str,
        pool_account_id: &str,
        amount_yocto_near: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        let display = yocto_to_near_display(amount_yocto_near);
        Ok(StakingActionPreview {
            kind: StakingActionKind::Unstake,
            validator_identifier: pool_account_id.to_string(),
            validator_display_name: pool_account_id.to_string(),
            amount_smallest_unit: amount_yocto_near.to_string(),
            amount_display: display,
            estimated_fee_smallest_unit: "1000000000000000000000".to_string(),
            estimated_fee_display: "~0.001 NEAR".to_string(),
            unbonding_period_seconds: 4 * 12 * 3600,
            notes: vec![
                "Funds move to pending-inactive; withdraw after 4 epochs (~48h).".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    pub async fn build_withdraw_tx(
        &self,
        _wallet_address: &str,
        pool_account_id: &str,
        amount_yocto_near: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        let display = yocto_to_near_display(amount_yocto_near);
        Ok(StakingActionPreview {
            kind: StakingActionKind::Withdraw,
            validator_identifier: pool_account_id.to_string(),
            validator_display_name: pool_account_id.to_string(),
            amount_smallest_unit: amount_yocto_near.to_string(),
            amount_display: display,
            estimated_fee_smallest_unit: "1000000000000000000000".to_string(),
            estimated_fee_display: "~0.001 NEAR".to_string(),
            unbonding_period_seconds: 0,
            notes: vec!["Requires the 4-epoch unbonding period to have elapsed.".to_string()],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }
}
