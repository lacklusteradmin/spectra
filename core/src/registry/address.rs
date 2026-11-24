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
    /// Testnets follow their mainnet; EVM and VeChainThor addresses, whose
    /// checksum case carries no meaning of its own, are lowercased, and
    /// Starknet's padded to 64 digits.
    pub fn address_normalization(self) -> AddressNormalization {
        // An EVM or VeChain address's checksum case and a bech32 address's
        // one case carry no meaning of their own.
        if self.is_evm() || self.is_vechain() || self.is_cosmos() {
            return AddressNormalization::Lowercase;
        }
        match self.mainnet_counterpart() {
            // Both write RFC 4648 base32, whose letters are uppercase.
            Chain::Stellar | Chain::Algorand => AddressNormalization::Uppercase,
            Chain::Starknet => AddressNormalization::HexFelt,
            Chain::Sui | Chain::Aptos => AddressNormalization::LowercaseHexPrefixed,
            Chain::Icp | Chain::Near => AddressNormalization::Lowercase,
            // `0.0.N`, its checksum dropped (`derivation::hedera`).
            Chain::Hedera => AddressNormalization::None,
            _ => AddressNormalization::None,
        }
    }
}
