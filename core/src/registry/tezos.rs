//! Tezos: the network's chain id, the protocols whose operation encoding
//! `send::tezos` forges, the constants of each that size a transfer's
//! limits, burn and expiry, and the fee bakers take by default.
//!
//! A protocol is known by its hash. An operation is forged in the encoding
//! of the protocol that will apply it, so `send::tezos` refuses to build or
//! sign while a node names a protocol that is not listed here; a new
//! protocol is added once its manager-operation encoding has been checked
//! against Taquito's forger and the network's own operations. The sources
//! and the dated readings that confirm each value are in
//! `docs/audits/tezos-2026-11-24.md`.

use super::*;

/// One Tezos protocol whose reveal and transaction encoding `send::tezos`
/// writes, with the constants a transfer under it is sized by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TezosProtocol {
    /// The protocol hash a node's `/chains/main/blocks/head/protocols`
    /// names.
    pub hash: &'static str,
    /// How many blocks an operation's branch stays live for: an operation
    /// whose branch is at level `L` can be included up to level
    /// `L + max_operations_ttl`.
    pub max_operations_ttl: u64,
    /// The mutez burned for each byte of storage an operation adds.
    pub cost_per_byte: u64,
    /// The storage bytes a transfer burns for when it creates the implicit
    /// account it pays.
    pub origination_size: u64,
    /// The most gas and storage one operation may name, and the most gas a
    /// block, so a batch, holds.
    pub hard_gas_limit_per_operation: u64,
    pub hard_storage_limit_per_operation: u64,
    pub hard_gas_limit_per_block: u64,
    /// The gas an Ed25519 key's reveal uses as the first operation of a
    /// reveal-and-transfer batch, the batch's signature check included.
    pub reveal_gas: u64,
    /// The gas a transfer to an implicit account uses when it is not the
    /// first operation of its batch, and so bears no signature check.
    pub implicit_transfer_gas: u64,
}

/// Protocol 025, Ushuaia, active on mainnet since level 13,857,889.
const USHUAIA: TezosProtocol = TezosProtocol {
    hash: "PsUshuai9QapM5TGj1JpuVGkdxz5GykdnEvS6Rh8SUVrARvZLCY",
    max_operations_ttl: 600,
    cost_per_byte: 250,
    origination_size: 257,
    hard_gas_limit_per_operation: 1_040_000,
    hard_storage_limit_per_operation: 60_000,
    hard_gas_limit_per_block: 1_040_000,
    reveal_gas: 171,
    implicit_transfer_gas: 2_101,
};

/// A Tezos network and the fee rule its bakers apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TezosNetwork {
    /// The chain id a node's `/chains/main/chain_id` answers, which every
    /// operation is checked against before it is signed.
    pub chain_id: &'static str,
    /// The protocols `send::tezos` forges for.
    pub protocols: &'static [TezosProtocol],
    /// The default mempool filter of an Octez node: a batch pays at least
    /// `minimal_fee_mutez`, plus `nanotez_per_gas` for each unit of its gas
    /// limits and `nanotez_per_byte` for each byte of the signed operation.
    pub minimal_fee_mutez: u64,
    pub nanotez_per_gas: u64,
    pub nanotez_per_byte: u64,
    /// What a batch pays above that minimum.
    pub fee_margin_mutez: u64,
    /// The gas each operation may use beyond what a simulation used:
    /// octez-client's safety guard.
    pub gas_headroom: u64,
    /// The storage bytes a contract's own storage may grow beyond what a
    /// simulation saw. An account a transfer creates always takes
    /// `origination_size` exactly.
    pub storage_headroom: u64,
    /// How many blocks below the head an operation's branch is taken: a
    /// block two levels down is final under Tenderbake, so the branch cannot
    /// be reorganized away.
    pub branch_depth: u64,
}

const MAINNET: TezosNetwork = TezosNetwork {
    chain_id: "NetXdQprcVkpaWU",
    protocols: &[USHUAIA],
    minimal_fee_mutez: 100,
    nanotez_per_gas: 100,
    nanotez_per_byte: 1_000,
    fee_margin_mutez: 100,
    gas_headroom: 100,
    storage_headroom: 20,
    branch_depth: 2,
};

impl Chain {
    /// This network's Tezos facts, or `None` elsewhere.
    pub(crate) fn tezos(self) -> Option<&'static TezosNetwork> {
        match self {
            Self::Tezos => Some(&MAINNET),
            _ => None,
        }
    }

    /// [`Chain::tezos`], or the refusal a caller outside Tezos gets.
    pub(crate) fn tezos_network(self) -> Result<&'static TezosNetwork, RegistryError> {
        self.tezos().ok_or_else(|| self.not_in("Tezos"))
    }
}

impl TezosNetwork {
    /// The protocol `hash` names, when `send::tezos` forges for it.
    pub(crate) fn protocol(&self, hash: &str) -> Option<&'static TezosProtocol> {
        self.protocols.iter().find(|protocol| protocol.hash == hash)
    }

    /// The least a batch with `gas` units of gas limits, `bytes` long once
    /// signed, pays bakers that run the default filter, in mutez.
    pub(crate) fn minimal_fee(&self, gas: u64, bytes: u64) -> Option<u64> {
        let nanotez = u128::from(gas)
            .checked_mul(u128::from(self.nanotez_per_gas))?
            .checked_add(u128::from(bytes).checked_mul(u128::from(self.nanotez_per_byte))?)?;
        u64::try_from(nanotez.div_ceil(1_000))
            .ok()?
            .checked_add(self.minimal_fee_mutez)
    }
}
