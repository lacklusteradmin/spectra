//! Kaspa address handling.
//!
//! Kaspa uses a CashAddr-variant bech32 (NOT BIP-173) with HRP `"kaspa"` for
//! mainnet. The address payload encodes:
//!   * 1-byte version: `0x00` Schnorr P2PK (32-byte x-only pubkey),
//!                     `0x01` ECDSA P2PK (33-byte compressed pubkey),
//!                     `0x08` P2SH (32-byte script hash).
//!   * Variable-length payload (32 bytes for Schnorr/P2SH, 33 for ECDSA).
//!
//! Spectra uses Schnorr P2PK addresses (the modern Kaspa default). The 32-byte
//! x-only pubkey is what gets encoded.
//!
//! Encoding:
//!   1. Convert `version || payload` to base32 (5-bit groups).
//!   2. Append CashAddr-style 8-symbol checksum keyed on the HRP.
//!   3. Base32 alphabet: `qpzry9x8gf2tvdw0s3jn54khce6mua7l`.
//!   4. Final form: `kaspa:` + base32(payload + checksum).

pub(crate) const KASPA_HRP: &str = "kaspa";
pub(crate) const KASPA_TESTNET_HRP: &str = "kaspatest";

const KASPA_VERSION_SCHNORR: u8 = 0x00;
const KASPA_VERSION_ECDSA: u8 = 0x01;
const KASPA_VERSION_P2SH: u8 = 0x08;

const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

/// 40-bit polymod generators (matches `rusty-kaspa` reference).
const POLYMOD_GENERATORS: [u64; 5] = [
    0x98_F2BC_8E61,
    0x79_B76D_99E2,
    0xF3_3E5F_B3C4,
    0xAE_2EAB_E2A8,
    0x1E_4F43_E470,
];

/// CashAddr-style polymod with Kaspa's specific 40-bit generators.
fn polymod(values: &[u8]) -> u64 {
    let mut c: u64 = 1;
    for &d in values {
        let c0 = (c >> 35) as u8;
        c = ((c & 0x07_FFFF_FFFF) << 5) ^ d as u64;
        for (i, gen) in POLYMOD_GENERATORS.iter().enumerate() {
            if (c0 >> i) & 1 == 1 {
                c ^= *gen;
            }
        }
    }
    c ^ 1
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    hrp.bytes().map(|b| b & 0x1f).collect()
}

fn checksum(hrp: &str, data: &[u8]) -> [u8; 8] {
    let mut values = hrp_expand(hrp);
    values.push(0); // separator
    values.extend_from_slice(data);
    values.extend_from_slice(&[0u8; 8]);
    let polymod = polymod(&values);
    let mut out = [0u8; 8];
    for i in 0..8 {
        out[i] = ((polymod >> (5 * (7 - i))) & 0x1f) as u8;
    }
    out
}

/// Convert a byte slice (8-bit groups) to 5-bit groups, big-endian-first.
/// CashAddr-variant requires this with `pad = true` for encoding.
fn convert_bits(data: &[u8], from: u32, to: u32, pad: bool) -> Result<Vec<u8>, String> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let max_v: u32 = (1 << to) - 1;
    let max_acc: u32 = (1 << (from + to - 1)) - 1;
    let mut out = Vec::new();
    for &v in data {
        let v = v as u32;
        if v >> from != 0 {
            return Err(format!("convert_bits: input value out of range: {v}"));
        }
        acc = ((acc << from) | v) & max_acc;
        bits += from;
        while bits >= to {
            bits -= to;
            out.push(((acc >> bits) & max_v) as u8);
        }
    }
    if pad && bits > 0 {
        out.push(((acc << (to - bits)) & max_v) as u8);
    } else if !pad && (bits >= from || (acc << (to - bits)) & max_v != 0) {
        return Err("convert_bits: invalid padding".to_string());
    }
    Ok(out)
}

/// Encode a Kaspa address from `version || payload`.
fn encode_kaspa_address(version: u8, payload: &[u8], hrp: &str) -> Result<String, String> {
    let mut data = Vec::with_capacity(1 + payload.len());
    data.push(version);
    data.extend_from_slice(payload);
    let bits5 = convert_bits(&data, 8, 5, true)?;
    let cs = checksum(hrp, &bits5);
    let mut all = bits5;
    all.extend_from_slice(&cs);
    let mut s = String::with_capacity(hrp.len() + 1 + all.len());
    s.push_str(hrp);
    s.push(':');
    for v in &all {
        s.push(CHARSET[*v as usize] as char);
    }
    Ok(s)
}

/// Encode a Schnorr-pubkey Kaspa address (`kaspa:qrXXXX…`). The pubkey is
/// the 32-byte x-only secp256k1 public key.
pub(crate) fn encode_kaspa_schnorr(pubkey_x_only: &[u8; 32]) -> String {
    encode_kaspa_address(KASPA_VERSION_SCHNORR, pubkey_x_only, KASPA_HRP)
        .expect("schnorr payload is always valid")
}

/// Decode a Kaspa address into `(version, payload, is_testnet)`.
pub(crate) fn decode_kaspa_address(address: &str) -> Result<(u8, Vec<u8>, bool), String> {
    let lower = address.trim().to_ascii_lowercase();
    let (hrp, body) = lower
        .split_once(':')
        .ok_or_else(|| "kaspa address missing HRP separator".to_string())?;
    let is_testnet = match hrp {
        KASPA_HRP => false,
        KASPA_TESTNET_HRP => true,
        other => return Err(format!("unknown kaspa hrp: {other}")),
    };
    let mut data = Vec::with_capacity(body.len());
    for ch in body.bytes() {
        let pos = CHARSET
            .iter()
            .position(|&c| c == ch)
            .ok_or_else(|| format!("invalid kaspa base32 char: {}", ch as char))?;
        data.push(pos as u8);
    }
    if data.len() < 8 {
        return Err("kaspa address too short for checksum".to_string());
    }
    let payload5 = &data[..data.len() - 8];
    let checksum_bytes = &data[data.len() - 8..];
    let mut buf = hrp_expand(hrp);
    buf.push(0);
    buf.extend_from_slice(payload5);
    buf.extend_from_slice(checksum_bytes);
    if polymod(&buf) != 0 {
        return Err("kaspa checksum mismatch".to_string());
    }
    let bytes = convert_bits(payload5, 5, 8, false)
        .map_err(|e| format!("kaspa decode 5→8: {e}"))?;
    if bytes.is_empty() {
        return Err("kaspa empty payload".to_string());
    }
    let version = bytes[0];
    let payload = bytes[1..].to_vec();
    let expected_len = match version {
        KASPA_VERSION_SCHNORR | KASPA_VERSION_P2SH => 32,
        KASPA_VERSION_ECDSA => 33,
        v => return Err(format!("unsupported kaspa address version: 0x{v:02x}")),
    };
    if payload.len() != expected_len {
        return Err(format!(
            "kaspa address payload length mismatch: version {version}, got {}, expected {expected_len}",
            payload.len()
        ));
    }
    Ok((version, payload, is_testnet))
}

pub fn validate_kaspa_address(address: &str) -> bool {
    decode_kaspa_address(address).is_ok()
}

// ── BIP-32 + BIP-39 derivation pipeline (self-contained) ─────────────────

use bip39::{Language, Mnemonic};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use secp256k1::{All, PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;


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

const HARDENED_OFFSET: u32 = 0x80000000;

fn parse_bip32_path(path: &str) -> Result<Vec<u32>, String> {
    let trimmed = path.trim().trim_start_matches('m').trim_start_matches('M');
    let trimmed = trimmed.trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for segment in trimmed.split('/') {
        let (value, hardened) = if let Some(stripped) = segment.strip_suffix('\'') {
            (stripped, true)
        } else if let Some(stripped) = segment.strip_suffix('h') {
            (stripped, true)
        } else if let Some(stripped) = segment.strip_suffix('H') {
            (stripped, true)
        } else {
            (segment, false)
        };
        let raw: u32 = value
            .parse()
            .map_err(|_| format!("invalid path segment: {segment}"))?;
        if raw >= HARDENED_OFFSET {
            return Err(format!("path segment out of range: {segment}"));
        }
        out.push(if hardened { raw | HARDENED_OFFSET } else { raw });
    }
    Ok(out)
}

#[derive(Clone)]
struct ExtendedPrivateKey {
    private_key: SecretKey,
    chain_code: [u8; 32],
}

impl ExtendedPrivateKey {
    fn master_from_seed(hmac_key: &[u8], seed: &[u8]) -> Result<Self, String> {
        let mut mac =
            HmacSha512::new_from_slice(hmac_key).map_err(|e| format!("HMAC init: {e}"))?;
        mac.update(seed);
        let tag = mac.finalize().into_bytes();
        let private_key = SecretKey::from_slice(&tag[..32])
            .map_err(|e| format!("Master key invalid: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);
        Ok(Self { private_key, chain_code })
    }

    fn derive_child(&self, secp: &Secp256k1<All>, index: u32) -> Result<Self, String> {
        let mut mac = HmacSha512::new_from_slice(&self.chain_code)
            .map_err(|e| format!("HMAC init: {e}"))?;
        if index >= HARDENED_OFFSET {
            mac.update(&[0x00]);
            mac.update(&self.private_key.secret_bytes());
        } else {
            let pk = PublicKey::from_secret_key(secp, &self.private_key);
            mac.update(&pk.serialize());
        }
        mac.update(&index.to_be_bytes());
        let tag = mac.finalize().into_bytes();
        let tweak = Scalar::from_be_bytes(
            tag[..32].try_into().map_err(|_| "tag slice".to_string())?,
        )
        .map_err(|_| "BIP-32 IL out of range".to_string())?;
        let private_key = self
            .private_key
            .add_tweak(&tweak)
            .map_err(|e| format!("BIP-32 tweak failed: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);
        Ok(Self { private_key, chain_code })
    }

    fn derive_path(&self, secp: &Secp256k1<All>, path: &[u32]) -> Result<Self, String> {
        let mut key = self.clone();
        for &index in path {
            key = key.derive_child(secp, index)?;
        }
        Ok(key)
    }
}

pub(crate) fn derive_from_seed_phrase(
    hrp: &str,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let secp = Secp256k1::new();
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let master = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())?;
    let path = parse_bip32_path(derivation_path)?;
    let xpriv = master.derive_path(&secp, &path)?;
    let public_key = PublicKey::from_secret_key(&secp, &xpriv.private_key);
    let private_bytes = xpriv.private_key.secret_bytes();

    let serialized = public_key.serialize();
    let mut x_only = [0u8; 32];
    x_only.copy_from_slice(&serialized[1..33]);

    let address = if want_address {
        Some(encode_kaspa_address(KASPA_VERSION_SCHNORR, &x_only, hrp)
            .expect("schnorr payload is always valid"))
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_bits_roundtrip() {
        let bytes = [0x12u8, 0x34, 0x56, 0x78];
        let to5 = convert_bits(&bytes, 8, 5, true).unwrap();
        let back = convert_bits(&to5, 5, 8, false).unwrap();
        assert_eq!(&back[..bytes.len()], &bytes);
    }

    #[test]
    fn schnorr_roundtrip() {
        // All-1s pubkey is a fine smoke fixture — round-trips through encode + decode.
        let pubkey = [0x11u8; 32];
        let addr = encode_kaspa_schnorr(&pubkey);
        assert!(addr.starts_with("kaspa:"));
        let (version, payload, testnet) = decode_kaspa_address(&addr).unwrap();
        assert_eq!(version, KASPA_VERSION_SCHNORR);
        assert_eq!(payload, pubkey);
        assert!(!testnet);
    }

    #[test]
    fn rejects_garbage() {
        assert!(!validate_kaspa_address(""));
        assert!(!validate_kaspa_address("kaspa:notavalidaddress"));
        assert!(!validate_kaspa_address("notkaspa:qq"));
    }

    #[test]
    fn rejects_corrupted_checksum() {
        let pubkey = [0x22u8; 32];
        let mut addr = encode_kaspa_schnorr(&pubkey);
        // Flip the last character — invalidates the checksum.
        let last_idx = addr.len() - 1;
        let last = addr.as_bytes()[last_idx];
        let replacement = if last == b'q' { b'p' } else { b'q' };
        addr.replace_range(last_idx..last_idx + 1, &(replacement as char).to_string());
        assert!(!validate_kaspa_address(&addr));
    }
}
