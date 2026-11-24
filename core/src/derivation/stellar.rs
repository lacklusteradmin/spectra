//! Stellar: address validation, BIP-39 + SLIP-10 ed25519 derivation, strkey
//! (G-account) encoding
//!
//! Strkey layout: `[version=0x30] || pubkey(32) || crc16_xmodem(le)(2)`,
//! then RFC 4648 base32 (no padding). Version byte 0x30 = `6 << 3` selects
//! the G-account address family.

use crate::derivation::error::DerivationError;

use crate::derivation::primitives::{derive_bip39_seed, derive_slip10_ed25519_key};
use crate::derivation::types::{DerivationResult, Wants};
use ed25519_dalek::SigningKey;

// ── Address validation (preserved) ───────────────────────────────────────

// Decode a Stellar G-account strkey and return the inner 32-byte ed25519 public key.
pub(crate) fn decode_stellar_address(address: &str) -> Result<[u8; 32], DerivationError> {
    if address.len() != 56 || address != address.to_ascii_uppercase() {
        return Err(DerivationError::Invalid(
            "stellar address is not a canonical G-account StrKey".into(),
        ));
    }
    let decoded = base32_decode_rfc4648(address.trim()).ok_or_else(|| {
        DerivationError::Invalid(format!("stellar base32 decode failed: {address}").into())
    })?;
    if decoded.len() != 35 {
        return Err(DerivationError::Invalid(
            format!("stellar address wrong length: {}", decoded.len()).into(),
        ));
    }
    let version = decoded[0];
    if version != 0x30 {
        return Err(DerivationError::Invalid(
            format!("stellar address wrong version: {version:#x}").into(),
        ));
    }
    let expected_checksum = crc16_xmodem(&decoded[..33]);
    let observed_checksum = u16::from_le_bytes(
        decoded[33..35]
            .try_into()
            .map_err(|_| DerivationError::Invalid("stellar checksum slice error".into()))?,
    );
    if observed_checksum != expected_checksum {
        return Err(DerivationError::Invalid(
            "stellar address checksum mismatch".into(),
        ));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&decoded[1..33]);
    Ok(key)
}

// Decode a no-padding RFC 4648 base32 string into bytes; returns None on invalid characters.
fn base32_decode_rfc4648(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bits: u32 = 0;
    let mut bit_count: u8 = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let val = ALPHABET.iter().position(|&b| b == c)? as u32;
        bits = (bits << 5) | val;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    Some(out)
}

// ── strkey (CRC-16/XMODEM + base32) ──────────────────────────────────────

fn crc16_xmodem(bytes: &[u8]) -> u16 {
    const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);
    CRC.checksum(bytes)
}

fn base32_no_pad(input: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(input)
}

pub(crate) fn address_from_public_key(public_key: &[u8; 32]) -> String {
    let mut payload = [0u8; 35];
    payload[0] = 0x30;
    payload[1..33].copy_from_slice(public_key);
    let checksum = crc16_xmodem(&payload[..33]);
    payload[33..].copy_from_slice(&checksum.to_le_bytes());
    base32_no_pad(&payload)
}

/// Derive a Stellar wallet (G-account strkey address) from a seed phrase via
/// BIP-39 + SLIP-10 ed25519. Every Stellar network has the same addresses.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, hmac_key)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    Ok(DerivationResult {
        address: wants.address.then(|| address_from_public_key(&public_key)),
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&*private_key),
    })
}
