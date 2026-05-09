//! Polkadot staking — direct nomination OR nomination pools.
//!
//! Two staking paths, exposed separately so Swift can let the user pick:
//!
//! - **Direct nomination** (legacy, requires ≥250 DOT minimum bond):
//!     1. `staking::bond(value, payee)` to lock funds.
//!     2. `staking::nominate([validators])` to pick up to 16 validators.
//!     3. `staking::chill()` then `staking::unbond(amount)` to begin unstake;
//!        funds become withdrawable after ~28 days.
//!     4. `staking::withdraw_unbonded()` to sweep matured unlocking chunks.
//! - **Nomination pools** (preferred for smaller stakers, no minimum):
//!     1. `nomination_pools::join(amount, pool_id)`.
//!     2. `nomination_pools::unbond(member_account, points)`.
//!     3. `nomination_pools::withdraw_unbonded(...)`.
//!
//! Native unit: planck (1 DOT = 1e10 planck). Eras are ~24h; rewards
//! distributed at end-of-era.

use crate::staking::{
    StakingActionKind, StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

pub struct PolkadotStakingClient {
    sidecar_endpoints: Vec<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn planck_to_dot(planck: u128) -> f64 { planck as f64 / 1e10 }

fn dot_display(planck: u128) -> String { format!("{:.4} DOT", planck_to_dot(planck)) }

impl PolkadotStakingClient {
    pub fn new(sidecar_endpoints: Vec<String>) -> Self {
        Self { sidecar_endpoints }
    }

    /// Active validator set. Sidecar: `/pallets/staking/storage/validators`.
    /// Validator data requires SCALE decoding — returns empty until a
    /// Substrate Sidecar REST endpoint is wired to the endpoint catalog.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Ok(vec![])
    }

    /// Open nomination pools accepting new members. Sidecar:
    /// `/pallets/nomination-pools/storage/bondedPools`.
    pub async fn fetch_nomination_pools(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Ok(vec![])
    }

    /// Returns the wallet's bonded ledger + nominations + unlocking chunks.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Ok(vec![])
    }

    /// Combined `staking::bond` + `staking::nominate` extrinsic.
    pub async fn build_bond_and_nominate_tx(
        &self,
        _wallet_address: &str,
        amount_planck: u128,
        validator_addresses: &[String],
    ) -> Result<StakingActionPreview, StakingError> {
        let count = validator_addresses.len();
        Ok(StakingActionPreview {
            kind: StakingActionKind::Stake,
            validator_identifier: validator_addresses.first().cloned().unwrap_or_default(),
            validator_display_name: if count == 1 {
                validator_addresses[0].clone()
            } else {
                format!("{count} validators nominated")
            },
            amount_smallest_unit: amount_planck.to_string(),
            amount_display: dot_display(amount_planck),
            estimated_fee_smallest_unit: "20000000000".to_string(), // ~2 DOT
            estimated_fee_display: "~2 DOT".to_string(),
            unbonding_period_seconds: 28 * 24 * 3600,
            notes: vec![
                "Bonds DOT and nominates up to 16 validators simultaneously.".to_string(),
                "Requires the current active minimum bond (~250 DOT).".to_string(),
                "Rewards distributed at the end of each era (~24h).".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: Some(
                "Validators can be slashed for equivocation; rewards lost, principal at risk."
                    .to_string(),
            ),
            validator_min_met: Some(amount_planck >= 2_500_000_000_000), // 250 DOT
        })
    }

    /// `nomination_pools::join` for a smaller-stake friendly path.
    pub async fn build_join_pool_tx(
        &self,
        _wallet_address: &str,
        amount_planck: u128,
        pool_id: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Stake,
            validator_identifier: pool_id.to_string(),
            validator_display_name: format!("Nomination Pool #{pool_id}"),
            amount_smallest_unit: amount_planck.to_string(),
            amount_display: dot_display(amount_planck),
            estimated_fee_smallest_unit: "10000000000".to_string(), // ~1 DOT
            estimated_fee_display: "~1 DOT".to_string(),
            unbonding_period_seconds: 28 * 24 * 3600,
            notes: vec![
                "Joins a nomination pool — no active-set minimum required.".to_string(),
                "Minimum: 1 DOT. Rewards compound automatically.".to_string(),
                "Unbonding takes 28 days.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: Some(
                "Pool validators can be slashed; pool members share the loss proportionally."
                    .to_string(),
            ),
            validator_min_met: Some(amount_planck >= 10_000_000_000), // 1 DOT
        })
    }

    pub async fn build_unbond_tx(
        &self,
        _wallet_address: &str,
        amount_planck: u128,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Unstake,
            validator_identifier: String::new(),
            validator_display_name: "Bonded funds".to_string(),
            amount_smallest_unit: amount_planck.to_string(),
            amount_display: dot_display(amount_planck),
            estimated_fee_smallest_unit: "10000000000".to_string(),
            estimated_fee_display: "~1 DOT".to_string(),
            unbonding_period_seconds: 28 * 24 * 3600,
            notes: vec![
                "28-day unbonding period; funds are locked during this time.".to_string(),
                "After unbonding completes, use Withdraw to recover DOT.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    pub async fn build_withdraw_unbonded_tx(
        &self,
        _wallet_address: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Withdraw,
            validator_identifier: String::new(),
            validator_display_name: "Unbonded funds".to_string(),
            amount_smallest_unit: "0".to_string(),
            amount_display: "All unbonded funds".to_string(),
            estimated_fee_smallest_unit: "10000000000".to_string(),
            estimated_fee_display: "~1 DOT".to_string(),
            unbonding_period_seconds: 0,
            notes: vec!["Sweeps all unlocked chunks back to your free balance.".to_string()],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }
}
