//! Solana staking — `StakeProgram` create / delegate / deactivate / withdraw.
//!
//! Wallet flow:
//! 1. User picks a vote account (validator). `fetch_validators` returns the
//!    active set. Each Solana stake position is a separate stake account
//!    keyed by a fresh keypair derived from the wallet.
//! 2. `build_create_and_delegate_tx` emits a single tx with three System +
//!    Stake program instructions: create stake account, initialize, delegate.
//! 3. `build_deactivate_tx` flips the stake account to deactivating; rewards
//!    stop accruing at the next epoch boundary.
//! 4. After deactivation completes (~2-3 days), `build_withdraw_tx` moves
//!    lamports back to the owning wallet.
//!
//! Native unit: lamport (1 SOL = 1e9 lamports). All amounts in smallest unit.

use crate::staking::{
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

pub struct SolanaStakingClient {
    rpc_endpoints: Vec<String>,
}

impl SolanaStakingClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Self {
        Self { rpc_endpoints }
    }

    /// Snapshot of the active validator set with vote-account identifier and
    /// computed APY. RPC: `getVoteAccounts` + epoch reward history.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Stake accounts owned by this wallet, with their delegate / amount /
    /// state (active, activating, deactivating, etc.). RPC:
    /// `getProgramAccounts` filtered to StakeProgram + memcmp on staker.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Single-tx create + initialize + delegate to `vote_account`. Allocates a
    /// fresh stake account from a derived keypair so each position is
    /// independently manageable.
    pub async fn build_create_and_delegate_tx(
        &self,
        _wallet_address: &str,
        _amount_lamports: u64,
        _vote_account: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_deactivate_tx(
        &self,
        _wallet_address: &str,
        _stake_account: &str,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_withdraw_tx(
        &self,
        _wallet_address: &str,
        _stake_account: &str,
        _amount_lamports: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }
}
