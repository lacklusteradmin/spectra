//! Cryptographic key + address derivation for every supported chain.
//!
//! Layout: `<chain>.rs` is the leaf — each file owns its full
//! derivation pipeline (BIP-39, the relevant curve walk, the chain-specific
//! address encoder, and the UniFFI export surface).

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
pub mod evm;
pub mod funds_finder;
pub mod icp;
pub mod import;
pub mod input;
pub mod kaspa;
pub mod litecoin;
pub mod monero;
pub mod near;
pub mod polkadot;
pub mod primitives;
pub mod solana;
pub mod stellar;
pub mod sui;
pub mod ton;
pub(crate) mod ton_cell;
pub mod tron;
pub mod types;
pub mod xpub_walker;
pub mod xrp;
pub mod zcash;

#[cfg(test)]
mod tests;
