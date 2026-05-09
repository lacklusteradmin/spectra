//! Bitcoin Cash: address validation, BIP-39 + BIP-32 derivation, legacy P2PKH
//! base58check encoding.


// ── Address validation (preserved) ───────────────────────────────────────

// Strip the "bitcoincash:" prefix if present, returning just the payload string.
pub(crate) fn normalize_bch_address(addr: &str) -> String {
    addr.strip_prefix("bitcoincash:")
        .unwrap_or(addr)
        .to_string()
}

// Base58check-decode a BCH address (with or without "bitcoincash:" prefix) into the 20-byte hash.
pub(crate) fn decode_bch_to_hash20(address: &str) -> Result<[u8; 20], String> {
    let norm = normalize_bch_address(address);
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        if decoded.len() == 21 {
            let mut hash = [0u8; 20];
            hash.copy_from_slice(&decoded[1..21]);
            return Ok(hash);
        }
    }
    Err(format!("cannot decode BCH address: {address}"))
}

// ── Derivation ────────────────────────────────────────────────────────────

use secp256k1::{PublicKey, Secp256k1, SecretKey};
use crate::derivation::types::{DerivationResult, parse_path_metadata};
use crate::derivation::chains::bitcoin::{base58check_encode, derive_secp_keypair, hash160};
use crate::SpectraBridgeError;

const BCH_MAINNET_VERSION: u8 = 0x00;
const BCH_TESTNET_VERSION: u8 = 0x6f;

// Build a BCH P2PKH address: base58check(version || hash160(pubkey)).
fn p2pkh_address(version: u8, pubkey: &PublicKey) -> String {
    let mut payload = vec![version];
    payload.extend_from_slice(&hash160(&pubkey.serialize()));
    base58check_encode(&payload)
}

/// BIP-39 → secp256k1 keypair → BCH mainnet P2PKH address.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str, derivation_path: &str, passphrase: Option<&str>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (pk, priv_bytes) = derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    Ok((
        want_address.then(|| p2pkh_address(BCH_MAINNET_VERSION, &pk)),
        want_public_key.then(|| hex::encode(pk.serialize())),
        want_private_key.then(|| hex::encode(priv_bytes)),
    ))
}

/// BIP-39 → secp256k1 keypair → BCH testnet P2PKH address.
pub(crate) fn derive_from_seed_phrase_testnet(
    seed_phrase: &str, derivation_path: &str, passphrase: Option<&str>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (pk, priv_bytes) = derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    Ok((
        want_address.then(|| p2pkh_address(BCH_TESTNET_VERSION, &pk)),
        want_public_key.then(|| hex::encode(pk.serialize())),
        want_private_key.then(|| hex::encode(priv_bytes)),
    ))
}

// Shared body for derive_bitcoin_cash / derive_bitcoin_cash_testnet; rejects non-P2PKH script types.
fn bch_internal(
    version: u8, seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: crate::derivation::types::BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    use crate::derivation::types::BitcoinScriptType;
    if !matches!(script_type, BitcoinScriptType::P2pkh) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "Bitcoin Cash only supports P2PKH addresses.".into(),
        });
    }
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (pk, priv_bytes) = derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    Ok(DerivationResult {
        address: want_address.then(|| p2pkh_address(version, &pk)),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(priv_bytes)),
        account, branch, index,
    })
}

/// UniFFI export: derive Bitcoin Cash mainnet keys (P2PKH only).
#[uniffi::export]
pub fn derive_bitcoin_cash(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: crate::derivation::types::BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bch_internal(BCH_MAINNET_VERSION, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Bitcoin Cash testnet keys (P2PKH only).
#[uniffi::export]
pub fn derive_bitcoin_cash_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: crate::derivation::types::BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bch_internal(BCH_TESTNET_VERSION, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Bitcoin Cash address/pubkey directly from a hex private key.
#[uniffi::export]
pub fn derive_bitcoin_cash_from_private_key(
    private_key_hex: String, want_address: bool, want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let trimmed = private_key_hex.trim();
    if trimmed.len() != 64 {
        return Err(SpectraBridgeError::InvalidInput {
            message: "Private key hex must be exactly 64 characters.".into(),
        });
    }
    let bytes = hex::decode(trimmed)?;
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&bytes);
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(&key_bytes).map_err(|e| e.to_string())?;
    let pk = PublicKey::from_secret_key(&secp, &secret_key);
    Ok(DerivationResult {
        address: want_address.then(|| p2pkh_address(BCH_MAINNET_VERSION, &pk)),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: None,
        account: 0, branch: 0, index: 0,
    })
}
