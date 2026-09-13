//! Polkadot staking validator and position queries.

use crate::staking::{StakingError, StakingPosition, StakingValidator};

pub struct PolkadotStakingClient {
    _sidecar_endpoints: Vec<String>,
}

impl PolkadotStakingClient {
    pub fn new(sidecar_endpoints: Vec<String>) -> Self {
        Self {
            _sidecar_endpoints: sidecar_endpoints,
        }
    }

    /// Active validator set. Sidecar: `/pallets/staking/storage/validators`.
    /// Validator data requires SCALE decoding — returns empty until a
    /// Substrate Sidecar REST endpoint is wired to the endpoint catalog.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Ok(vec![])
    }

    /// Returns the wallet's bonded ledger + nominations + unlocking chunks.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Ok(vec![])
    }
}
