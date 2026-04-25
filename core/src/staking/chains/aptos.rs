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

use crate::staking::{
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

#[allow(dead_code)]
pub struct AptosStakingClient {
    rest_endpoints: Vec<String>,
}

#[allow(dead_code)]
impl AptosStakingClient {
    pub fn new(rest_endpoints: Vec<String>) -> Self {
        Self { rest_endpoints }
    }

    /// REST: `/v1/accounts/0x1/resource/0x1::stake::ValidatorSet` for the
    /// active set; for delegation pools, scan the on-chain registry of pool
    /// addresses (e.g. via Aptos Labs API or indexer).
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// REST: per-pool `get_stake(pool_address, wallet_address)` view function
    /// returns (active, inactive, pending_inactive) buckets.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_add_stake_tx(
        &self,
        _wallet_address: &str,
        _pool_address: &str,
        _amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_unlock_tx(
        &self,
        _wallet_address: &str,
        _pool_address: &str,
        _amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_withdraw_tx(
        &self,
        _wallet_address: &str,
        _pool_address: &str,
        _amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }
}
