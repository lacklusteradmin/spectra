//! XRP Ledger: address validation, BIP-32 derivation, base58check with the
//! XRP/Ripple alphabet
//!
//! Address derivation: `0x00 || hash160(compressed_pubkey)` then base58check
//! with the Ripple alphabet (`rpshnaf3…`).

use crate::derivation::primitives::{derive_bip39_seed, parse_bip32_path};
use ripemd::Ripemd160;
use secp256k1::{PublicKey, Secp256k1};
use sha2::{Digest, Sha256};

const XRP_ALPHABET_BYTES: &[u8; 58] = b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz";

// ── Address validation (preserved) ───────────────────────────────────────

// Decode an XRP address using the Ripple base58 alphabet; returns the 20-byte account ID.
pub(crate) fn decode_xrp_address(address: &str) -> Result<Vec<u8>, String> {
    let alphabet = bs58::Alphabet::new(XRP_ALPHABET_BYTES).map_err(|e| format!("alphabet: {e}"))?;
    let decoded = bs58::decode(address)
        .with_alphabet(&alphabet)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("xrp address decode: {e}"))?;
    if decoded.len() != 21 {
        return Err(format!("xrp address length: {}", decoded.len()));
    }
    if decoded[0] != 0x00 {
        return Err(format!("xrp address version: 0x{:02x}", decoded[0]));
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

// Derive XRP address, public key, and private key from a mnemonic via BIP-39 + BIP-32 secp256k1.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, String> {
    let secp = Secp256k1::new();
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let master = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())?;
    let path = parse_bip32_path(derivation_path)?;
    let xpriv = master.derive_path(&secp, &path)?;
    let public_key = PublicKey::from_secret_key(&secp, &xpriv.private_key);
    let private_bytes = xpriv.private_key.secret_bytes();

    let address = if want_address {
        let mut payload = vec![0x00u8];
        payload.extend_from_slice(&hash160_bytes(&public_key.serialize()));
        let alphabet =
            bs58::Alphabet::new(XRP_ALPHABET_BYTES).map_err(|e| format!("xrp alphabet: {e}"))?;
        Some(
            bs58::encode(&payload)
                .with_alphabet(&alphabet)
                .with_check()
                .into_string(),
        )
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::{parse_path_metadata, DerivationResult};
use crate::SpectraBridgeError;
use crate::derivation::primitives::ExtendedPrivateKey;

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

/// UniFFI export: derive XRP Ledger mainnet wallet from a seed phrase.
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

/// UniFFI export: derive XRP Ledger testnet wallet from a seed phrase.
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
