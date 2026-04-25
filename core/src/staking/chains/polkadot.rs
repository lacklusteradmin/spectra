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
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

#[allow(dead_code)]
pub struct PolkadotStakingClient {
    sidecar_endpoints: Vec<String>,
}

#[allow(dead_code)]
impl PolkadotStakingClient {
    pub fn new(sidecar_endpoints: Vec<String>) -> Self {
        Self { sidecar_endpoints }
    }

    /// Active validator set. Sidecar: `/pallets/staking/storage/validators`.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Open nomination pools accepting new members. Sidecar:
    /// `/pallets/nomination-pools/storage/bondedPools`.
    pub async fn fetch_nomination_pools(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Returns the wallet's bonded ledger + nominations + unlocking chunks.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Combined `staking::bond` + `staking::nominate` extrinsic.
    pub async fn build_bond_and_nominate_tx(
        &self,
        _wallet_address: &str,
        _amount_planck: u128,
        _validator_addresses: &[String],
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// `nomination_pools::join` for a smaller-stake friendly path.
    pub async fn build_join_pool_tx(
        &self,
        _wallet_address: &str,
        _amount_planck: u128,
        _pool_id: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_unbond_tx(
        &self,
        _wallet_address: &str,
        _amount_planck: u128,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_withdraw_unbonded_tx(
        &self,
        _wallet_address: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }
}
