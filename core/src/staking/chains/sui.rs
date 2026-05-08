//! Sui staking — `0x3::sui_system::request_add_stake` Move call.
//!
//! Wallet flow:
//! 1. `fetch_validators` returns the active validator set from
//!    `0x5::sui_system_state::SuiSystemState` epoch info.
//! 2. `build_request_add_stake_tx` constructs a programmable tx with a single
//!    `request_add_stake_mul_coin` call: passes a SUI Coin, validator address,
//!    and amount. Returns a `StakedSui` object owned by the wallet.
//! 3. `build_request_withdraw_stake_tx` calls `request_withdraw_stake` with
//!    the `StakedSui` object reference. Funds + rewards are returned at the
//!    end of the current epoch.
//!
//! Native unit: MIST (1 SUI = 1e9 MIST). Rewards accrue per-epoch (~24h).

use crate::staking::{
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

pub struct SuiStakingClient {
    rpc_endpoints: Vec<String>,
}

impl SuiStakingClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Self {
        Self { rpc_endpoints }
    }

    /// RPC: `suix_getLatestSuiSystemState`. Validator list comes back with
    /// pool_id, voting_power, commission_rate, next_epoch_stake.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// RPC: `suix_getStakes` returns active + pending stakes for `wallet_address`.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_request_add_stake_tx(
        &self,
        _wallet_address: &str,
        _amount_mist: u64,
        _validator_address: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// `staked_sui_object_id` must be a `StakedSui` object owned by the wallet.
    pub async fn build_request_withdraw_stake_tx(
        &self,
        _wallet_address: &str,
        _staked_sui_object_id: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }
}
