//! Bitcoin SV: address validation, BIP-39 + BIP-32 derivation, legacy P2PKH
//! base58check encoding.


// ── Address validation (preserved) ───────────────────────────────────────

// Base58check-decode a BSV address and return the 20-byte pubkey hash.
pub(crate) fn decode_bsv_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid bsv address: {e}"))?;
    if decoded.len() != 21 {
        return Err("bsv address wrong length".to_string());
    }
    let version = decoded[0];
    if version != 0x00 && version != 0x05 && version != 0x6f && version != 0xc4 {
        return Err(format!("unexpected bsv version byte: 0x{version:02x}"));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

/// True if the address is valid BSV base58check with a recognised version byte.
pub fn validate_bsv_address(address: &str) -> bool {
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map(|b| b.len() == 21 && (b[0] == 0x00 || b[0] == 0x05 || b[0] == 0x6f || b[0] == 0xc4))
        .unwrap_or(false)
}

// ── Derivation ────────────────────────────────────────────────────────────

use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::derivation::chains::bitcoin::{base58check_encode, derive_secp_keypair, hash160};
use crate::SpectraBridgeError;

const BSV_MAINNET_VERSION: u8 = 0x00;
const BSV_TESTNET_VERSION: u8 = 0x6f;

// Build a BSV P2PKH address: base58check(version || hash160(pubkey)).
fn p2pkh_address(version: u8, pubkey: &secp256k1::PublicKey) -> String {
    let mut payload = vec![version];
    payload.extend_from_slice(&hash160(&pubkey.serialize()));
    base58check_encode(&payload)
}

// Shared body for derive_bitcoin_sv / derive_bitcoin_sv_testnet; rejects non-P2PKH script types.
fn bsv_internal(
    version: u8, seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    if !matches!(script_type, BitcoinScriptType::P2pkh) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "Bitcoin SV only supports P2PKH addresses.".into(),
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

/// UniFFI export: derive Bitcoin SV mainnet keys (P2PKH only).
#[uniffi::export]
pub fn derive_bitcoin_sv(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bsv_internal(BSV_MAINNET_VERSION, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Bitcoin SV testnet keys (P2PKH only).
#[uniffi::export]
pub fn derive_bitcoin_sv_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bsv_internal(BSV_TESTNET_VERSION, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

