//! The Substrate family: each network's SS58 prefix, genesis, balance width,
//! fee extension and the inert transaction extensions its runtime adds.
//!
//! The genesis hash is bound into every signature, and the prefix and the
//! balance width decide what an address and an account record are, so a
//! network is known by its variant rather than by what a node answers.
//! Everything else a transfer needs (pallet and call indices, the existential
//! deposit, the extensions and their order) is read from the runtime's own
//! metadata and checked against these (`api::substrate_json_rpc::metadata`).

use super::*;

/// The transaction extension a network's fees are paid through, which sets
/// what a signed transaction carries for it. Spectra pays in the network's
/// own coin, without a tip, on either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubstrateFeePayment {
    /// `ChargeAssetTxPayment`, Asset Hub's: a tip and the asset that pays,
    /// `None` for the network's coin.
    Asset,
    /// `ChargeTransactionPayment`: a tip alone.
    Native,
}

/// One Substrate network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SubstrateNetwork {
    /// The SS58 prefix its addresses carry; an address with another is
    /// refused.
    pub ss58_prefix: u16,
    /// Block zero's hash, which a node must answer and every signature binds.
    pub genesis_hash: &'static str,
    /// The byte width of the runtime's `Balance`, which sizes its
    /// `System.Account` record.
    pub balance_bytes: usize,
    pub fee_payment: SubstrateFeePayment,
    /// Transaction extensions of this runtime's own that carry nothing in a
    /// transaction, beside the Polkadot SDK's that every runtime may have.
    pub inert_extensions: &'static [&'static str],
}

/// DOT balances and transfers live on Polkadot Asset Hub, not the relay
/// chain.
const POLKADOT: SubstrateNetwork = SubstrateNetwork {
    ss58_prefix: 0,
    genesis_hash: "0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f",
    balance_bytes: 16,
    fee_payment: SubstrateFeePayment::Asset,
    inert_extensions: &[],
};

/// Westend Asset Hub.
const POLKADOT_WESTEND: SubstrateNetwork = SubstrateNetwork {
    ss58_prefix: 42,
    genesis_hash: "0x67f9723393ef76214df0118c34bbbd3dbebc8ed46a10973a8c969d48fe7598c9",
    balance_bytes: 16,
    fee_payment: SubstrateFeePayment::Asset,
    inert_extensions: &[],
};

/// Cross-checked on both official Finney RPCs on 2026-10-04.
const BITTENSOR: SubstrateNetwork = SubstrateNetwork {
    ss58_prefix: 42,
    genesis_hash: "0x2f0555cc76fc2840a25a6ea3b9637146806f1f44b090c175ffde2a7e5ab36c03",
    balance_bytes: 8,
    fee_payment: SubstrateFeePayment::Native,
    inert_extensions: &[
        "SudoTransactionExtension",
        "CheckShieldedTxValidity",
        "SubtensorTransactionExtension",
        "DrandPriority",
    ],
};

/// KSM balances and transfers live on Kusama Asset Hub, not the relay chain,
/// as DOT's do on Polkadot's. Its genesis was read on three public RPCs on
/// 2026-11-24.
const KUSAMA: SubstrateNetwork = SubstrateNetwork {
    ss58_prefix: 2,
    genesis_hash: "0x48239ef607d7928874027a43a67689209727dfb3d3dc5e5b03a39bdc2eda771a",
    balance_bytes: 16,
    fee_payment: SubstrateFeePayment::Asset,
    inert_extensions: &[],
};

/// Astar's Substrate side, whose native accounts hold ASTR; its EVM side is
/// a separate account space. Its genesis was read on four public RPCs on
/// 2026-11-24.
const ASTAR: SubstrateNetwork = SubstrateNetwork {
    ss58_prefix: 5,
    genesis_hash: "0x9eb76c5184c4ab8679d2d5d819fdf90b9c001403e9e17da2e14b6d8aec4029c6",
    balance_bytes: 16,
    fee_payment: SubstrateFeePayment::Native,
    inert_extensions: &[],
};

impl Chain {
    /// This network's Substrate facts, or `None` outside the family.
    pub(crate) fn substrate(self) -> Option<&'static SubstrateNetwork> {
        Some(match self {
            Self::Polkadot => &POLKADOT,
            Self::PolkadotWestend => &POLKADOT_WESTEND,
            Self::Bittensor => &BITTENSOR,
            Self::Kusama => &KUSAMA,
            Self::Astar => &ASTAR,
            _ => return None,
        })
    }

    /// Whether this is a Substrate network, read through
    /// `api::substrate_json_rpc` and signed by `send::polkadot`.
    pub fn is_substrate(self) -> bool {
        self.substrate().is_some()
    }

    /// The SS58 prefix this network's addresses carry: Polkadot's 0,
    /// Kusama's 2, Astar's 5 and the generic 42 on Westend and Bittensor.
    pub fn ss58_prefix(self) -> Option<u16> {
        self.substrate().map(|network| network.ss58_prefix)
    }

    /// The genesis hash a Substrate network's nodes must answer.
    pub fn substrate_genesis_hash(self) -> Option<&'static str> {
        self.substrate().map(|network| network.genesis_hash)
    }

    /// The byte width of a Substrate network's `Balance` type, which sizes
    /// its `System.Account` record: `u128` everywhere but subtensor's `u64`.
    pub fn substrate_balance_bytes(self) -> Option<usize> {
        self.substrate().map(|network| network.balance_bytes)
    }
}
