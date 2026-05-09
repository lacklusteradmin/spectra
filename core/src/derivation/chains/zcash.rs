//! Zcash transparent: address validation, BIP-32 derivation, t1… P2PKH
//! base58check encoding (2-byte version prefix). Self-contained — see
//! `REFACTOR_NOTES.md`.


// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) const ZCASH_T1_VERSION: [u8; 2] = [0x1C, 0xB8];
pub(crate) const ZCASH_T3_VERSION: [u8; 2] = [0x1C, 0xBD];

// Base58check-decode a Zcash transparent address; accepts t1 (P2PKH) and t3 (P2SH) forms.
pub(crate) fn decode_zcash_address(address: &str) -> Result<[u8; 20], String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid zcash address: {e}"))?;
    if decoded.len() != 22 {
        return Err("zcash address payload must be 22 bytes (2 version + 20 hash)".to_string());
    }
    let version = [decoded[0], decoded[1]];
    if version != ZCASH_T1_VERSION && version != ZCASH_T3_VERSION {
        return Err(format!("unrecognised zcash version bytes: {version:02x?}"));
    }
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[2..22]);
    Ok(hash)
}

// Build the standard P2PKH script for a Zcash transparent address.
pub(crate) fn zcash_p2pkh_script(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x76u8, 0xa9, 0x14];
    s.extend_from_slice(pubkey_hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

/// True if address passes Zcash base58check decode with a recognised t1/t3 version prefix.
pub fn validate_zcash_address(address: &str) -> bool {
    decode_zcash_address(address).is_ok()
}

// ── Derivation ────────────────────────────────────────────────────────────

use crate::derivation::types::{DerivationResult, parse_path_metadata};
use crate::derivation::chains::bitcoin::{base58check_encode, derive_secp_keypair, hash160};
use crate::SpectraBridgeError;

const ZCASH_MAINNET_VERSION: [u8; 2] = [0x1C, 0xB8];
const ZCASH_TESTNET_VERSION: [u8; 2] = [0x1D, 0x25];

// Build a Zcash transparent P2PKH address from a 2-byte version prefix and compressed pubkey.
fn zcash_p2pkh_addr(version: [u8; 2], pubkey: &secp256k1::PublicKey) -> String {
    let mut payload = vec![version[0], version[1]];
    payload.extend_from_slice(&hash160(&pubkey.serialize()));
    base58check_encode(&payload)
}

/// BIP-39 → secp256k1 keypair → Zcash mainnet transparent P2PKH address.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str, derivation_path: &str, passphrase: Option<&str>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (pk, priv_bytes) = derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    Ok((
        want_address.then(|| zcash_p2pkh_addr(ZCASH_MAINNET_VERSION, &pk)),
        want_public_key.then(|| hex::encode(pk.serialize())),
        want_private_key.then(|| hex::encode(priv_bytes)),
    ))
}

/// BIP-39 → secp256k1 keypair → Zcash testnet transparent P2PKH address.
pub(crate) fn derive_from_seed_phrase_testnet(
    seed_phrase: &str, derivation_path: &str, passphrase: Option<&str>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (pk, priv_bytes) = derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    Ok((
        want_address.then(|| zcash_p2pkh_addr(ZCASH_TESTNET_VERSION, &pk)),
        want_public_key.then(|| hex::encode(pk.serialize())),
        want_private_key.then(|| hex::encode(priv_bytes)),
    ))
}

// Shared body for derive_zcash / derive_zcash_testnet; builds transparent P2PKH address.
fn zcash_internal(
    version: [u8; 2], seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (pk, priv_bytes) = derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    Ok(DerivationResult {
        address: want_address.then(|| zcash_p2pkh_addr(version, &pk)),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(priv_bytes)),
        account, branch, index,
    })
}

/// UniFFI export: derive Zcash mainnet transparent keys.
#[uniffi::export]
pub fn derive_zcash(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    zcash_internal(ZCASH_MAINNET_VERSION, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Zcash testnet transparent keys.
#[uniffi::export]
pub fn derive_zcash_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    zcash_internal(ZCASH_TESTNET_VERSION, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_random_garbage() {
        assert!(!validate_zcash_address(""));
        assert!(!validate_zcash_address("not-a-zec-address"));
    }

    #[test]
    fn rejects_btc_p2pkh() {
        assert!(!validate_zcash_address("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"));
    }

    #[test]
    fn p2pkh_script_shape() {
        let hash = [0u8; 20];
        let s = zcash_p2pkh_script(&hash);
        assert_eq!(s.len(), 25);
        assert_eq!(s[0], 0x76);
        assert_eq!(s[1], 0xa9);
        assert_eq!(s[2], 0x14);
        assert_eq!(s[23], 0x88);
        assert_eq!(s[24], 0xac);
    }
}
