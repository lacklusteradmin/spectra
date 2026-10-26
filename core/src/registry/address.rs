//! How addresses are spelled, compared and named.

use super::*;

impl Chain {
    /// Does a name typed as a destination resolve to an address on this chain?
    ///
    /// ENS is a registry deployed on Ethereum mainnet, so that is the only
    /// chain a `.eth` name is looked up for. The address it returns is a plain
    /// EVM address that would spend anywhere, but a name pointing at a mainnet
    /// contract need not point at anything on an L2 — a destination reached by
    /// name is accepted only where the registry that named it lives, which is
    /// the stricter of the two readings.
    pub fn resolves_ens_names(self) -> bool {
        matches!(self, Chain::Ethereum)
    }

    /// Canonical address normalization for storage and comparison.
    /// Testnets follow their mainnet; EVM addresses are lowercased.
    pub fn address_normalization(self) -> AddressNormalization {
        if self.is_evm() {
            return AddressNormalization::Lowercase;
        }
        match self.mainnet_counterpart() {
            Chain::Stellar => AddressNormalization::Uppercase,
            Chain::Sui | Chain::Aptos => AddressNormalization::LowercaseHexPrefixed,
            Chain::Icp | Chain::Near => AddressNormalization::Lowercase,
            _ => AddressNormalization::None,
        }
    }

    pub fn flags_evm_address_as_wrong_chain(self) -> bool {
        matches!(
            self,
            Chain::Bitcoin
                | Chain::BitcoinCash
                | Chain::Litecoin
                | Chain::Dogecoin
                | Chain::BitcoinTestnet
                | Chain::BitcoinTestnet4
                | Chain::BitcoinSignet
                | Chain::BitcoinCashTestnet
                | Chain::LitecoinTestnet
                | Chain::DogecoinTestnet
                | Chain::Peercoin
                | Chain::PeercoinTestnet
        )
    }
}
