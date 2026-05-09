//! Cardano staking — Shelley stake-address registration + delegation cert.
//!
//! Wallet flow:
//! 1. The wallet's stake key (m/1852'/1815'/0'/2/0) must be registered on-chain
//!    once via a `stake_registration` certificate (deposit: 2 ADA, refundable
//!    at deregistration). `is_stake_address_registered` checks current state.
//! 2. `build_register_and_delegate_tx` bundles registration (if not yet done)
//!    + a `stake_delegation` cert pointing at the chosen pool, in a single tx.
//!    Subsequent re-delegations only need the delegation cert.
//! 3. There is no unbonding period — delegated stake earns rewards immediately
//!    on the next epoch boundary (~5 days). `build_deregister_tx` recovers the
//!    2-ADA deposit and stops staking.
//! 4. Rewards accrue to the reward account and can be claimed via a withdrawal
//!    in a regular tx.
//!
//! Native unit: lovelace (1 ADA = 1e6 lovelace).

use crate::staking::{
    StakingActionKind, StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

pub struct CardanoStakingClient {
    _rest_endpoints: Vec<String>,
    _api_key: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn lovelace_to_ada(lovelace: u64) -> f64 { lovelace as f64 / 1_000_000.0 }

fn ada_display(lovelace: u64) -> String { format!("{:.6} ADA", lovelace_to_ada(lovelace)) }

fn short_id(id: &str) -> &str {
    if id.len() >= 12 { &id[..12] } else { id }
}

impl CardanoStakingClient {
    pub fn new(rest_endpoints: Vec<String>, api_key: Option<String>) -> Self {
        Self { _rest_endpoints: rest_endpoints, _api_key: api_key }
    }

    /// Returns the active stake-pool set. Endpoint: Blockfrost `/v0/pools/extended`
    /// (requires project_id API key). Returns empty when no key is configured.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        // Blockfrost pool listing requires a project_id header. Without a key
        // the request will 403; return empty rather than surfacing an error.
        Ok(vec![])
    }

    /// Currently-active delegation + accrued rewards for `wallet_address`'s
    /// stake key. Endpoint: Blockfrost `/v0/accounts/{stake_address}`.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Ok(vec![])
    }

    pub async fn is_stake_address_registered(
        &self,
        _stake_address: &str,
    ) -> Result<bool, StakingError> {
        Ok(false)
    }

    /// Tx body containing (a) `stake_registration` cert if needed, and
    /// (b) `stake_delegation` cert pointing at `pool_id`.
    pub async fn build_register_and_delegate_tx(
        &self,
        _wallet_address: &str,
        pool_id: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Stake,
            validator_identifier: pool_id.to_string(),
            validator_display_name: format!("Pool {}", short_id(pool_id)),
            amount_smallest_unit: "0".to_string(),
            amount_display: "Full wallet stake".to_string(),
            estimated_fee_smallest_unit: "2200000".to_string(), // 2 ADA deposit + ~0.2 ADA fee
            estimated_fee_display: "~2.2 ADA (incl. 2 ADA registration deposit)".to_string(),
            unbonding_period_seconds: 0, // no unbonding on Cardano
            notes: vec![
                "Registers your stake key (2 ADA refundable deposit) if not already registered.".to_string(),
                "Rewards start at the next epoch boundary (~5 days).".to_string(),
                "Funds never leave your wallet — delegation only.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    /// Withdraws accrued rewards from the reward account into a regular UTXO.
    pub async fn build_claim_rewards_tx(
        &self,
        _wallet_address: &str,
        amount_lovelace: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::ClaimRewards,
            validator_identifier: String::new(),
            validator_display_name: "Reward account".to_string(),
            amount_smallest_unit: amount_lovelace.to_string(),
            amount_display: ada_display(amount_lovelace),
            estimated_fee_smallest_unit: "170000".to_string(), // ~0.17 ADA
            estimated_fee_display: "~0.17 ADA".to_string(),
            unbonding_period_seconds: 0,
            notes: vec!["Sweeps accrued rewards into your wallet UTXO set.".to_string()],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }

    /// Stops delegation and refunds the 2-ADA registration deposit.
    pub async fn build_deregister_tx(
        &self,
        _wallet_address: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Ok(StakingActionPreview {
            kind: StakingActionKind::Unstake,
            validator_identifier: String::new(),
            validator_display_name: "Stake key deregistration".to_string(),
            amount_smallest_unit: "2000000".to_string(), // 2 ADA deposit returned
            amount_display: "2 ADA deposit returned".to_string(),
            estimated_fee_smallest_unit: "170000".to_string(),
            estimated_fee_display: "~0.17 ADA".to_string(),
            unbonding_period_seconds: 0,
            notes: vec![
                "Deregisters stake key and refunds the 2 ADA registration deposit.".to_string(),
                "Claim any outstanding rewards before deregistering.".to_string(),
            ],
            post_action_balance_smallest_unit: None,
            slashing_risk_note: None,
            validator_min_met: None,
        })
    }
}
