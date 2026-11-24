//! VeChainThor: the network a transaction is bound to, the contract whose
//! token pays its gas, and how long a signed transaction lives.
//!
//! Every fact here is bound into what a transaction signs (the chain tag) or
//! decides what pays for it (VTHO's contract), so the network is known by its
//! variant rather than by anything a node answers; a node is asked for its
//! genesis block and refused unless it is this one.

use super::*;

/// One VeChainThor network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThorNetwork {
    /// The id of block 0, `0x`-prefixed lowercase hex, which a node's
    /// `GET /blocks/0` must answer.
    pub genesis_id: &'static str,
    /// VTHO, the built-in `Energy` contract: the VIP-180 token every
    /// transaction's gas is paid in.
    pub energy: &'static str,
}

impl ThorNetwork {
    /// The byte every transaction on this network carries: the genesis id's
    /// last.
    pub(crate) fn chain_tag(&self) -> u8 {
        let last = &self.genesis_id[self.genesis_id.len() - 2..];
        u8::from_str_radix(last, 16).expect("a genesis id is hex")
    }
}

const MAINNET: ThorNetwork = ThorNetwork {
    genesis_id: "0x00000000851caf3cfdb6e899cf5958bfb1ac3413d346d43539627e6be7ec1b4a",
    energy: "0x0000000000000000000000000000456e65726779",
};

/// How many blocks past its block reference a signed transaction stays
/// valid: about ten minutes of ten-second blocks, as thor's own client
/// signs them.
pub(crate) const THOR_EXPIRATION_BLOCKS: u32 = 60;

impl Chain {
    /// This network's VeChainThor facts, or `None` outside it.
    pub(crate) fn thor(self) -> Option<&'static ThorNetwork> {
        match self {
            Self::Vechain => Some(&MAINNET),
            _ => None,
        }
    }

    /// Whether this is VeChainThor, read through `api::thor_rest` and signed
    /// by `send::vechain`.
    pub fn is_vechain(self) -> bool {
        self.thor().is_some()
    }

    /// [`Chain::thor`], or the refusal a caller outside it gets.
    pub(crate) fn thor_network(self) -> Result<&'static ThorNetwork, RegistryError> {
        self.thor().ok_or_else(|| self.not_in("VeChainThor"))
    }
}
