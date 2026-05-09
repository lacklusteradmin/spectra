//! Dash: address validation, BIP-32 derivation, P2PKH (X…) base58check
//! encoding. Self-contained — see `REFACTOR_NOTES.md`.


// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) const DASH_P2PKH_VERSION: u8 = 0x4C;
pub(crate) const DASH_P2SH_VERSION: u8 = 0x10;

// Base58check-decode a Dash address; rejects non-Dash version bytes (0x4C / 0x10).
pub(crate) fn decode_dash_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid dash address: {e}"))?;
    if decoded.len() != 21 {
        return Err("dash legacy payload must be 21 bytes".to_string());
    }
    if decoded[0] != DASH_P2PKH_VERSION && decoded[0] != DASH_P2SH_VERSION {
        return Err(format!("unrecognised dash version byte: 0x{:02x}", decoded[0]));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok(hash)
}

// Build the standard P2PKH locking script for the given 20-byte pubkey hash.
pub(crate) fn dash_p2pkh_script(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

/// True if address passes Dash base58check decode with a recognised version byte.
pub fn validate_dash_address(address: &str) -> bool {
    decode_dash_address(address).is_ok()
}

// ── Derivation ────────────────────────────────────────────────────────────

use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::derivation::chains::bitcoin::{base58check_encode, derive_secp_keypair, hash160};
use crate::SpectraBridgeError;

const DASH_MAINNET_P2PKH: u8 = 0x4C;
const DASH_TESTNET_P2PKH: u8 = 0x8C;

// Build a Dash P2PKH address: base58check(version || hash160(pubkey)).
fn p2pkh_address(version: u8, pubkey: &secp256k1::PublicKey) -> String {
    let mut payload = vec![version];
    payload.extend_from_slice(&hash160(&pubkey.serialize()));
    base58check_encode(&payload)
}

// Shared body for derive_dash / derive_dash_testnet; rejects non-P2PKH script types.
fn dash_internal(
    version: u8, seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    if !matches!(script_type, BitcoinScriptType::P2pkh) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "Dash only supports P2PKH addresses.".into(),
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

/// UniFFI export: derive Dash mainnet keys (P2PKH only).
#[uniffi::export]
pub fn derive_dash(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    dash_internal(DASH_MAINNET_P2PKH, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Dash testnet keys.
#[uniffi::export]
pub fn derive_dash_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    dash_internal(DASH_TESTNET_P2PKH, seed_phrase, derivation_path, passphrase, script_type, want_address, want_public_key, want_private_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_btc_p2pkh() {
        assert!(!validate_dash_address("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"));
    }

    #[test]
    fn p2pkh_script_shape() {
        let s = dash_p2pkh_script(&[0u8; 20]);
        assert_eq!(s.len(), 25);
        assert_eq!(s[0], 0x76);
        assert_eq!(s[24], 0xac);
    }
}
