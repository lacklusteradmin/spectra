//! EVM address derivation (keccak256 over uncompressed pubkey) + EIP-55
//! checksum formatting + address validation.

/// Derive the Ethereum address (checksum-cased) from an uncompressed public key.
pub fn pubkey_to_eth_address(pubkey_uncompressed: &[u8]) -> Result<String, String> {
    if pubkey_uncompressed.len() != 65 || pubkey_uncompressed[0] != 0x04 {
        return Err("expected 65-byte uncompressed public key".to_string());
    }
    let hash = keccak256(&pubkey_uncompressed[1..]);
    let addr_bytes = &hash[12..]; // last 20 bytes
    Ok(eip55_checksum(addr_bytes))
}

/// EIP-55 mixed-case checksum address.
pub fn eip55_checksum(addr_bytes: &[u8]) -> String {
    let hex = hex::encode(addr_bytes);
    let hash = keccak256(hex.as_bytes());
    let mut result = String::with_capacity(42);
    result.push_str("0x");
    for (i, c) in hex.chars().enumerate() {
        if c.is_ascii_alphabetic() {
            let nibble = (hash[i / 2] >> (if i % 2 == 0 { 4 } else { 0 })) & 0x0f;
            if nibble >= 8 {
                result.push(c.to_ascii_uppercase());
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn validate_evm_address(address: &str) -> bool {
    let s = address.strip_prefix("0x").unwrap_or(address);
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

pub(crate) fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut h = Keccak::v256();
    h.update(data);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

// ── BIP-32 + BIP-39 derivation pipeline (self-contained EVM family) ──────
// Used by Ethereum, Ethereum Classic, Arbitrum, Optimism, Avalanche,
// Hyperliquid (and their testnets). All produce identical addresses for
// the same seed phrase + path because EVM addresses don't depend on the
// chain — only on the public key. The chain identity matters elsewhere
// (chain ID for signing, RPC endpoint selection) but not for derivation.

use bip39::{Language, Mnemonic};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use secp256k1::{All, PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::derivation::engine::{
    DerivedOutput, ParsedRequest, PublicKeyFormat, OUTPUT_ADDRESS, OUTPUT_PRIVATE_KEY,
    OUTPUT_PUBLIC_KEY,
};

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

fn requests_output(requested_outputs: u32, output: u32) -> bool {
    requested_outputs & output != 0
}

/// Derive an EVM keypair + address from BIP-39 + BIP-32. Address is
/// `0x` + EIP-55-cased last-20-bytes of `keccak256(uncompressed_pubkey[1..])`.
/// All EVM chains (Ethereum, EthClassic, Arbitrum, Optimism, Avalanche,
/// Hyperliquid, Sepolia, Hoodi, Mordor, Fuji, …) dispatch here — they all
/// produce the same address from the same seed/path.
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

    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        let uncompressed = public_key.serialize_uncompressed();
        let hash = keccak256(&uncompressed[1..]);
        Some(eip55_checksum(&hash[12..]))
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
