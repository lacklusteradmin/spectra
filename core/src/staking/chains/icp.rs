//! Internet Computer staking — neuron lock-ups via the NNS governance canister.
//!
//! Wallet flow (NNS, canister `rrkah-fqaaa-aaaaa-aaaaq-cai`):
//! 1. `manage_neuron::ClaimOrRefresh` to register a neuron with a fresh
//!    subaccount derived from the wallet. Funds (≥1 ICP) get locked into the
//!    governance subaccount on the ICP ledger.
//! 2. `manage_neuron::Configure(SetDissolveTimestamp { timestamp })` or
//!    `IncreaseDissolveDelay` to set lock-up. Minimum is 6 months for
//!    voting-rewards eligibility; max is 8 years (96 months).
//! 3. Voting rewards accrue automatically based on dissolve delay × stake +
//!    voting-participation bonus. `claim_maturity` to pay them out.
//! 4. `Configure(StartDissolving)` begins the dissolve countdown. Once
//!    `dissolve_state == Dissolved`, `Disburse(amount)` sweeps the principal
//!    + rewards back to the wallet account.
//!
//! Native unit: e8s (1 ICP = 1e8 e8s).

use crate::staking::{
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

#[allow(dead_code)]
pub struct IcpStakingClient {
    rosetta_endpoints: Vec<String>,
}

#[allow(dead_code)]
impl IcpStakingClient {
    pub fn new(rosetta_endpoints: Vec<String>) -> Self {
        Self { rosetta_endpoints }
    }

    /// Known-good neurons / followee identities the user can delegate
    /// liquid-democracy votes to. ICP doesn't have validator picking like
    /// other PoS chains; instead users follow other neurons for proposal
    /// votes. Returned list maps to those followee neurons.
    pub async fn fetch_validators(&self) -> Result<Vec<StakingValidator>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// All neurons controlled by this wallet's principal. Calls
    /// `list_neurons` on the NNS governance canister.
    pub async fn fetch_positions(
        &self,
        _wallet_address: &str,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Creates + funds a neuron with the requested dissolve delay (months).
    pub async fn build_create_neuron_tx(
        &self,
        _wallet_address: &str,
        _amount_e8s: u64,
        _dissolve_delay_months: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_increase_dissolve_delay_tx(
        &self,
        _wallet_address: &str,
        _neuron_id: u64,
        _additional_months: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    pub async fn build_start_dissolving_tx(
        &self,
        _wallet_address: &str,
        _neuron_id: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }

    /// Sweeps a fully-dissolved neuron's principal + maturity back to the
    /// wallet account.
    pub async fn build_disburse_tx(
        &self,
        _wallet_address: &str,
        _neuron_id: u64,
        _amount_e8s: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        Err(StakingError::NotYetImplemented)
    }
}
