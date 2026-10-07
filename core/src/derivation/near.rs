//! NEAR: BIP-39 + SLIP-10 ed25519 derivation and hex implicit-account
//! encoding.
//!
//! NEAR's wallets (near-seed-phrase, behind MyNearWallet and Meteor) walk
//! SLIP-10 along `m/44'/397'/{account}'`; the implicit account is the hex of
//! the public key.

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{derive_bip39_seed, derive_slip10_ed25519_key};
use ed25519_dalek::SigningKey;

/// BIP-39 seed → SLIP-10 ed25519 key along `derivation_path`; NEAR address =
/// hex(pubkey).
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, DerivationError> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, None)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    Ok((
        want_address.then(|| hex::encode(public_key)),
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(*private_key)),
    ))
}

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::SpectraBridgeError;
use crate::derivation::types::DerivationResult;

// Shared body for derive_near / derive_near_testnet.
fn near_internal(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        &seed_phrase,
        &derivation_path,
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

/// Derive NEAR mainnet keys (SLIP-10 ed25519; address = hex pubkey).
pub fn derive_near(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    near_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive NEAR testnet keys (identical derivation to mainnet).
pub fn derive_near_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    near_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}
