//! Bitcoin Cash: address validation, BIP-32 derivation, legacy P2PKH
//! base58check encoding. Self-contained — see `REFACTOR_NOTES.md`.

use bip39::{Language, Mnemonic};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use ripemd::Ripemd160;
use secp256k1::{All, PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::{Digest, Sha256, Sha512};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::derivation::engine::{
    DerivedOutput, ParsedRequest, PublicKeyFormat, OUTPUT_ADDRESS, OUTPUT_PRIVATE_KEY,
    OUTPUT_PUBLIC_KEY,
};
use crate::derivation::enums::Chain;

// ── Address validation (preserved) ───────────────────────────────────────

pub(crate) fn normalize_bch_address(addr: &str) -> String {
    addr.strip_prefix("bitcoincash:")
        .unwrap_or(addr)
        .to_string()
}

pub(crate) fn decode_bch_to_hash20(address: &str) -> Result<[u8; 20], String> {
    let norm = normalize_bch_address(address);
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        if decoded.len() == 21 {
            let mut hash = [0u8; 20];
            hash.copy_from_slice(&decoded[1..21]);
            return Ok(hash);
        }
    }
    Err(format!("cannot decode BCH address: {address}"))
}

pub fn validate_bch_address(address: &str) -> bool {
    let norm = normalize_bch_address(address);
    if let Ok(decoded) = bs58::decode(&norm).with_check(None).into_vec() {
        return decoded.len() == 21 && (decoded[0] == 0x00 || decoded[0] == 0x05);
    }
    norm.starts_with('q') || norm.starts_with('p')
}

// ── Hashing primitives ───────────────────────────────────────────────────

type HmacSha512 = Hmac<Sha512>;

fn hash160_bytes(bytes: &[u8]) -> [u8; 20] {
    let sha = {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let out = hasher.finalize();
        let mut result = [0u8; 32];
        result.copy_from_slice(&out);
        result
    };
    let mut hasher = Ripemd160::new();
    hasher.update(sha);
    let out = hasher.finalize();
    let mut result = [0u8; 20];
    result.copy_from_slice(&out);
    result
}

fn base58check_encode(payload: &[u8]) -> String {
    bs58::encode(payload).with_check().into_string()
}

// ── BIP-39 ───────────────────────────────────────────────────────────────

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

// ── BIP-32 ───────────────────────────────────────────────────────────────

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

// ── Public key formatting ────────────────────────────────────────────────

fn format_secp_public_key(
    public_key: &PublicKey,
    format: PublicKeyFormat,
) -> Result<Vec<u8>, String> {
    Ok(match format {
        PublicKeyFormat::Compressed => public_key.serialize().to_vec(),
        PublicKeyFormat::Uncompressed => public_key.serialize_uncompressed().to_vec(),
        PublicKeyFormat::XOnly => public_key.x_only_public_key().0.serialize().to_vec(),
        PublicKeyFormat::Raw => public_key.serialize().to_vec(),
        PublicKeyFormat::Auto => {
            return Err("Public key format must be explicit.".to_string());
        }
    })
}

// ── Top-level derivation ─────────────────────────────────────────────────

fn requests_output(requested_outputs: u32, output: u32) -> bool {
    requested_outputs & output != 0
}

/// Derive a Bitcoin Cash legacy P2PKH address. BCH uses Bitcoin's version
/// bytes for the legacy form (0x00 mainnet / 0x6f testnet); CashAddr is
/// validation-only here, the derivation flow emits legacy P2PKH.
pub(crate) fn derive(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let secp = Secp256k1::new();
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

    let key_bytes = request
        .hmac_key
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|v| v.as_bytes())
        .unwrap_or(b"Bitcoin seed");
    let master = ExtendedPrivateKey::master_from_seed(key_bytes, seed.as_ref())?;
    let path = parse_bip32_path(&derivation_path)?;
    let xpriv = master.derive_path(&secp, &path)?;
    let public_key = PublicKey::from_secret_key(&secp, &xpriv.private_key);
    let private_bytes = xpriv.private_key.secret_bytes();

    let version: u8 = if matches!(request.chain, Chain::BitcoinCashTestnet) { 0x6f } else { 0x00 };
    let mut payload = vec![version];
    payload.extend_from_slice(&hash160_bytes(&public_key.serialize()));
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(base58check_encode(&payload))
    } else {
        None
    };

    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_bytes))
        } else {
            None
        },
    })
}
