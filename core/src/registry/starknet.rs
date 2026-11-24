//! Starknet: the network a transaction hash binds, the contract that holds
//! its own coin, and the account contract Spectra's wallets are.
//!
//! A Starknet account is a contract, so a key alone is not an address: the
//! address is the hash of the account class, its constructor's arguments and
//! a salt. Spectra's accounts are Ready's (formerly Argent X), the most used
//! Starknet wallet: Argent account 0.4.0, owned by one Stark key with no
//! guardian, deployed by the account itself. The sources and the dated
//! readings behind each value are in `docs/audits/starknet-2026-11-24.md`.

use super::*;

/// One Starknet network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StarknetNetwork {
    /// The chain id every transaction hash binds, as the short string
    /// `starknet_chainId` answers in hex.
    pub chain_id: &'static str,
    /// The ERC-20 contract that holds the network's own coin, STRK, which
    /// pays every v3 transaction's fee.
    pub fee_token: &'static str,
    /// The class a new account is deployed with and the one whose
    /// signature and calldata layout signing writes: Argent account 0.4.0,
    /// the latest class Ready deploys (`TXV3_ACCOUNT_CLASS_HASH` in
    /// @argent/x-shared 1.79.4).
    pub account_class_hash: &'static str,
}

const MAINNET: StarknetNetwork = StarknetNetwork {
    chain_id: "SN_MAIN",
    fee_token: "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d",
    account_class_hash: "0x036078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f",
};

/// How much a transaction's resource bounds exceed what an estimate used,
/// in percent, on each resource's amount and on its price: what starknet.js
/// and the wallets built on it add. The network charges what is used, at
/// most the bounds.
pub(crate) const FEE_HEADROOM_PERCENT: u128 = 50;

impl Chain {
    /// This network's Starknet facts, or `None` outside Starknet.
    pub(crate) fn starknet(self) -> Option<&'static StarknetNetwork> {
        match self {
            Self::Starknet => Some(&MAINNET),
            _ => None,
        }
    }

    /// Whether this is a Starknet network, read through
    /// `api::starknet_json_rpc` and signed by `send::starknet`.
    pub fn is_starknet(self) -> bool {
        self.starknet().is_some()
    }

    /// [`Chain::starknet`], or the refusal a caller outside Starknet gets.
    pub(crate) fn starknet_network(self) -> Result<&'static StarknetNetwork, RegistryError> {
        self.starknet().ok_or_else(|| self.not_in("Starknet"))
    }
}
