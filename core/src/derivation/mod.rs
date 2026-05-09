//! Cryptographic key + address derivation for every supported chain.
//!
//! Layout: `chains/<chain>.rs` is the leaf — each file owns its full
//! derivation pipeline (BIP-39, the relevant curve walk, the chain-specific
//! address encoder, and the UniFFI export surface).

pub mod chains;
pub mod funds_finder;
pub mod import;
pub mod types;
pub mod validation;
pub mod xpub_walker;

#[cfg(test)]
mod tests;

// File renames from the prior restructure round.
pub use validation as addressing;
pub use xpub_walker as utxo_hd;
