//! Tron: address validation, BIP-32 derivation, base58check encoding with
//! 0x41 prefix
//!
//! Tron's address derivation:
//!   keccak256(uncompressed_pubkey[1..])[12..32]  → 20-byte EVM-style hash
//!   prepend 0x41                                  → 21-byte payload
//!   base58check (default alphabet)                → "T…" address

use crate::derivation::error::DerivationError;

use crate::derivation::types::{DerivationResult, Wants};
use secp256k1::{PublicKey, Secp256k1};

// ── Address validation + helpers (preserved) ─────────────────────────────

/// Decode a Tron base58check address and return the 20-byte EVM-style hex account hash (without 0x41 prefix).
pub fn tron_base58_to_evm_hex(address: &str) -> Result<String, DerivationError> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| DerivationError::Invalid(format!("base58 decode: {e}").into()))?;
    if decoded.len() != 21 || decoded[0] != 0x41 {
        return Err(DerivationError::Invalid(
            format!("invalid Tron address length/prefix: len={}", decoded.len()).into(),
        ));
    }
    Ok(hex::encode(&decoded[1..]))
}

/// Encode a node API's hex address (`41` followed by the 20-byte account
/// hash) as the base58check "T…" form.
pub fn tron_hex_to_base58(hex_address: &str) -> Result<String, DerivationError> {
    let payload = hex::decode(hex_address)
        .map_err(|e| DerivationError::Invalid(format!("Tron hex address: {e}").into()))?;
    if payload.len() != 21 || payload[0] != 0x41 {
        return Err(DerivationError::Invalid(
            format!(
                "invalid Tron hex address length/prefix: len={}",
                payload.len()
            )
            .into(),
        ));
    }
    Ok(bs58::encode(&payload).with_check().into_string())
}

// Keccak-256 hash; used for Tron address derivation.
fn keccak256(data: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    Keccak256::digest(data).into()
}

pub(crate) fn address_from_public_key(key: &PublicKey) -> String {
    let hash = keccak256(&key.serialize_uncompressed()[1..]);
    let mut payload = vec![0x41];
    payload.extend_from_slice(&hash[12..]);
    bs58::encode(&payload).with_check().into_string()
}

/// Derive a Tron wallet from a seed phrase via BIP-39 + BIP-32 secp256k1.
/// Every Tron network has the same addresses.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let secp = Secp256k1::new();
    let private_key =
        crate::derivation::bip32::phrase_key(seed_phrase, passphrase, derivation_path)?;
    let public_key = PublicKey::from_secret_key(&secp, &private_key);
    let private_bytes = private_key.secret_bytes();

    Ok(DerivationResult {
        address: wants.address.then(|| address_from_public_key(&public_key)),
        public_key_hex: wants.public_key_hex(&public_key.serialize()),
        private_key_hex: wants.private_key_hex(&private_bytes),
    })
}
