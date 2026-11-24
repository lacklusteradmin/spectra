//! Polkadot: SS58 address validation, BIP-39 → substrate-bip39 PBKDF2
//! mini-secret → sr25519 derivation, SS58 v1 address encoding.
//!
//!
//! - SS58 prefix: 0 = `Chain::Polkadot` (mainnet, addresses start with `1…`),
//!   42 = `Chain::PolkadotWestend` (testnet, addresses start with `5…`).
//! - A derivation path is Substrate's hard (`//`) and soft (`/`) junctions
//!   (`derivation::substrate_path`); none derives the root sr25519 keypair.

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{
    decode_ss58 as decode_ss58_with_prefix, derive_substrate_sr25519_material, encode_ss58,
};

// Decode a Polkadot/Substrate SS58 address and return the inner 32-byte public key.
pub(crate) fn decode_ss58(address: &str) -> Result<[u8; 32], DerivationError> {
    decode_ss58_with_prefix(address, None).map(|(_, key)| key)
}

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::derivation::types::{DerivationResult, Wants};
use crate::registry::Chain;

/// Derive a Polkadot wallet from a seed phrase along a Substrate junction
/// path; the address carries `chain`'s SS58 prefix.
pub(crate) fn derive_from_seed_phrase(
    chain: Chain,
    seed_phrase: &str,
    path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let ss58_prefix = chain
        .ss58_prefix()
        .ok_or_else(|| DerivationError::invalid(format!("{chain} is not a Substrate network")))?;
    // The override's one meaning here: `uniform` expands the root seed in
    // schnorrkel's Uniform mode. Any other value would be ignored, so it is
    // refused rather than deriving the key it does not change.
    let uniform_expansion = match hmac_key.map(str::trim) {
        None | Some("") => false,
        Some("uniform") => true,
        Some(_) => {
            return Err(DerivationError::refused(
                "%@ takes only `uniform` as its HMAC override.",
                ["Polkadot"],
            ));
        }
    };
    let (key, public_key) = derive_substrate_sr25519_material(
        seed_phrase,
        passphrase.unwrap_or(""),
        None,
        None,
        0,
        Some(path),
        uniform_expansion,
    )?;
    Ok(DerivationResult {
        address: wants.address.then(|| encode_ss58(&public_key, ss58_prefix)),
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&key),
    })
}
