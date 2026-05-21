//! NEAR: account-id validation (named + implicit hex), BIP-39 + direct-seed
//! ed25519 derivation, hex address encoding
//!
//! NEAR uses *direct-seed* ed25519: the BIP-39 seed's first 32 bytes are the
//! ed25519 private key — no SLIP-10 path walk. Address = hex(public_key).

use crate::derivation::primitives::derive_bip39_seed;
use ed25519_dalek::SigningKey;
use zeroize::Zeroizing;

/// BIP-39 seed first 32 bytes → ed25519 keypair; NEAR address = hex(pubkey) (no path walk).
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, String> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let mut private_key = Zeroizing::new([0u8; 32]);
    private_key.copy_from_slice(&seed[..32]);
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    Ok((
        want_address.then(|| hex::encode(public_key)),
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(*private_key)),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::DerivationResult;
use crate::SpectraBridgeError;

// Shared body for derive_near / derive_near_testnet.
fn near_internal(
    seed_phrase: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        &seed_phrase,
        passphrase.as_deref(),
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

/// UniFFI export: derive NEAR mainnet keys (direct-seed ed25519; address = hex pubkey).
#[uniffi::export]
pub fn derive_near(
    seed_phrase: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    near_internal(
        seed_phrase,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// UniFFI export: derive NEAR testnet keys (identical derivation to mainnet).
#[uniffi::export]
pub fn derive_near_testnet(
    seed_phrase: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    near_internal(
        seed_phrase,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}
