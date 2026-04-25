//! Shared, chain-agnostic staking types. Per-chain clients return / accept
//! these wherever a generic shape is meaningful; chain-specific extras
//! (e.g. Polkadot nomination pools) live alongside the chain client.

use serde::{Deserialize, Serialize};

/// User-facing classification of a staking action. The flow Swift asks for;
/// each chain client maps it to the chain-native operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum StakingActionKind {
    /// Begin staking (delegate / bond / stake / lock-up).
    Stake,
    /// Begin un-staking; rewards stop accruing immediately.
    Unstake,
    /// After an unbonding/cooldown period, sweep withdrawable funds.
    Withdraw,
    /// Move existing stake to a different validator/pool without unbonding.
    Restake,
    /// Claim outstanding rewards without touching principal.
    ClaimRewards,
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

/// Validator / pool / canister metadata as it appears in the picker UI.
/// Per-chain fields that don't fit the generic surface (commission rate,
/// minimum delegation, identity verification) are expressed via
/// `details` k/v pairs so the UI can show them without typing pressure.
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
}

/// Amount preview for a staking action — what Swift renders before sign.
/// Mirrors `EthereumSendPreview` etc. but with staking-specific fields.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct StakingActionPreview {
    pub kind: StakingActionKind,
    pub validator_identifier: String,
    pub validator_display_name: String,
    /// Action amount in smallest unit, decimal string.
    pub amount_smallest_unit: String,
    /// Display-formatted amount in the chain's native unit ("1.5 SOL").
    pub amount_display: String,
    /// Estimated chain fee, smallest unit decimal string.
    pub estimated_fee_smallest_unit: String,
    /// Display-formatted fee.
    pub estimated_fee_display: String,
    /// Cooldown / unbonding period in seconds. 0 if instant.
    pub unbonding_period_seconds: i64,
    /// Free-form notes the UI should surface ("Activates next epoch", "Min
    /// delegation 1 SOL", "Locked for 6 months", etc.).
    pub notes: Vec<String>,
}

/// Errors returned by staking client operations. UniFFI-friendly; the
/// `String` carries provider-specific detail for diagnostics.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum StakingError {
    #[error("staking is not yet implemented for this chain")]
    NotYetImplemented,
    #[error("invalid validator identifier: {0}")]
    InvalidValidator(String),
    #[error("amount below minimum: {0}")]
    AmountBelowMinimum(String),
    #[error("insufficient balance: {0}")]
    InsufficientBalance(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("provider returned malformed response: {0}")]
    MalformedResponse(String),
}
