//! Solana: address validation, BIP-39 + SLIP-10 ed25519 derivation,
//! base58 pubkey encoding

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{derive_bip39_seed, derive_slip10_ed25519_key};
use crate::derivation::types::{DerivationResult, Wants};
use ed25519_dalek::SigningKey;

// Decode a base58 string and assert it is exactly 32 bytes (used for Solana pubkeys).
pub(crate) fn decode_b58_32(b58: &str) -> Result<[u8; 32], DerivationError> {
    let bytes = bs58::decode(b58)
        .into_vec()
        .map_err(|e| DerivationError::Invalid(format!("b58 decode {b58}: {e}").into()))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        DerivationError::Invalid(format!("b58 {b58} not 32 bytes: {}", v.len()).into())
    })
}

// ── HMAC-SHA512 + SLIP-10 ed25519 ────────────────────────────────────────

/// Derive a Solana wallet from a seed phrase: BIP-39 → SLIP-10 ed25519 →
/// address (base58 pubkey), the same on every Solana network.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, hmac_key)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    Ok(DerivationResult {
        address: wants
            .address
            .then(|| bs58::encode(public_key).into_string()),
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&*private_key),
    })
}
