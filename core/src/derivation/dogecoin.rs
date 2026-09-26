//! Dogecoin: address validation, BIP-32 derivation, P2PKH (D…) base58check
//! encoding

// ── Address validation (preserved from prior file) ───────────────────────

// Base58check-decode a DOGE address and return the 20-byte pubkey hash.
pub(crate) fn decode_doge_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid doge address: {e}"))?;
    if decoded.len() < 21 {
        return Err("address too short".to_string());
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::{derive_legacy_p2pkh, encode_p2pkh};
use crate::derivation::types::{BitcoinScriptType, DerivationResult};
use secp256k1::{PublicKey, Secp256k1, SecretKey};

pub(crate) const DOGE_MAINNET_VERSION: u8 = 0x1e;
pub(crate) const DOGE_TESTNET_VERSION: u8 = 0x71;

/// Derive Dogecoin mainnet keys (P2PKH only).
pub fn derive_dogecoin(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        DOGE_MAINNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

pub fn derive_dogecoin_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        DOGE_TESTNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive Dogecoin address/pubkey directly from a hex private key.
pub fn derive_dogecoin_from_private_key(
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
        address: want_address.then(|| encode_p2pkh(DOGE_MAINNET_VERSION, &pk.serialize())),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: None,
        account: 0,
        branch: 0,
        index: 0,
    })
}
