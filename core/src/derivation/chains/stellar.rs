//! Stellar: address validation, BIP-39 + SLIP-10 ed25519 derivation, strkey
//! (G-account) encoding
//!
//! Strkey layout: `[version=0x30] || pubkey(32) || crc16_xmodem(le)(2)`,
//! then RFC 4648 base32 (no padding). Version byte 0x30 = `6 << 3` selects
//! the G-account address family.

use crate::derivation::primitives::derive_bip39_seed;
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use sha2::Sha512;
use zeroize::Zeroizing;

// ── Address validation (preserved) ───────────────────────────────────────

// Decode a Stellar G-account strkey and return the inner 32-byte ed25519 public key.
pub(crate) fn decode_stellar_address(address: &str) -> Result<[u8; 32], String> {
    let decoded = base32_decode_rfc4648(address.trim())
        .ok_or_else(|| format!("stellar base32 decode failed: {address}"))?;
    if decoded.len() != 35 {
        return Err(format!("stellar address wrong length: {}", decoded.len()));
    }
    let version = decoded[0];
    if version != 0x30 {
        return Err(format!("stellar address wrong version: {version:#x}"));
    }
    let expected_checksum = crc16_xmodem(&decoded[..33]);
    let observed_checksum = u16::from_le_bytes(
        decoded[33..35]
            .try_into()
            .map_err(|_| "stellar checksum slice error".to_string())?,
    );
    if observed_checksum != expected_checksum {
        return Err("stellar address checksum mismatch".to_string());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&decoded[1..33]);
    Ok(key)
}

// Decode a no-padding RFC 4648 base32 string into bytes; returns None on invalid characters.
fn base32_decode_rfc4648(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let s = s.to_uppercase();
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

type HmacSha512 = Hmac<Sha512>;

// ── SLIP-10 ed25519 ──────────────────────────────────────────────────────

// HMAC-SHA512 over concatenated chunks; returns a 64-byte Zeroizing buffer.
fn hmac_sha512(key: &[u8], chunks: &[&[u8]]) -> Result<Zeroizing<[u8; 64]>, String> {
    let mut mac = HmacSha512::new_from_slice(key)
        .map_err(|error| format!("Invalid HMAC-SHA512 key: {error}"))?;
    for chunk in chunks {
        mac.update(chunk);
    }
    let tag = mac.finalize().into_bytes();
    let mut out = Zeroizing::new([0u8; 64]);
    out.copy_from_slice(&tag);
    Ok(out)
}

// Parse SLIP-10 ed25519 derivation path; all segments are forced hardened (0x8000_0000 bit set).
fn parse_slip10_ed25519_path(path: &str) -> Result<Vec<u32>, String> {
    let trimmed = path.trim();
    let body = trimmed
        .strip_prefix("m/")
        .or_else(|| trimmed.strip_prefix("M/"))
        .unwrap_or_else(|| {
            if trimmed == "m" || trimmed == "M" {
                ""
            } else {
                trimmed
            }
        });
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut indices = Vec::new();
    for segment in body.split('/') {
        let cleaned = segment.trim_end_matches('\'').trim_end_matches('h');
        let raw: u32 = cleaned
            .parse()
            .map_err(|_| format!("Invalid derivation path segment: {segment}"))?;
        if raw & 0x8000_0000 != 0 {
            return Err(format!("Derivation path segment out of range: {segment}"));
        }
        indices.push(raw | 0x8000_0000);
    }
    Ok(indices)
}

// Walk the SLIP-10 ed25519 derivation path from the seed to produce the final 32-byte private key.
fn derive_slip10_ed25519_key(
    seed: &[u8],
    derivation_path: &str,
    hmac_key: Option<&str>,
) -> Result<Zeroizing<[u8; 32]>, String> {
    let key_bytes = hmac_key
        .filter(|value| !value.is_empty())
        .map(|value| value.as_bytes())
        .unwrap_or(b"ed25519 seed");
    let master = hmac_sha512(key_bytes, &[seed])?;
    let mut private_key = Zeroizing::new([0u8; 32]);
    let mut chain_code = Zeroizing::new([0u8; 32]);
    private_key.copy_from_slice(&master[..32]);
    chain_code.copy_from_slice(&master[32..]);
    for index in parse_slip10_ed25519_path(derivation_path)? {
        let index_bytes = index.to_be_bytes();
        let child = hmac_sha512(
            &*chain_code,
            &[&[0x00], &*private_key as &[u8], &index_bytes],
        )?;
        private_key.copy_from_slice(&child[..32]);
        chain_code.copy_from_slice(&child[32..]);
    }
    Ok(private_key)
}

// ── strkey (CRC-16/XMODEM + base32) ──────────────────────────────────────

// CRC-16/XMODEM checksum used by the Stellar strkey format for address integrity.
fn crc16_xmodem(bytes: &[u8]) -> u16 {
    const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);
    CRC.checksum(bytes)
}

// RFC 4648 base32 encode without padding; used to produce the final strkey address string.
fn base32_no_pad(input: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(input)
}

// Derive Stellar address, public key, and private key from a mnemonic via BIP-39 + SLIP-10 ed25519.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, String> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, hmac_key)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    let address = if want_address {
        let mut payload = [0u8; 35];
        payload[0] = 0x30;
        payload[1..33].copy_from_slice(&public_key);
        let checksum = crc16_xmodem(&payload[..33]);
        payload[33] = (checksum & 0xff) as u8;
        payload[34] = (checksum >> 8) as u8;
        Some(base32_no_pad(&payload))
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(*private_key)),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::{parse_path_metadata, DerivationResult};
use crate::SpectraBridgeError;

// Shared derivation logic for all Stellar networks.
fn stellar_internal(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        &seed_phrase,
        &derivation_path,
        passphrase.as_deref(),
        hmac_key.as_deref(),
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

/// UniFFI export: derive Stellar mainnet wallet (G-account strkey address) from a seed phrase.
#[uniffi::export]
pub fn derive_stellar(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    stellar_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        hmac_key,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// UniFFI export: derive Stellar testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_stellar_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    stellar_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        hmac_key,
        want_address,
        want_public_key,
        want_private_key,
    )
}
