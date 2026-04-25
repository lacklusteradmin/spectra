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
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

#[allow(dead_code)]
pub struct CardanoStakingClient {
    rest_endpoints: Vec<String>,
}

#[allow(dead_code)]
impl CardanoStakingClient {
    pub fn new(rest_endpoints: Vec<String>) -> Self {
        Self { rest_endpoints }
    }

    /// Returns the active stake-pool set. Endpoint: Koios `/pool_list` +
    /// per-pool `/pool_info` for live pledge / margin / rho_alpha.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Currently-active delegation + accrued rewards for `wallet_address`'s
    /// stake key. Endpoint: Koios `/account_info` + `/account_rewards`.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn is_stake_address_registered(
        &self,
        _stake_address: &str,
    ) -> Result<bool, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Tx body containing (a) `stake_registration` cert if needed, and
    /// (b) `stake_delegation` cert pointing at `pool_id`.
    pub async fn build_register_and_delegate_tx(
        &self,
        _wallet_address: &str,
        _pool_id: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Withdraws accrued rewards from the reward account into a regular UTXO.
    pub async fn build_claim_rewards_tx(
        &self,
        _wallet_address: &str,
        _amount_lovelace: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Stops delegation and refunds the 2-ADA registration deposit.
    pub async fn build_deregister_tx(
        &self,
        _wallet_address: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }
}
