//! Aptos: address validation, BIP-39 + SLIP-10 ed25519 derivation, hex
//! address encoding

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{derive_bip39_seed, derive_slip10_ed25519_key};
use crate::derivation::types::{DerivationResult, Wants};
use ed25519_dalek::SigningKey;

pub(crate) fn address_from_public_key(public_key: &[u8; 32]) -> String {
    use sha3::{Digest, Sha3_256};
    let digest = Sha3_256::new()
        .chain_update(public_key)
        .chain_update([0x00])
        .finalize();
    format!("0x{}", hex::encode(digest))
}

// ── SLIP-10 ed25519 ──────────────────────────────────────────────────────

/// Derive an Aptos wallet from a seed phrase: BIP-39 seed → SLIP-10 ed25519
/// key → address (Sha3_256 with 0x00 auth tag). Every Aptos network has the
/// same addresses.
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
        address: wants.address.then(|| address_from_public_key(&public_key)),
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&*private_key),
    })
}
