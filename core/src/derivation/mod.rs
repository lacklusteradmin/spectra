//! Cryptographic key + address derivation for every supported chain.
//!
//! Layout: `chains/<chain>.rs` is the leaf — each file owns its full
//! derivation pipeline (BIP-39, the relevant curve walk, and the
//! chain-specific address encoder). `engine.rs` owns the FFI surface,
//! request/response types, validation, and the dispatch table that fans
//! out per chain. The address validator and xpub walker reach directly
//! into `chains::bitcoin` for shared BIP-32 / xpub / address-parsing
//! helpers.

pub mod chains;
pub mod import;
pub mod validation;
pub mod xpub_walker;

pub(crate) mod engine;

mod presets;

#[cfg(test)]
mod tests;

// Re-export the FFI-facing types and entry points so callers outside
// `derivation/` keep their existing import paths.
pub use engine::{
    derivation_build_material, derivation_build_material_from_private_key, derivation_derive,
    derivation_derive_all_addresses, derivation_derive_from_private_key, UniFFIDerivationRequest,
    UniFFIDerivationResponse, UniFFIMaterialRequest, UniFFIMaterialResponse,
    UniFFIPrivateKeyDerivationRequest, UniFFIPrivateKeyMaterialRequest,
};
pub(crate) use engine::derive_key_material_for_chain_with_overrides;

// Back-compat aliases. Some modules still refer to
// `crate::derivation::wire::*` and `::enums::*`; both are now embedded
// in `engine.rs` but the old paths still resolve.
pub(crate) use engine as wire;
pub(crate) use engine as enums;

// File renames from the prior restructure round.
pub use validation as addressing;
pub use xpub_walker as utxo_hd;
