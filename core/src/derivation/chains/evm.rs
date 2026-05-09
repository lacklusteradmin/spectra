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

// Keccak-256 hash; used for EVM address derivation and EIP-55 checksum computation.
pub(crate) fn keccak256(data: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    Keccak256::digest(data).into()
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


type HmacSha512 = Hmac<Sha512>;

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

// BIP-39 mnemonic → 64-byte seed via NFKD normalization and PBKDF2-HMAC-SHA512.
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

// Parse a BIP-32 derivation path string ("m/44'/60'/0'/0/0") into a list of child index integers.
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
    // Derive BIP-32 master key: HMAC-SHA512(hmac_key, seed) → private key (IL) + chain code (IR).
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

    // Derive a BIP-32 child key; hardened indices use private key as input, non-hardened use public key.
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

    // Walk the full BIP-32 derivation path by applying derive_child for each index.
    fn derive_path(&self, secp: &Secp256k1<All>, path: &[u32]) -> Result<Self, String> {
        let mut key = self.clone();
        for &index in path {
            key = key.derive_child(secp, index)?;
        }
        Ok(key)
    }
}

// Derive an EVM address (EIP-55 checksummed), public key, and private key from a mnemonic.
pub(crate) fn derive_from_seed_phrase(
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

    let address = if want_address {
        let uncompressed = public_key.serialize_uncompressed();
        let hash = keccak256(&uncompressed[1..]);
        Some(eip55_checksum(&hash[12..]))
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

// Derive an EVM address and public key directly from a 32-byte raw private key.
pub(crate) fn derive_from_private_key_bytes(
    key_bytes: &[u8; 32],
    want_address: bool,
    want_public_key: bool,
) -> Result<(Option<String>, Option<String>), String> {
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(key_bytes)
        .map_err(|e| format!("invalid private key: {e}"))?;
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);

    let address = if want_address {
        let uncompressed = public_key.serialize_uncompressed();
        let hash = keccak256(&uncompressed[1..]);
        Some(eip55_checksum(&hash[12..]))
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::{DerivationResult, parse_path_metadata};
use crate::SpectraBridgeError;

// Shared derivation logic for all EVM-compatible chains (same address for all since there's no chain-specific encoding).
fn evm_internal(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        &seed_phrase, &derivation_path, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

// Validate and decode a 64-character hex private key string into 32 raw bytes.
fn decode_privkey_hex(hex_str: &str) -> Result<[u8; 32], SpectraBridgeError> {
    let trimmed = hex_str.trim();
    if trimmed.len() != 64 {
        return Err(SpectraBridgeError::InvalidInput {
            message: "Private key hex must be exactly 64 characters.".into(),
        });
    }
    let bytes = hex::decode(trimmed)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// UniFFI export: derive Ethereum mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_ethereum(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Ethereum Classic wallet from a seed phrase (same address derivation as Ethereum).
#[uniffi::export]
pub fn derive_ethereum_classic(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Arbitrum mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_arbitrum(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Optimism mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_optimism(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Avalanche C-Chain mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_avalanche(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Base mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_base(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive BNB Chain mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_bnb(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Polygon mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_polygon(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Hyperliquid mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_hyperliquid(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Linea mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_linea(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Scroll mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_scroll(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Blast mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_blast(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Mantle mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_mantle(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Sei EVM mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_sei(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Celo mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_celo(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Cronos mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_cronos(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive opBNB mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_op_bnb(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive zkSync Era mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_zksync_era(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Sonic mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_sonic(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Berachain mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_berachain(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Unichain mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_unichain(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Ink mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_ink(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive X Layer mainnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_x_layer(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Ethereum Sepolia testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_ethereum_sepolia(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Ethereum Hoodi testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_ethereum_hoodi(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Ethereum Classic Mordor testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_ethereum_classic_mordor(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Arbitrum Sepolia testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_arbitrum_sepolia(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Optimism Sepolia testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_optimism_sepolia(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Base Sepolia testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_base_sepolia(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive BNB Chain testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_bnb_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Avalanche Fuji testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_avalanche_fuji(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Polygon Amoy testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_polygon_amoy(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Hyperliquid testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_hyperliquid_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    evm_internal(seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive an EVM address and public key from a raw private key hex string.
#[uniffi::export]
pub fn derive_evm_from_private_key(
    private_key_hex: String,
    want_address: bool,
    want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let key_bytes = decode_privkey_hex(&private_key_hex)?;
    let (address, public_key_hex) =
        derive_from_private_key_bytes(&key_bytes, want_address, want_public_key)?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex: None,
        account: 0,
        branch: 0,
        index: 0,
    })
}
