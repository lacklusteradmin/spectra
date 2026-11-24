//! NEAR: BIP-39 + SLIP-10 ed25519 derivation and hex implicit-account
//! encoding.
//!
//! NEAR's wallets (near-seed-phrase, behind MyNearWallet and Meteor) walk
//! SLIP-10 along `m/44'/397'/{account}'`; the implicit account is the hex of
//! the public key.

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{derive_bip39_seed, derive_slip10_ed25519_key};
use crate::derivation::types::{DerivationResult, Wants};
use ed25519_dalek::SigningKey;

/// Derive a NEAR wallet from a seed phrase: BIP-39 seed → SLIP-10 ed25519 key
/// along `derivation_path`; the address is hex(pubkey) on every network.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, None)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    Ok(DerivationResult {
        address: wants.address.then(|| hex::encode(public_key)),
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&*private_key),
    })
}
