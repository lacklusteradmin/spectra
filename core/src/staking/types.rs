//! Shared staking query results and errors.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum StakingPositionStatus {
    Active,
    Activating,
    Unbonding,
    Withdrawable,
    Inactive,
}

/// Validator / pool / canister metadata as it appears in the picker UI.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct StakingValidator {
    /// Stable on-chain identifier (vote account, pool ID, validator address,
    /// neuron follow target, etc.). Kept opaque to Swift.
    pub identifier: String,
    /// Display name. Falls back to a truncated identifier if the chain has no
    /// validator naming convention.
    pub display_name: String,
    /// Annualised reward rate as a fraction (0.06 == 6%). 0 if unknown.
    pub apy: f64,
    /// Validator commission as a fraction (0.05 == 5%). None if not modeled.
    pub commission: Option<f64>,
    /// Total stake assigned to this validator, in the chain's native smallest
    /// unit, as a decimal string. None if unknown.
    pub total_stake_smallest_unit: Option<String>,
    /// True if this validator is currently active in the active set.
    pub is_active: bool,
    /// Free-form chain-specific tags ("nomination pool", "commission 5%",
    /// "verified", "saturated", etc.) for UI badges.
    pub tags: Vec<String>,
    /// Minimum delegation amount in the chain's native smallest unit.
    /// `None` if the chain imposes no per-validator minimum.
    pub min_delegation_smallest_unit: Option<String>,
    /// Historical uptime percentage (0.0–100.0). `None` if not reported.
    pub uptime_pct: Option<f64>,
    /// Validator's self-reported website URL.
    pub website: Option<String>,
    /// Validator's self-reported description / identity blurb.
    pub description: Option<String>,
    /// True when this validator will join the active set next epoch.
    /// Useful for showing "activating soon" in the picker. `None` if not tracked.
    pub next_epoch_active: Option<bool>,
}

/// One staking position held by a wallet on a given chain. A wallet can
/// hold multiple positions if it stakes to multiple validators / pools.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct StakingPosition {
    pub validator_identifier: String,
    pub validator_display_name: String,
    pub status: StakingPositionStatus,
    /// Active stake principal in smallest unit, decimal string.
    pub staked_amount_smallest_unit: String,
    /// Pending unbonded amount that's not yet withdrawable, decimal string.
    pub unbonding_amount_smallest_unit: String,
    /// Withdrawable amount (cooldown elapsed), decimal string.
    pub withdrawable_amount_smallest_unit: String,
    /// Outstanding rewards yet to be claimed, decimal string.
    pub claimable_rewards_smallest_unit: String,
    /// Unix timestamp when the unbonding period ends, if applicable.
    pub unbonding_completes_at_unix: Option<i64>,
    /// Chain epoch (or slot / era) when this position was first created.
    /// Useful for calculating lock-up age and APY realised. `None` if not tracked.
    pub epoch_created: Option<i64>,
    /// Rewards accrued since the last claim action, smallest unit decimal string.
    /// Distinct from `claimable_rewards` on chains where rewards vest continuously
    /// but can only be claimed periodically. `None` if not tracked.
    pub accrued_since_last_claim_smallest_unit: Option<String>,
}

/// Internal staking query refusal; WalletService maps it to the bridge error.
#[derive(Debug, thiserror::Error)]
pub enum StakingError {
    #[error("staking is not yet implemented for this chain")]
    NotYetImplemented,
}
