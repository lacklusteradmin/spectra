//! Internet Computer (ICP): address validation, BIP-39 + SLIP-10 ed25519
//! derivation, double-SHA-256 address encoding
//!
//! Derived address: `hex(sha256(sha256(pubkey || "icp")))`.

use crate::derivation::primitives::derive_bip39_seed;
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};


// ── SLIP-10 ed25519 ──────────────────────────────────────────────────────

// SHA-256 of the input; a helper to avoid repeated Sha256::new() boilerplate.
fn sha256_bytes(input: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let out = hasher.finalize();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

/// BIP-39 → SLIP-10 ed25519 → ICP address: hex(SHA-256(SHA-256(pubkey || "icp"))).
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, String> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, None)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    let address = if want_address {
        let mut data = Vec::from(public_key);
        data.extend_from_slice(b"icp");
        let digest = sha256_bytes(&data);
        let digest2 = sha256_bytes(&digest);
        Some(hex::encode(digest2))
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(*private_key)),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::{parse_path_metadata, DerivationResult};
use crate::SpectraBridgeError;
use crate::derivation::primitives::derive_slip10_ed25519_key;

/// UniFFI export: derive Internet Computer keys from a BIP-39 seed phrase.
pub fn derive_icp(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
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
        account,
        branch,
        index,
    })
}
