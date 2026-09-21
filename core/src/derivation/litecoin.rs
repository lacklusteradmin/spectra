//! Litecoin: address validation, P2PKH (L…) base58check encoding,
//! and MWEB stealth address parsing

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
    let (hrp, data) = bech32::decode(address).map_err(|e| format!("invalid mweb address: {e}"))?;
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
    Ok(MwebAddress {
        scan_pubkey,
        spend_pubkey,
    })
}

/// Returns true if `address` is a mainnet or testnet MWEB stealth address.
/// True if address starts with "ltcmweb1" (mainnet) or "tmweb1" (testnet).
pub fn is_mweb_address(address: &str) -> bool {
    address.starts_with("ltcmweb1") || address.starts_with("tmweb1")
}

use crate::derivation::bitcoin::{derive_legacy_p2pkh, encode_p2pkh};
use crate::derivation::types::{BitcoinScriptType, DerivationResult};
use crate::SpectraBridgeError;
use secp256k1::{PublicKey, Secp256k1, SecretKey};

pub(crate) const LTC_MAINNET_VERSION: u8 = 0x30;
pub(crate) const LTC_TESTNET_VERSION: u8 = 0x6f;

/// Derive Litecoin mainnet keys (P2PKH only).
pub fn derive_litecoin(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        LTC_MAINNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

pub fn derive_litecoin_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        LTC_TESTNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive Litecoin address/pubkey directly from a hex private key.
pub fn derive_litecoin_from_private_key(
    private_key_hex: String,
    want_address: bool,
    want_public_key: bool,
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
        address: want_address.then(|| encode_p2pkh(LTC_MAINNET_VERSION, &pk.serialize())),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: None,
        account: 0,
        branch: 0,
        index: 0,
    })
}
