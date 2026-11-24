//! Hedera: the network's identity and the terms a transfer is built with.
//!
//! A Hedera account is an entity (`0.0.N`) the network creates by a
//! transaction. It holds a key on chain, and no address derives from a key:
//! a wallet holds an account the network already has, found by its key on a
//! mirror node or named at import. Each value names its source where it is
//! defined.

use super::*;

/// One Hedera network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HederaNetwork {
    /// The ledger id (HIP-198) an account id's checksum (HIP-15) binds:
    /// `0x00` on mainnet.
    pub ledger_id: &'static [u8],
    /// The hash a mirror node gives the network's first record file
    /// (`/api/v1/blocks/0`), which only that network's mirror nodes serve.
    pub genesis_block_hash: &'static str,
}

const MAINNET: HederaNetwork = HederaNetwork {
    ledger_id: &[0x00],
    // Record file 2019-09-13T21_53_51.396440Z.rcd.
    genesis_block_hash: "0x420fffe68fcd2a1eadcce589fdf9565bcf5a269d02232fe07cdc565b3b6f76ce46a9418ddc1bbe051d4894e04d091f8e",
};

/// How long a signed transaction stays valid after its valid start: the
/// `transactionValidDuration` every transfer carries (the network takes 15
/// to 180 seconds).
pub(crate) const TRANSACTION_VALID_SECS: i64 = 120;

/// The longest memo a transaction carries, in bytes of UTF-8
/// (`MEMO_TOO_LONG` past it).
pub(crate) const MEMO_MAX_BYTES: usize = 100;

/// How many consensus nodes a transfer is signed for, one body each: a busy
/// node passes the same transaction id to the next.
pub(crate) const SUBMISSION_NODES: usize = 3;

/// The most a transfer may pay in fees, in thousandths of a US dollar:
/// $0.001, ten times the $0.0001 the fee schedule charges a `CryptoTransfer`,
/// so a 100-byte memo and the network's congestion pricing fit. The network
/// charges what the transfer costs, up to the `transactionFee` this sets.
pub(crate) const TRANSFER_FEE_BOUND_MILLS: u128 = 1;

/// Account numbers up to this are the network's own (fee collection,
/// staking reward and node accounts); a person's account is above it.
pub(crate) const LAST_SYSTEM_ACCOUNT: u64 = 1000;

impl Chain {
    /// This network's Hedera facts, or `None` outside Hedera.
    pub(crate) fn hedera(self) -> Option<&'static HederaNetwork> {
        match self {
            Self::Hedera => Some(&MAINNET),
            _ => None,
        }
    }

    /// Whether this is a Hedera network, read through `api::hedera_mirror`
    /// and submitted through `api::hedera_hapi`.
    pub fn is_hedera(self) -> bool {
        self.hedera().is_some()
    }

    /// [`Chain::hedera`], or the refusal a caller outside Hedera gets.
    pub(crate) fn hedera_network(self) -> Result<&'static HederaNetwork, RegistryError> {
        self.hedera().ok_or_else(|| self.not_in("Hedera"))
    }

    /// Whether an account on this network is created by a transaction rather
    /// than derived from a key. A key holds no address of its own: an import
    /// finds the account that holds it on the network, or takes the one it
    /// names, and a phrase or key that holds none is refused.
    pub fn finds_accounts_by_key(self) -> bool {
        self.is_hedera()
    }
}
