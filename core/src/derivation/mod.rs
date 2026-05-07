//! Cryptographic key + address derivation for every supported chain.
//!
//! Layout: `chains/<chain>.rs` is the leaf — each file owns its full
//! derivation pipeline (BIP-39, the relevant curve walk, and the
//! chain-specific address encoder). `engine.rs` owns validation and the
//! internal dispatch table used by tests and the signing pipeline.
//! `api/` owns the named per-chain UniFFI exports.

pub mod api;
pub mod chains;
pub mod import;
pub mod validation;
pub mod xpub_walker;

#[cfg(test)]
mod tests;

// File renames from the prior restructure round.
pub use validation as addressing;
pub use xpub_walker as utxo_hd;
