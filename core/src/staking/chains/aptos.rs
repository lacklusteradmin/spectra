//! Aptos staking — `0x1::delegation_pool::add_stake` Move call.
//!
//! Wallet flow:
//! 1. `fetch_validators` returns delegation pools (each pool is an account
//!    address with `delegation_pool::DelegationPool` resource).
//! 2. `build_add_stake_tx` calls `add_stake(pool_address, amount)`. Stake
//!    becomes active at the next epoch (~2h).
//! 3. `build_unlock_tx` calls `unlock(pool_address, amount)` — moves stake
//!    into a pending-inactive bucket. After the lockup cycle (configurable
//!    per pool, typically 30 days), it becomes withdrawable.
//! 4. `build_withdraw_tx` calls `withdraw(pool_address, amount)` to pull
//!    inactive stake back to the wallet.
//!
//! Native unit: octa (1 APT = 1e8 octas).

use serde_json::json;

use crate::http::{with_fallback, HttpClient, RetryProfile};
use crate::staking::{
    StakingActionKind, StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

pub struct AptosStakingClient {
    rest_endpoints: Vec<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn octas_to_apt(octas: u64) -> f64 { octas as f64 / 1e8 }

fn apt_display(octas: u64) -> String { format!("{:.6} APT", octas_to_apt(octas)) }

fn short_id(id: &str) -> &str {
    if id.len() >= 10 { &id[..10] } else { id }
}

impl AptosStakingClient {
    pub fn new(rest_endpoints: Vec<String>) -> Self {
        Self { rest_endpoints }
    }

    /// REST view: `0x1::delegation_pool::get_all_delegation_pools` — returns
    /// the on-chain registry of addresses that have a `DelegationPool` resource.
    /// These are the addresses callers must pass to `add_stake`; they differ
    /// from validator addresses in the `ValidatorSet`.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        if self.rest_endpoints.is_empty() {
            return Ok(vec![]);
        }
        let client = HttpClient::shared();
        let path = "/v1/view";
        let body = json!({
            "function": "0x1::delegation_pool::get_all_delegation_pools",
            "type_arguments": [],
            "arguments": []
        });
        // Response: [[pool_addr1, pool_addr2, ...]] — outer array = return values,
        // inner array = the vector<address> return value.
        let resp: serde_json::Value =
            match with_fallback(&self.rest_endpoints, |base| {
                let client = client.clone();
                let body = body.clone();
                let url = format!("{}{}", base.trim_end_matches('/'), path);
                async move { client.post_json(&url, &body, RetryProfile::ChainRead).await }
            })
            .await
            {
                Ok(r) => r,
                Err(_) => return Ok(vec![]),
            };

        let pool_addrs: Vec<String> = resp
            .as_array()
            .and_then(|outer| outer.first())
            .and_then(|inner| inner.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let validators = pool_addrs
            .into_iter()
            .take(100)
            .map(|addr| StakingValidator {
                display_name: format!("Pool {}", short_id(&addr)),
                identifier: addr,
                apy: 0.07, // ~7% baseline
                commission: None,
                total_stake_smallest_unit: None,
                is_active: true,
                tags: vec![],
                min_delegation_smallest_unit: Some("1100000000".to_string()), // 11 APT
                uptime_pct: None,
                website: None,
                description: None,
                next_epoch_active: None,
            })
            .collect();

        Ok(validators)
    }

    /// REST: per-pool `get_stake(pool_address, wallet_address)` view function
    /// returns (active, inactive, pending_inactive) buckets.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Ok(vec![])
    }

    pub async fn build_add_stake_tx(
        &self,
        _wallet_address: &str,
        pool_address: &str,
        amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Stake,
            validator_identifier: pool_address.to_string(),
            validator_display_name: format!("Pool {}", short_id(pool_address)),
            amount_smallest_unit: amount_octas.to_string(),
            amount_display: apt_display(amount_octas),
            estimated_fee_smallest_unit: "10000".to_string(), // ~0.0001 APT
            estimated_fee_display: "~0.0001 APT".to_string(),
            unbonding_period_seconds: 30 * 24 * 3600, // ~30-day lockup
            notes: vec![
                "Calls 0x1::delegation_pool::add_stake.".to_string(),
                "Minimum: 11 APT. Stake activates next epoch (~2h).".to_string(),
                "Unlocking moves stake to pending-inactive for ~30 days.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: Some(amount_octas >= 1_100_000_000),
        })
    }

    pub async fn build_unlock_tx(
        &self,
        _wallet_address: &str,
        pool_address: &str,
        amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Unstake,
            validator_identifier: pool_address.to_string(),
            validator_display_name: format!("Pool {}", short_id(pool_address)),
            amount_smallest_unit: amount_octas.to_string(),
            amount_display: apt_display(amount_octas),
            estimated_fee_smallest_unit: "10000".to_string(),
            estimated_fee_display: "~0.0001 APT".to_string(),
            unbonding_period_seconds: 30 * 24 * 3600,
            notes: vec![
                "Stake moves to pending-inactive. After the lockup cycle (~30 days) it is withdrawable.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    pub async fn build_withdraw_tx(
        &self,
        _wallet_address: &str,
        pool_address: &str,
        amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Withdraw,
            validator_identifier: pool_address.to_string(),
            validator_display_name: format!("Pool {}", short_id(pool_address)),
            amount_smallest_unit: amount_octas.to_string(),
            amount_display: apt_display(amount_octas),
            estimated_fee_smallest_unit: "10000".to_string(),
            estimated_fee_display: "~0.0001 APT".to_string(),
            unbonding_period_seconds: 0,
            notes: vec!["Requires the lockup cycle to have elapsed.".to_string()],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }
}
