//! Staking programs and their protocol limits.

use super::*;

/// How a network stakes: the shape every staking read, build and check
/// takes on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StakingModel {
    /// The wallet's own account delegates to a validator or pool, with a
    /// transaction it signs as it signs a transfer: Solana, Sui, Aptos and
    /// NEAR.
    Delegation,
    /// Nomination pools, joined and left by extrinsics: Polkadot.
    NominationPool,
    /// A neuron the wallet's principal controls in the NNS governance
    /// canister, funded by a ledger transfer and read with the wallet's key:
    /// ICP.
    Neuron,
}

impl Chain {
    /// How this network stakes; `None` where Spectra does not. Exact: a
    /// testnet never stakes, since every staking client is built against
    /// its mainnet's programs and pools.
    pub(crate) fn staking_model(self) -> Option<StakingModel> {
        match self {
            Chain::Solana | Chain::Sui | Chain::Aptos | Chain::Near => {
                Some(StakingModel::Delegation)
            }
            Chain::Polkadot => Some(StakingModel::NominationPool),
            Chain::Icp => Some(StakingModel::Neuron),
            _ => None,
        }
    }

    pub fn supports_staking(self) -> bool {
        self.staking_model().is_some()
    }

    pub(crate) fn solana_stake_program(self) -> Result<&'static str, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Solana => Ok("Stake11111111111111111111111111111111111111"),
            _ => Err(RegistryError::NotIn {
                chain: self,
                family: "Solana",
            }),
        }
    }

    pub(crate) fn sui_staking_system(
        self,
    ) -> Result<(&'static str, &'static str, u64), RegistryError> {
        match self.mainnet_counterpart() {
            Self::Sui => Ok(("0x3", "0x5", 1)),
            _ => Err(RegistryError::NotIn {
                chain: self,
                family: "Sui",
            }),
        }
    }

    pub(crate) fn near_staking_whitelist(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Near => Ok("lockup-whitelist.near"),
            _ => Err(RegistryError::NotIn {
                chain: self,
                family: "NEAR mainnet",
            }),
        }
    }

    /// Delegation-pool shares must retain at least 10 APT (framework constant).
    pub fn aptos_delegation_minimum(self) -> Option<u64> {
        (self == Self::Aptos).then_some(1_000_000_000)
    }

    /// Validator-set minimum stake, expressed in MIST.
    pub fn sui_staking_minimum(self) -> Option<u64> {
        (self == Self::Sui).then_some(1_000_000_000)
    }

    /// Maximum native payment reviewed for the staking PTB; dry-run must fit it.
    pub fn sui_staking_gas_budget(self) -> Option<u64> {
        (self == Self::Sui).then_some(10_000_000)
    }

    /// Attached gas budget for the standard staking-pool entry points.
    pub fn near_staking_gas_limit(self) -> Option<u64> {
        (self == Self::Near).then_some(100_000_000_000_000)
    }
}
