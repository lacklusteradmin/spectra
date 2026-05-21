//! Polkadot: SS58 address validation, BIP-39 → substrate-bip39 PBKDF2
//! mini-secret → sr25519 derivation, SS58 v1 address encoding.
//!
//!
//! - SS58 prefix: 0 = `Chain::Polkadot` (mainnet, addresses start with `1…`),
//!   42 = `Chain::PolkadotWestend` (testnet, addresses start with `5…`).
//! - Substrate junction derivation (`//hard`, `/soft`) is not yet supported —
//!   omit the derivation path to derive the root sr25519 keypair.

use crate::derivation::primitives::{
    decode_ss58 as decode_ss58_with_prefix, derive_substrate_sr25519_material, encode_ss58,
};

// Decode a Polkadot/Substrate SS58 address and return the inner 32-byte public key.
pub(crate) fn decode_ss58(address: &str) -> Result<[u8; 32], String> {
    decode_ss58_with_prefix(address, None).map(|(_, key)| key)
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::DerivationResult;
use crate::SpectraBridgeError;

// Shared derivation logic for all Substrate-based networks; ss58_prefix selects the network.
fn substrate_internal(
    ss58_prefix: u16,
    seed_phrase: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let uniform_expansion = hmac_key.as_deref() == Some("uniform");
    let (mini_secret, public_key) = derive_substrate_sr25519_material(
        &seed_phrase,
        passphrase.as_deref().unwrap_or(""),
        None,
        None,
        0,
        None,
        uniform_expansion,
    )?;
    Ok(DerivationResult {
        address: want_address.then(|| encode_ss58(&public_key, ss58_prefix)),
        public_key_hex: want_public_key.then(|| hex::encode(public_key)),
        private_key_hex: want_private_key.then(|| hex::encode(mini_secret)),
        account: 0,
        branch: 0,
        index: 0,
    })
}

/// UniFFI export: derive Polkadot mainnet wallet (SS58 prefix 0, "1…" addresses).
#[uniffi::export]
pub fn derive_polkadot(
    seed_phrase: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    substrate_internal(
        0,
        seed_phrase,
        passphrase,
        hmac_key,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// UniFFI export: derive Polkadot Westend testnet wallet (SS58 prefix 42, "5…" addresses).
#[uniffi::export]
pub fn derive_polkadot_westend(
    seed_phrase: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    substrate_internal(
        42,
        seed_phrase,
        passphrase,
        hmac_key,
        want_address,
        want_public_key,
        want_private_key,
    )
}
