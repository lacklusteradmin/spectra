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

use crate::staking::{
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

pub struct NearStakingClient {
    rpc_endpoints: Vec<String>,
}

impl NearStakingClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Self {
        Self { rpc_endpoints }
    }

    /// JSON-RPC: `validators` for the active set; supplement with view calls
    /// to each pool's `get_reward_fee_fraction` and `get_total_staked_balance`.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// View call: `get_account(account_id)` on each pool the wallet has
    /// interacted with. Returns staked / unstaked / can_withdraw.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_deposit_and_stake_tx(
        &self,
        _wallet_address: &str,
        _pool_account_id: &str,
        _amount_yocto_near: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_unstake_tx(
        &self,
        _wallet_address: &str,
        _pool_account_id: &str,
        _amount_yocto_near: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_withdraw_tx(
        &self,
        _wallet_address: &str,
        _pool_account_id: &str,
        _amount_yocto_near: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }
}
