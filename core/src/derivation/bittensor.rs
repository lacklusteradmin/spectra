//! Bittensor: SS58 address validation, BIP-39 → substrate-bip39 PBKDF2
//! mini-secret → sr25519 derivation, SS58 v1 address encoding.
//!
//!
//! Bittensor wallets use the substrate-generic SS58 prefix (42), producing
//! addresses that start with `5…` and share the same length as Polkadot's
//! `1…` mainnet addresses. Wire-level the format is identical:
//!   `[prefix(1-2 bytes)] || [pubkey(32)] || [checksum(2)]`, base58-encoded.
//!
//! The 32-byte payload is treated as a sr25519 public key by the runtime;
//! Bittensor does not currently use the optional ECDSA path that the
//! generic SS58 envelope reserves.
//!
//! A derivation path is Substrate's hard (`//`) and soft (`/`) junctions
//! (`derivation::substrate_path`); none derives the root sr25519 keypair.

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{decode_ss58, derive_substrate_sr25519_material, encode_ss58};
use crate::derivation::types::{DerivationResult, Wants};

// Decode a Bittensor SS58 address and return the inner 32-byte sr25519 public key.
pub(crate) fn decode_bittensor_ss58(address: &str) -> Result<[u8; 32], DerivationError> {
    decode_ss58(address, Some(42)).map(|(_, key)| key)
}

/// True if address is a valid Bittensor SS58 address.
pub fn validate_bittensor_address(address: &str) -> bool {
    decode_bittensor_ss58(address).is_ok()
}

/// Derive a Bittensor wallet (address, public key, and signing key: a seed,
/// or a soft junction's expanded key) from a seed phrase along a Substrate
/// junction path.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let (key, public_key) = derive_substrate_sr25519_material(
        seed_phrase,
        passphrase.unwrap_or(""),
        None,
        None,
        0,
        Some(path),
        false,
    )?;
    Ok(DerivationResult {
        address: wants.address.then(|| encode_ss58(&public_key, 42)),
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&key),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage() {
        assert!(!validate_bittensor_address(""));
        assert!(!validate_bittensor_address("not-a-bittensor-address"));
    }
}
