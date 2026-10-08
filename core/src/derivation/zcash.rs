//! Zcash transparent: BIP-32 derivation and t1… P2PKH base58check encoding
//! (2-byte version prefix). Every Zcash address form, shielded ones too, is
//! validated by `validation::address` through librustzcash.

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::{base58check_encode, derive_secp_keypair, hash160};
use crate::derivation::types::{DerivationResult, parse_path_metadata};

const ZCASH_MAINNET_VERSION: [u8; 2] = [0x1C, 0xB8];
const ZCASH_TESTNET_VERSION: [u8; 2] = [0x1D, 0x25];

// Build a Zcash transparent P2PKH address from a 2-byte version prefix and compressed pubkey.
fn zcash_p2pkh_addr(version: [u8; 2], pubkey: &secp256k1::PublicKey) -> String {
    let mut payload = vec![version[0], version[1]];
    payload.extend_from_slice(&hash160(&pubkey.serialize()));
    base58check_encode(&payload)
}

// Shared body for derive_zcash / derive_zcash_testnet; builds transparent P2PKH address.
fn zcash_internal(
    version: [u8; 2],
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (pk, priv_bytes) =
        derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    Ok(DerivationResult {
        address: want_address.then(|| zcash_p2pkh_addr(version, &pk)),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(priv_bytes)),
        account,
        branch,
        index,
    })
}

/// Derive Zcash mainnet transparent keys.
pub fn derive_zcash(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    zcash_internal(
        ZCASH_MAINNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive Zcash testnet transparent keys.
pub fn derive_zcash_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    zcash_internal(
        ZCASH_TESTNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}
