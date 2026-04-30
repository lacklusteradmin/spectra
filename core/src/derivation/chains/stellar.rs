//! Stellar: address validation, BIP-39 + SLIP-10 ed25519 derivation, strkey
//! (G-account) encoding. Self-contained — see `REFACTOR_NOTES.md`.
//!
//! Strkey layout: `[version=0x30] || pubkey(32) || crc16_xmodem(le)(2)`,
//! then RFC 4648 base32 (no padding). Version byte 0x30 = `6 << 3` selects
//! the G-account address family.

use bip39::{Language, Mnemonic};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::derivation::engine::{
    DerivedOutput, ParsedRequest, OUTPUT_ADDRESS, OUTPUT_PRIVATE_KEY, OUTPUT_PUBLIC_KEY,
};

// ── Address validation (preserved) ───────────────────────────────────────

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
    let mut key = [0u8; 32];
    key.copy_from_slice(&decoded[1..33]);
    Ok(key)
}

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

pub fn validate_stellar_address(address: &str) -> bool {
    decode_stellar_address(address).is_ok()
}

// ── BIP-39 ───────────────────────────────────────────────────────────────

type HmacSha512 = Hmac<Sha512>;

fn resolve_bip39_language(name: Option<&str>) -> Result<Language, String> {
    let value = match name {
        Some(value) if !value.trim().is_empty() => value.trim().to_ascii_lowercase(),
        _ => return Ok(Language::English),
    };
    match value.as_str() {
        "english" | "en" => Ok(Language::English),
        "czech" | "cs" => Ok(Language::Czech),
        "french" | "fr" => Ok(Language::French),
        "italian" | "it" => Ok(Language::Italian),
        "japanese" | "ja" | "jp" => Ok(Language::Japanese),
        "korean" | "ko" | "kr" => Ok(Language::Korean),
        "portuguese" | "pt" => Ok(Language::Portuguese),
        "spanish" | "es" => Ok(Language::Spanish),
        "simplified-chinese" | "chinese-simplified" | "simplified_chinese" | "zh-hans"
        | "zh-cn" | "zh" => Ok(Language::SimplifiedChinese),
        "traditional-chinese" | "chinese-traditional" | "traditional_chinese" | "zh-hant"
        | "zh-tw" => Ok(Language::TraditionalChinese),
        other => Err(format!("Unsupported mnemonic wordlist: {other}")),
    }
}

fn derive_bip39_seed(
    seed_phrase: &str,
    passphrase: &str,
    iteration_count: u32,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
) -> Result<Zeroizing<[u8; 64]>, String> {
    let language = resolve_bip39_language(mnemonic_wordlist)?;
    let mnemonic =
        Mnemonic::parse_in_normalized(language, seed_phrase).map_err(|e| e.to_string())?;
    let iterations = if iteration_count == 0 { 2048 } else { iteration_count };
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_mnemonic = Zeroizing::new(mnemonic.to_string().nfkd().collect::<String>());
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let mut seed = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(
        normalized_mnemonic.as_bytes(),
        salt.as_bytes(),
        iterations,
        &mut *seed,
    );
    Ok(seed)
}

// ── SLIP-10 ed25519 ──────────────────────────────────────────────────────

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

fn crc16_xmodem(bytes: &[u8]) -> u16 {
    const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);
    CRC.checksum(bytes)
}

fn base32_no_pad(input: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(input)
}

fn requests_output(requested_outputs: u32, output: u32) -> bool {
    requested_outputs & output != 0
}

/// Derive a Stellar G-address from BIP-39 + SLIP-10 ed25519. Address is
/// the strkey-encoded ed25519 public key (version byte 0x30 + pubkey +
/// little-endian CRC-16/XMODEM, base32 no-pad).
pub(crate) fn derive(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let derivation_path = request
        .derivation_path
        .clone()
        .ok_or("Derivation path is required.")?;
    let seed = derive_bip39_seed(
        &request.seed_phrase,
        &request.passphrase,
        request.iteration_count,
        request.mnemonic_wordlist.as_deref(),
        request.salt_prefix.as_deref(),
    )?;
    let private_key =
        derive_slip10_ed25519_key(seed.as_ref(), &derivation_path, request.hmac_key.as_deref())?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
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

    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(*private_key))
        } else {
            None
        },
    })
}
