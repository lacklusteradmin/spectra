//! Polkadot: SS58 address validation, BIP-39 → substrate-bip39 PBKDF2
//! mini-secret → sr25519 derivation, SS58 v1 address encoding.
//! Self-contained — see `REFACTOR_NOTES.md`.
//!
//! - SS58 prefix: 0 = `Chain::Polkadot` (mainnet, addresses start with `1…`),
//!   42 = `Chain::PolkadotWestend` (testnet, addresses start with `5…`).
//! - Substrate junction derivation (`//hard`, `/soft`) is not yet supported —
//!   omit the derivation path to derive the root sr25519 keypair.

use bip39::{Language, Mnemonic};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;


// ── SS58 decoding (preserved) ────────────────────────────────────────────

// Decode a Polkadot/Substrate SS58 address and return the inner 32-byte public key.
pub(crate) fn decode_ss58(address: &str) -> Result<[u8; 32], String> {
    let decoded = bs58::decode(address)
        .into_vec()
        .map_err(|e| format!("ss58 decode: {e}"))?;
    // SS58: [prefix(1-2 bytes)] + [key(32)] + [checksum(2)]
    if decoded.len() < 34 {
        return Err(format!("ss58 too short: {}", decoded.len()));
    }
    let key_start = if decoded[0] < 64 { 1 } else { 2 };
    let key_bytes: [u8; 32] = decoded[key_start..key_start + 32]
        .try_into()
        .map_err(|_| "ss58 key slice error".to_string())?;
    Ok(key_bytes)
}

// ── BIP-39 ───────────────────────────────────────────────────────────────

// Map locale string ("en", "zh-cn", etc.) to BIP-39 wordlist; defaults to English.
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

// ── substrate-bip39 mini-secret ──────────────────────────────────────────

// substrate-bip39: PBKDF2(password=entropy, salt="mnemonic"||passphrase)[..32] → mini-secret key.
fn derive_substrate_mini_secret(
    mnemonic: &str,
    passphrase: &str,
    wordlist: Option<&str>,
    salt_prefix: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 32]>, String> {
    // substrate-bip39: mini-secret = PBKDF2-HMAC-SHA512(
    //   password = BIP-39 entropy bytes (NOT the mnemonic string),
    //   salt     = "mnemonic" || passphrase,
    //   iter     = 2048,
    //   dklen    = 64
    // )[0..32].
    let language = resolve_bip39_language(wordlist)?;
    let parsed =
        Mnemonic::parse_in_normalized(language, mnemonic).map_err(|e| e.to_string())?;
    let entropy = Zeroizing::new(parsed.to_entropy());
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let iterations = if iteration_count == 0 { 2048 } else { iteration_count };
    let mut buf = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(&entropy, salt.as_bytes(), iterations, &mut *buf);
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&buf[..32]);
    Ok(out)
}

// Derive sr25519 mini-secret and public key from mnemonic; uniform_expansion selects the schnorrkel expansion mode.
pub(crate) fn derive_substrate_sr25519_material(
    seed_phrase: &str,
    passphrase: &str,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
    iteration_count: u32,
    derivation_path: Option<&str>,
    uniform_expansion: bool,
) -> Result<([u8; 32], [u8; 32]), String> {
    let path = derivation_path.unwrap_or("").trim();
    if !path.is_empty() && path != "m" && path != "M" {
        return Err(
            "Substrate junction derivation (//hard, /soft) is not yet supported; \
             omit the derivation path to derive the root sr25519 keypair."
                .to_string(),
        );
    }

    let mini_secret = derive_substrate_mini_secret(
        seed_phrase,
        passphrase,
        mnemonic_wordlist,
        salt_prefix,
        iteration_count,
    )?;

    let mini = schnorrkel::MiniSecretKey::from_bytes(&*mini_secret)
        .map_err(|e| format!("Invalid sr25519 mini-secret: {e}"))?;
    let mode = if uniform_expansion {
        schnorrkel::ExpansionMode::Uniform
    } else {
        schnorrkel::ExpansionMode::Ed25519
    };
    let keypair = mini.expand_to_keypair(mode);
    let public_key = keypair.public.to_bytes();

    let mut mini_out = [0u8; 32];
    mini_out.copy_from_slice(&*mini_secret);
    Ok((mini_out, public_key))
}

// ── SS58 v1 encoding ─────────────────────────────────────────────────────

/// Encode a 32-byte sr25519 public key into an SS58 v1 address with the given network prefix.
pub(crate) fn encode_ss58(public_key: &[u8; 32], network_prefix: u16) -> String {
    use blake2::digest::consts::U64;
    use blake2::digest::Digest;
    use blake2::Blake2b;
    type Blake2b512 = Blake2b<U64>;

    let prefix_bytes: Vec<u8> = if network_prefix < 64 {
        vec![network_prefix as u8]
    } else {
        // 14-bit prefix packed into two bytes per the SS58 spec.
        let lower = (network_prefix & 0b0000_0000_1111_1111) as u8;
        let upper = ((network_prefix & 0b0011_1111_0000_0000) >> 8) as u8;
        let first = ((lower & 0b1111_1100) >> 2) | ((upper & 0b0000_0011) << 6);
        let second = (lower & 0b0000_0011) | (upper & 0b1111_1100) | 0b0100_0000;
        vec![first | 0b0100_0000, second]
    };

    let mut payload = Vec::with_capacity(prefix_bytes.len() + 32 + 2);
    payload.extend_from_slice(&prefix_bytes);
    payload.extend_from_slice(public_key);

    let mut hasher = Blake2b512::new();
    hasher.update(b"SS58PRE");
    hasher.update(&payload);
    let checksum = hasher.finalize();
    payload.extend_from_slice(&checksum[..2]);

    bs58::encode(payload).into_string()
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::DerivationResult;
use crate::SpectraBridgeError;

// Shared derivation logic for all Substrate-based networks; ss58_prefix selects the network.
fn substrate_internal(
    ss58_prefix: u16,
    seed_phrase: String, passphrase: Option<String>, hmac_key: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let uniform_expansion = hmac_key.as_deref() == Some("uniform");
    let (mini_secret, public_key) = derive_substrate_sr25519_material(
        &seed_phrase, passphrase.as_deref().unwrap_or(""),
        None, None, 0, None, uniform_expansion,
    )?;
    Ok(DerivationResult {
        address: want_address.then(|| encode_ss58(&public_key, ss58_prefix)),
        public_key_hex: want_public_key.then(|| hex::encode(public_key)),
        private_key_hex: want_private_key.then(|| hex::encode(mini_secret)),
        account: 0, branch: 0, index: 0,
    })
}

/// UniFFI export: derive Polkadot mainnet wallet (SS58 prefix 0, "1…" addresses).
#[uniffi::export]
pub fn derive_polkadot(
    seed_phrase: String, passphrase: Option<String>, hmac_key: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    substrate_internal(0, seed_phrase, passphrase, hmac_key, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Polkadot Westend testnet wallet (SS58 prefix 42, "5…" addresses).
#[uniffi::export]
pub fn derive_polkadot_westend(
    seed_phrase: String, passphrase: Option<String>, hmac_key: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    substrate_internal(42, seed_phrase, passphrase, hmac_key, want_address, want_public_key, want_private_key)
}

