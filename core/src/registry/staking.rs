//! Staking programs and their protocol limits.

use super::*;

impl Chain {
    pub fn supports_staking(self) -> bool {
        matches!(
            self,
            Chain::Solana | Chain::Sui | Chain::Aptos | Chain::Near | Chain::Polkadot | Chain::Icp
        )
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
