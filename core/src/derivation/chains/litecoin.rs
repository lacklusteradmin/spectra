//! Litecoin: address validation, P2PKH (L…) base58check encoding,
//! and MWEB stealth address parsing. Self-contained — see `REFACTOR_NOTES.md`.


// ── Address validation ───────────────────────────────────────────────────

// Base58check-decode an LTC address and return the 20-byte pubkey hash.
pub(crate) fn decode_ltc_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid ltc address: {e}"))?;
    if decoded.len() < 21 {
        return Err("address too short".to_string());
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

// Build the standard P2PKH locking script for the given 20-byte pubkey hash.
pub(crate) fn ltc_p2pkh_script(pubkey_hash: &[u8; 20]) -> Result<Vec<u8>, String> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    Ok(s)
}

/// Parsed form of an `ltcmweb1…` or `tmweb1…` stealth address.
/// `scan_pubkey` (A) and `spend_pubkey` (B) are 33-byte compressed secp256k1 points.
#[derive(Debug, Clone)]
pub struct MwebAddress {
    pub scan_pubkey: [u8; 33],
    pub spend_pubkey: [u8; 33],
}

/// Decode a bech32m MWEB address into its constituent scan and spend public keys.
/// Returns an error for non-MWEB addresses or malformed payloads.
/// Decode a bech32m MWEB stealth address into its constituent scan and spend public keys.
pub fn parse_mweb_address(address: &str) -> Result<MwebAddress, String> {
    let (hrp, data) = bech32::decode(address)
        .map_err(|e| format!("invalid mweb address: {e}"))?;
    if hrp.as_str() != "ltcmweb" && hrp.as_str() != "tmweb" {
        return Err(format!(
            "expected ltcmweb or tmweb HRP, got \"{}\"",
            hrp.as_str()
        ));
    }
    if data.len() != 66 {
        return Err(format!(
            "mweb address payload must be 66 bytes (scan+spend pubkeys), got {}",
            data.len()
        ));
    }
    let mut scan_pubkey = [0u8; 33];
    let mut spend_pubkey = [0u8; 33];
    scan_pubkey.copy_from_slice(&data[0..33]);
    spend_pubkey.copy_from_slice(&data[33..66]);
    Ok(MwebAddress { scan_pubkey, spend_pubkey })
}

/// Returns true if `address` is a mainnet or testnet MWEB stealth address.
/// True if address starts with "ltcmweb1" (mainnet) or "tmweb1" (testnet).
pub fn is_mweb_address(address: &str) -> bool {
    address.starts_with("ltcmweb1") || address.starts_with("tmweb1")
}

// ── Derivation ────────────────────────────────────────────────────────────

use secp256k1::{PublicKey, Secp256k1, SecretKey};
use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::derivation::chains::bitcoin::{base58check_encode, derive_secp_keypair, hash160};
use crate::SpectraBridgeError;

const LTC_MAINNET_VERSION: u8 = 0x30;
const LTC_TESTNET_VERSION: u8 = 0x6f;

// Build an LTC P2PKH address: base58check(version || hash160(pubkey)).
fn p2pkh_address(version: u8, pubkey: &PublicKey) -> String {
    let mut payload = vec![version];
    payload.extend_from_slice(&hash160(&pubkey.serialize()));
    base58check_encode(&payload)
}

// Shared body for derive_litecoin / derive_litecoin_testnet; rejects non-P2PKH script types.
fn ltc_internal(
    version: u8, seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    if !matches!(script_type, BitcoinScriptType::P2pkh) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "This chain only supports P2PKH addresses.".into(),
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

/// UniFFI export: derive Litecoin mainnet keys (P2PKH only).
#[uniffi::export]
pub fn derive_litecoin(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    ltc_internal(LTC_MAINNET_VERSION, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Litecoin testnet keys.
#[uniffi::export]
pub fn derive_litecoin_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    ltc_internal(LTC_TESTNET_VERSION, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Litecoin address/pubkey directly from a hex private key.
#[uniffi::export]
pub fn derive_litecoin_from_private_key(
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
        address: want_address.then(|| p2pkh_address(LTC_MAINNET_VERSION, &pk)),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: None,
        account: 0, branch: 0, index: 0,
    })
}
