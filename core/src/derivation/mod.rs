//! Cryptographic key + address derivation for every supported chain.
//!
//! Chain modules implement key and address derivation; typed dispatch
//! selects the implementation for each network.

pub(crate) mod account_key;
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
pub(crate) mod key_formats;
pub mod litecoin;
pub mod monero;
pub(crate) mod monero_words;
pub(crate) mod multisig;
pub mod near;
pub mod path;
pub mod peercoin;
pub(crate) mod phrase;
pub mod polkadot;
pub(crate) mod polyseed;
pub mod primitives;
mod private_key;
pub mod setup;
pub mod solana;
pub mod stellar;
pub(crate) mod substrate_path;
pub mod sui;
pub mod ton;
pub(crate) mod ton_cell;
pub mod tron;
pub mod types;
pub(crate) mod utxo_address;
pub mod xrp;
pub mod zcash;

#[cfg(test)]
mod tests;
