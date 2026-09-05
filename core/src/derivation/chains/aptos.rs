//! Aptos: address validation, BIP-39 + SLIP-10 ed25519 derivation, hex
//! address encoding

use crate::derivation::primitives::derive_bip39_seed;
use ed25519_dalek::SigningKey;


// ── SLIP-10 ed25519 ──────────────────────────────────────────────────────

/// Full pipeline: BIP-39 seed → SLIP-10 ed25519 key → Aptos address (Keccak256 with 0x00 auth tag).
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
        use sha3::{Digest, Keccak256};
        let digest: [u8; 32] = Keccak256::new()
            .chain_update(public_key)
            .chain_update([0x00])
            .finalize()
            .into();
        Some(format!("0x{}", hex::encode(digest)))
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

// Shared body for derive_aptos / derive_aptos_testnet: runs derivation and packs DerivationResult.
fn aptos_internal(
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

/// UniFFI export: derive Aptos mainnet keys from a BIP-39 seed phrase.
pub fn derive_aptos(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    aptos_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// UniFFI export: derive Aptos testnet keys (identical derivation to mainnet; network differs at RPC layer).
pub fn derive_aptos_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    aptos_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}
