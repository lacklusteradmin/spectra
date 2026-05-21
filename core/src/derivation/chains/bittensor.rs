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
//! Substrate junction derivation (`//hard`, `/soft`) is not yet supported —
//! omit the derivation path to derive the root sr25519 keypair.

use crate::derivation::primitives::{
    decode_ss58, derive_substrate_sr25519_material, encode_ss58, OptionalKeyMaterial,
};

// Decode a Bittensor SS58 address and return the inner 32-byte sr25519 public key.
pub(crate) fn decode_bittensor_ss58(address: &str) -> Result<[u8; 32], String> {
    decode_ss58(address, Some(42)).map(|(_, key)| key)
}

/// True if address is a valid Bittensor SS58 address.
pub fn validate_bittensor_address(address: &str) -> bool {
    decode_bittensor_ss58(address).is_ok()
}

// Derive Bittensor address, public key, and mini-secret hex from a mnemonic seed phrase.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    passphrase: Option<&str>,
    path: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<OptionalKeyMaterial, String> {
    let (mini_secret, public_key) = derive_substrate_sr25519_material(
        seed_phrase,
        passphrase.unwrap_or(""),
        None,
        None,
        0,
        path,
        false,
    )?;
    Ok((
        want_address.then(|| encode_ss58(&public_key, 42)),
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(mini_secret)),
    ))
}

// ── UniFFI export ─────────────────────────────────────────────────────────

use crate::derivation::types::DerivationResult;
use crate::SpectraBridgeError;

/// UniFFI export: derive Bittensor wallet (address, public key, mini-secret) from a seed phrase.
#[uniffi::export]
pub fn derive_bittensor(
    seed_phrase: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        &seed_phrase,
        passphrase.as_deref(),
        None,
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex,
        account: 0,
        branch: 0,
        index: 0,
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
