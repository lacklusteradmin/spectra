//! Solana: address validation, BIP-39 + SLIP-10 ed25519 derivation,
//! base58 pubkey encoding

use crate::derivation::primitives::derive_bip39_seed;
use ed25519_dalek::SigningKey;

// Decode a base58 string and assert it is exactly 32 bytes (used for Solana pubkeys).
pub(crate) fn decode_b58_32(b58: &str) -> Result<[u8; 32], String> {
    let bytes = bs58::decode(b58)
        .into_vec()
        .map_err(|e| format!("b58 decode {b58}: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("b58 {b58} not 32 bytes: {}", v.len()))
}


// ── HMAC-SHA512 + SLIP-10 ed25519 ────────────────────────────────────────

/// BIP-39 → SLIP-10 ed25519 → Solana address (base58 pubkey).
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, String> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, hmac_key)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    Ok((
        want_address.then(|| bs58::encode(public_key).into_string()),
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(*private_key)),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::{parse_path_metadata, DerivationResult};
use crate::SpectraBridgeError;
use crate::derivation::primitives::derive_slip10_ed25519_key;

// Shared body for derive_solana / derive_solana_devnet.
fn solana_internal(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        &seed_phrase,
        &derivation_path,
        passphrase.as_deref(),
        hmac_key.as_deref(),
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex,
        account,
        branch,
        index,
    })
}

/// UniFFI export: derive Solana mainnet keys from a BIP-39 seed phrase.
pub fn derive_solana(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    solana_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        hmac_key,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// UniFFI export: derive Solana devnet keys (identical derivation to mainnet).
pub fn derive_solana_devnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    solana_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        hmac_key,
        want_address,
        want_public_key,
        want_private_key,
    )
}
