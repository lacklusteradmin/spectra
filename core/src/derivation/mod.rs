//! Cryptographic key + address derivation for every supported chain.
//!
//! Chain modules implement key and address derivation; typed dispatch
//! selects the implementation for each network.

pub mod aptos;
pub mod bitcoin;
pub mod bitcoin_cash;
pub mod bitcoin_gold;
pub mod bitcoin_sv;
pub mod bittensor;
pub mod cardano;
pub mod dash;
pub mod decred;
pub mod dispatch;
pub mod dogecoin;
pub mod error;
pub mod evm;
pub mod funds_finder;
pub mod icp;
pub mod import;
pub mod input;
pub mod kaspa;
pub mod litecoin;
pub mod monero;
pub mod near;
pub mod path;
pub mod polkadot;
pub mod primitives;
pub mod solana;
pub mod stellar;
pub mod sui;
pub mod ton;
pub(crate) mod ton_cell;
pub mod tron;
pub mod types;
pub(crate) mod utxo_address;
pub mod xpub_walker;
pub mod xrp;
pub mod zcash;

#[cfg(test)]
mod tests;
