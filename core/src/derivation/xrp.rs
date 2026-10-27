//! XRP Ledger: address validation, BIP-32 derivation, base58check with the
//! XRP/Ripple alphabet
//!
//! Address derivation: `0x00 || hash160(compressed_pubkey)` then base58check
//! with the Ripple alphabet (`rpshnaf3…`).

use crate::derivation::error::DerivationError;

use ripemd::Ripemd160;
use secp256k1::{PublicKey, Secp256k1};
use sha2::{Digest, Sha256};

const XRP_ALPHABET_BYTES: &[u8; 58] = b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz";

// ── Address validation (preserved) ───────────────────────────────────────

// Decode an XRP address using the Ripple base58 alphabet; returns the 20-byte account ID.
pub(crate) fn decode_xrp_address(address: &str) -> Result<Vec<u8>, DerivationError> {
    let alphabet = bs58::Alphabet::new(XRP_ALPHABET_BYTES)
        .map_err(|e| DerivationError::Invalid(format!("alphabet: {e}").into()))?;
    let decoded = bs58::decode(address)
        .with_alphabet(&alphabet)
        .with_check(None)
        .into_vec()
        .map_err(|e| DerivationError::Invalid(format!("xrp address decode: {e}").into()))?;
    if decoded.len() != 21 {
        return Err(DerivationError::Invalid(
            format!("xrp address length: {}", decoded.len()).into(),
        ));
    }
    if decoded[0] != 0x00 {
        return Err(DerivationError::Invalid(
            format!("xrp address version: 0x{:02x}", decoded[0]).into(),
        ));
    }
    Ok(decoded[1..].to_vec())
}

// ── Hashing primitives ───────────────────────────────────────────────────

// RIPEMD-160(SHA-256(bytes)) — the XRP address hash primitive.
fn hash160_bytes(bytes: &[u8]) -> [u8; 20] {
    let sha = {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let out = hasher.finalize();
        let mut result = [0u8; 32];
        result.copy_from_slice(&out);
        result
    };
    let mut hasher = Ripemd160::new();
    hasher.update(sha);
    let out = hasher.finalize();
    let mut result = [0u8; 20];
    result.copy_from_slice(&out);
    result
}

pub(crate) fn address_from_public_key(key: &PublicKey) -> Result<String, DerivationError> {
    address_of_account_id(&hash160_bytes(&key.serialize()))
}

/// The classic address of a 20-byte account id.
pub(crate) fn address_of_account_id(account_id: &[u8; 20]) -> Result<String, DerivationError> {
    let mut payload = vec![0x00];
    payload.extend_from_slice(account_id);
    let alphabet = bs58::Alphabet::new(XRP_ALPHABET_BYTES)
        .map_err(|e| DerivationError::invalid(format!("xrp alphabet: {e}")))?;
    Ok(bs58::encode(payload)
        .with_alphabet(&alphabet)
        .with_check()
        .into_string())
}

/// The account id a secp256k1 public key's own account has.
pub(crate) fn account_id_of_key(key: &PublicKey) -> [u8; 20] {
    hash160_bytes(&key.serialize())
}

// Derive XRP address, public key, and private key from a mnemonic via BIP-39 + BIP-32 secp256k1.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, DerivationError> {
    let secp = Secp256k1::new();
    let private_key =
        crate::derivation::bip32::phrase_key(seed_phrase, passphrase, derivation_path)?;
    let public_key = PublicKey::from_secret_key(&secp, &private_key);
    let private_bytes = private_key.secret_bytes();

    let address = if want_address {
        Some(address_from_public_key(&public_key)?)
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::derivation::types::{DerivationResult, parse_path_metadata};
use crate::error::SpectraBridgeError;

// Shared derivation logic for all XRP Ledger networks.
fn xrp_internal(
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

/// Derive XRP Ledger mainnet wallet from a seed phrase.
pub fn derive_xrp(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    xrp_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive XRP Ledger testnet wallet from a seed phrase.
pub fn derive_xrp_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    xrp_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}
