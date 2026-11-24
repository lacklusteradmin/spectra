//! Algorand: the network identity every transaction binds and the consensus
//! limits a payment is built within.
//!
//! The genesis id and hash are written into each transaction (`gen`, `gh`),
//! and a node's `/v2/transactions/params` must name both before core signs
//! through it. Values read from go-algorand's consensus parameters and from
//! the mainnet nodes themselves on 2026-11-24.

use super::*;

/// The network a transaction is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AlgorandNetwork {
    /// `gen`: the genesis id.
    pub genesis_id: &'static str,
    /// `gh`: SHA-512/256 of the genesis block,
    /// `wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=` in base64.
    pub genesis_hash: [u8; 32],
}

const MAINNET: AlgorandNetwork = AlgorandNetwork {
    genesis_id: "mainnet-v1.0",
    genesis_hash: [
        0xc0, 0x61, 0xc4, 0xd8, 0xfc, 0x1d, 0xbd, 0xde, 0xd2, 0xd7, 0x60, 0x4b, 0xe4, 0x56, 0x8e,
        0x3f, 0x6d, 0x04, 0x19, 0x87, 0xac, 0x37, 0xbd, 0xe4, 0xb6, 0x20, 0xb5, 0xab, 0x39, 0x24,
        0x8a, 0xdf,
    ],
};

/// The least an account holds, in microAlgos, while it holds anything:
/// consensus `MinBalance`. A payment that would leave its recipient below
/// it fails, so a new account's first payment must bring at least this.
/// Each asset or application an account opts into raises its own minimum,
/// which a node reports as the account's `min-balance`.
pub(crate) const MIN_BALANCE: u64 = 100_000;

/// The most rounds a transaction's validity window spans: consensus
/// `MaxTxnLife`, so `last_valid - first_valid <= 1000`.
pub(crate) const MAX_TXN_LIFE: u64 = 1_000;

impl Chain {
    /// The Algorand network this chain's transactions bind, or the refusal
    /// a caller outside it gets.
    pub(crate) fn algorand_network(self) -> Result<&'static AlgorandNetwork, RegistryError> {
        match self {
            Self::Algorand => Ok(&MAINNET),
            _ => Err(self.not_in("Algorand")),
        }
    }
}
