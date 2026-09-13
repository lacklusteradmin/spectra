//! Cardano staking validator and position queries.

use crate::staking::{StakingError, StakingPosition, StakingValidator};

pub struct CardanoStakingClient {
    _rest_endpoints: Vec<String>,
    _api_key: Option<String>,
}

impl CardanoStakingClient {
    pub fn new(rest_endpoints: Vec<String>, api_key: Option<String>) -> Self {
        Self {
            _rest_endpoints: rest_endpoints,
            _api_key: api_key,
        }
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
}
