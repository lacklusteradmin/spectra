use bip39::{Language, Mnemonic};
use blake2::digest::consts::U64;
use blake2::digest::Digest;
use blake2::Blake2b;
use pbkdf2::pbkdf2_hmac;
use hmac::{Hmac, Mac};
use secp256k1::{All, PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

pub(crate) const HARDENED_OFFSET: u32 = 0x80000000;
type Blake2b512 = Blake2b<U64>;
type HmacSha512 = Hmac<Sha512>;

pub(crate) type OptionalKeyMaterial = (Option<String>, Option<String>, Option<String>);

/// Map locale string ("en", "zh-cn", etc.) to BIP-39 wordlist; defaults to English.
pub(crate) fn resolve_bip39_language(name: Option<&str>) -> Result<Language, String> {
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
        "traditional-chinese"
        | "chinese-traditional"
        | "traditional_chinese"
        | "zh-hant"
        | "zh-tw" => Ok(Language::TraditionalChinese),
        other => Err(format!("Unsupported mnemonic wordlist: {other}")),
    }
}

/// BIP-39 mnemonic -> 64-byte seed via NFKD normalization and PBKDF2-HMAC-SHA512.
pub(crate) fn derive_bip39_seed(
    seed_phrase: &str,
    passphrase: &str,
    iteration_count: u32,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
) -> Result<Zeroizing<[u8; 64]>, String> {
    let language = resolve_bip39_language(mnemonic_wordlist)?;
    let mnemonic =
        Mnemonic::parse_in_normalized(language, seed_phrase).map_err(|e| e.to_string())?;
    let iterations = if iteration_count == 0 {
        2048
    } else {
        iteration_count
    };
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

/// Parse a BIP-32 derivation path string ("m/44'/0'/0'/0/0") into child indices.
pub(crate) fn parse_bip32_path(path: &str) -> Result<Vec<u32>, String> {
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

pub(crate) fn derive_substrate_mini_secret(
    mnemonic: &str,
    passphrase: &str,
    wordlist: Option<&str>,
    salt_prefix: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 32]>, String> {
    let language = resolve_bip39_language(wordlist)?;
    let parsed = Mnemonic::parse_in_normalized(language, mnemonic).map_err(|e| e.to_string())?;
    let entropy = Zeroizing::new(parsed.to_entropy());
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let iterations = if iteration_count == 0 {
        2048
    } else {
        iteration_count
    };
    let mut buf = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(&entropy, salt.as_bytes(), iterations, &mut *buf);
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&buf[..32]);
    Ok(out)
}

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

fn ss58_prefix_bytes(network_prefix: u16) -> Vec<u8> {
    if network_prefix < 64 {
        vec![network_prefix as u8]
    } else {
        let lower = (network_prefix & 0b0000_0000_1111_1111) as u8;
        let upper = ((network_prefix & 0b0011_1111_0000_0000) >> 8) as u8;
        let first = ((lower & 0b1111_1100) >> 2) | ((upper & 0b0000_0011) << 6);
        let second = (lower & 0b0000_0011) | (upper & 0b1111_1100) | 0b0100_0000;
        vec![first | 0b0100_0000, second]
    }
}

fn ss58_prefix_from_bytes(decoded: &[u8]) -> Result<(u16, usize), String> {
    let Some(first) = decoded.first().copied() else {
        return Err("ss58 empty payload".to_string());
    };
    if first < 64 {
        return Ok((first as u16, 1));
    }
    let second = *decoded
        .get(1)
        .ok_or_else(|| "ss58 missing second prefix byte".to_string())?;
    let lower = ((first & 0b0011_1111) << 2) | (second & 0b0000_0011);
    let upper = (second & 0b0011_1100) >> 2;
    Ok((((upper as u16) << 8) | lower as u16, 2))
}

fn ss58_checksum(payload_without_checksum: &[u8]) -> [u8; 2] {
    let mut hasher = Blake2b512::new();
    hasher.update(b"SS58PRE");
    hasher.update(payload_without_checksum);
    let checksum = hasher.finalize();
    [checksum[0], checksum[1]]
}

pub(crate) fn encode_ss58(public_key: &[u8; 32], network_prefix: u16) -> String {
    let prefix_bytes = ss58_prefix_bytes(network_prefix);
    let mut payload = Vec::with_capacity(prefix_bytes.len() + 32 + 2);
    payload.extend_from_slice(&prefix_bytes);
    payload.extend_from_slice(public_key);

    let checksum = ss58_checksum(&payload);
    payload.extend_from_slice(&checksum);

    bs58::encode(payload).into_string()
}

pub(crate) fn decode_ss58(
    address: &str,
    expected_prefix: Option<u16>,
) -> Result<(u16, [u8; 32]), String> {
    let decoded = bs58::decode(address)
        .into_vec()
        .map_err(|e| format!("ss58 decode: {e}"))?;
    let (prefix, key_start) = ss58_prefix_from_bytes(&decoded)?;
    if let Some(expected) = expected_prefix {
        if prefix != expected {
            return Err(format!("ss58 prefix: {prefix}"));
        }
    }
    let checksum_start = key_start + 32;
    if decoded.len() != checksum_start + 2 {
        return Err(format!("ss58 payload length: {}", decoded.len()));
    }
    let checksum = ss58_checksum(&decoded[..checksum_start]);
    if decoded[checksum_start] != checksum[0] || decoded[checksum_start + 1] != checksum[1] {
        return Err("ss58 checksum mismatch".to_string());
    }
    let key_bytes: [u8; 32] = decoded[key_start..key_start + 32]
        .try_into()
        .map_err(|_| "ss58 key slice error".to_string())?;
    Ok((prefix, key_bytes))
}

/// BIP-32 secp256k1 extended private key: the 32-byte scalar and its chain code.
///
/// One copy, not one per chain. Decred, EVM, Kaspa, Tron and XRP each carried a
/// byte-identical 60-line version of this; the derivation is BIP-32's, not any
/// of theirs, and five copies is five places for a fix to miss four. Bitcoin
/// keeps its own in `chains/bitcoin.rs` because it also serialises xpubs.
#[derive(Clone)]
pub(crate) struct ExtendedPrivateKey {
    pub(crate) private_key: SecretKey,
    pub(crate) chain_code: [u8; 32],
}

impl ExtendedPrivateKey {
    /// BIP-32 master key: HMAC-SHA512(hmac_key, seed) → private key (IL) + chain code (IR).
    pub(crate) fn master_from_seed(hmac_key: &[u8], seed: &[u8]) -> Result<Self, String> {
        let mut mac =
            HmacSha512::new_from_slice(hmac_key).map_err(|e| format!("HMAC init: {e}"))?;
        mac.update(seed);
        let tag = mac.finalize().into_bytes();
        let private_key =
            SecretKey::from_slice(&tag[..32]).map_err(|e| format!("Master key invalid: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);
        Ok(Self {
            private_key,
            chain_code,
        })
    }

    /// Hardened indices feed the private key into the HMAC, non-hardened the public key.
    pub(crate) fn derive_child(&self, secp: &Secp256k1<All>, index: u32) -> Result<Self, String> {
        let mut mac =
            HmacSha512::new_from_slice(&self.chain_code).map_err(|e| format!("HMAC init: {e}"))?;
        if index >= HARDENED_OFFSET {
            mac.update(&[0x00]);
            mac.update(&self.private_key.secret_bytes());
        } else {
            let pk = PublicKey::from_secret_key(secp, &self.private_key);
            mac.update(&pk.serialize());
        }
        mac.update(&index.to_be_bytes());
        let tag = mac.finalize().into_bytes();
        let tweak =
            Scalar::from_be_bytes(tag[..32].try_into().map_err(|_| "tag slice".to_string())?)
                .map_err(|_| "BIP-32 IL out of range".to_string())?;
        let private_key = self
            .private_key
            .add_tweak(&tweak)
            .map_err(|e| format!("BIP-32 tweak failed: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);
        Ok(Self {
            private_key,
            chain_code,
        })
    }

    pub(crate) fn derive_path(&self, secp: &Secp256k1<All>, path: &[u32]) -> Result<Self, String> {
        let mut key = self.clone();
        for &index in path {
            key = key.derive_child(secp, index)?;
        }
        Ok(key)
    }
}

/// HMAC-SHA512 over concatenated chunks; returns a 64-byte `Zeroizing` buffer.
///
/// Aptos, Cardano, Internet Computer, Solana, Stellar, Sui and TON each had a
/// byte-identical copy of this and the two functions below.
pub(crate) fn hmac_sha512(key: &[u8], chunks: &[&[u8]]) -> Result<Zeroizing<[u8; 64]>, String> {
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

/// Parse a SLIP-10 derivation path and force every segment to hardened.
///
/// Separate from [`parse_bip32_path`] on purpose: SLIP-10 over ed25519 has no
/// non-hardened derivation, so a path that omits the `'` still means hardened.
pub(crate) fn parse_slip10_ed25519_path(path: &str) -> Result<Vec<u32>, String> {
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

/// Walk SLIP-10 hardened child derivation from seed to a 32-byte ed25519 key.
pub(crate) fn derive_slip10_ed25519_key(
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
