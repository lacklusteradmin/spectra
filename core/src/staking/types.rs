//! Shared staking query results and errors.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum StakingAction {
    Stake,
    Unstake,
    Withdraw,
    ClaimRewards,
}

impl StakingAction {
    pub fn transaction_kind(self) -> crate::store::wallet_domain::TransactionKind {
        use crate::store::wallet_domain::TransactionKind as K;
        match self {
            Self::Stake => K::Stake,
            Self::Unstake => K::Unstake,
            Self::Withdraw => K::Withdraw,
            Self::ClaimRewards => K::ClaimRewards,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct StakingReview {
    pub network_fee: String,
    pub fee_is_upper_bound: bool,
    /// Governance disbursement fees reduce the reviewed principal payout.
    pub fee_is_deducted_from_amount: bool,
    /// Recovery completes management steps for an already funded neuron.
    pub funding_already_completed: bool,
    pub refundable_deposit: Option<String>,
    pub lockup_seconds: Option<u64>,
    /// ICP maturity is queued for a delayed payout and modulated by governance.
    pub reward_payout_is_delayed: bool,
}

/// An intent resolved against the wallet's owned identity and current chain state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
pub struct StakingRequest {
    pub wallet_id: String,
    pub chain_id: crate::registry::Chain,
    pub action: StakingAction,
    pub validator_id: Option<String>,
    pub position_id: Option<String>,
    pub amount: Option<String>,
    /// Explicit ICP neuron dissolve delay. It is part of the signed review.
    pub lockup_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum StakingPositionStatus {
    Active,
    Activating,
    Unbonding,
    Withdrawable,
    Inactive,
}

/// Exact on-chain balances, with unknown rewards distinguished from zero.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct StakingPosition {
    pub id: String,
    pub owner: String,
    pub validator_identifier: String,
    pub status: StakingPositionStatus,
    pub staked_amount_smallest_unit: String,
    pub unbonding_amount_smallest_unit: String,
    pub withdrawable_amount_smallest_unit: String,
    pub claimable_rewards_smallest_unit: Option<String>,
    /// Rewards already queued for a protocol-delayed payout.
    pub pending_rewards_smallest_unit: Option<String>,
    pub rewards_unlock_time_unix: Option<u64>,
    pub unlock_epoch: Option<u64>,
    pub unlock_time_unix: Option<u64>,
    /// Core determines which actions are currently valid; the UI renders them.
    pub available_actions: Vec<StakingAction>,
}

/// Form requirements are protocol facts. They guide rendering; builders still
/// resolve authority, balances and valid actions against current chain state.
#[derive(Debug, Clone, uniffi::Record)]
pub struct StakingInputRules {
    pub amount_required: bool,
    pub amount_allowed: bool,
    pub validator_required: bool,
    pub lockup_required: bool,
    pub positions_require_authorization: bool,
    pub repair_allowed: bool,
}

#[uniffi::export]
pub fn staking_input_rules(
    chain: crate::registry::Chain,
    action: StakingAction,
) -> StakingInputRules {
    use crate::registry::Chain;
    let amount_required = match chain {
        Chain::Polkadot => matches!(action, StakingAction::Stake | StakingAction::Unstake),
        Chain::Icp => action == StakingAction::Stake,
        Chain::Solana | Chain::Sui | Chain::Aptos | Chain::Near => {
            action != StakingAction::ClaimRewards
        }
        _ => false,
    };
    StakingInputRules {
        amount_required,
        amount_allowed: amount_required
            || (chain == Chain::Icp && action == StakingAction::Withdraw),
        validator_required: action == StakingAction::Stake && chain.supports_staking(),
        lockup_required: chain == Chain::Icp && action == StakingAction::Stake,
        positions_require_authorization: chain == Chain::Icp,
        repair_allowed: chain == Chain::Icp && action == StakingAction::Stake,
    }
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
}

/// Internal staking query refusal; WalletService maps it to the bridge error.
#[derive(Debug, thiserror::Error)]
pub enum StakingError {
    #[error(transparent)]
    Api(#[from] crate::api::error::ApiError),
    #[error("staking is not yet implemented for this chain")]
    NotYetImplemented,
}
