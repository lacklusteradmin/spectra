//! Dogecoin: address validation, BIP-32 derivation, P2PKH (D…) base58check
//! encoding. Self-contained — see `REFACTOR_NOTES.md`.


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

// Build the DOGE P2PKH locking script for the given 20-byte pubkey hash.
pub(crate) fn p2pkh_script(pubkey_hash: &[u8; 20]) -> Result<Vec<u8>, String> {
    Ok(vec![
        0x76, 0xa9, 0x14, pubkey_hash[0], pubkey_hash[1], pubkey_hash[2], pubkey_hash[3],
        pubkey_hash[4], pubkey_hash[5], pubkey_hash[6], pubkey_hash[7], pubkey_hash[8],
        pubkey_hash[9], pubkey_hash[10], pubkey_hash[11], pubkey_hash[12], pubkey_hash[13],
        pubkey_hash[14], pubkey_hash[15], pubkey_hash[16], pubkey_hash[17], pubkey_hash[18],
        pubkey_hash[19], 0x88, 0xac,
    ])
}

// ── Derivation ────────────────────────────────────────────────────────────

use secp256k1::{PublicKey, Secp256k1, SecretKey};
use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::derivation::chains::bitcoin::{base58check_encode, derive_secp_keypair, hash160};
use crate::SpectraBridgeError;

const DOGE_MAINNET_VERSION: u8 = 0x1e;
const DOGE_TESTNET_VERSION: u8 = 0x71;

// Build a DOGE P2PKH address: base58check(version || hash160(pubkey)).
fn doge_p2pkh_address(version: u8, pubkey: &PublicKey) -> String {
    let mut payload = vec![version];
    payload.extend_from_slice(&hash160(&pubkey.serialize()));
    base58check_encode(&payload)
}

// Shared body for derive_dogecoin / derive_dogecoin_testnet; rejects non-P2PKH script types.
fn doge_internal(
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
        address: want_address.then(|| doge_p2pkh_address(version, &pk)),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(priv_bytes)),
        account, branch, index,
    })
}

/// UniFFI export: derive Dogecoin mainnet keys (P2PKH only).
#[uniffi::export]
pub fn derive_dogecoin(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    doge_internal(DOGE_MAINNET_VERSION, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Dogecoin testnet keys.
#[uniffi::export]
pub fn derive_dogecoin_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    doge_internal(DOGE_TESTNET_VERSION, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Dogecoin address/pubkey directly from a hex private key.
#[uniffi::export]
pub fn derive_dogecoin_from_private_key(
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
        address: want_address.then(|| doge_p2pkh_address(DOGE_MAINNET_VERSION, &pk)),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: None,
        account: 0, branch: 0, index: 0,
    })
}
