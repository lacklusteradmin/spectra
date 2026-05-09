//! Bitcoin: address validation, BIP-32 derivation, P2PKH / P2SH-P2WPKH /
//! P2WPKH / P2TR encoding, and the full BIP-39 → BIP-32 → secp256k1 →
//! address pipeline.
//!
//! This file is **self-contained**: BIP-32, BIP-39, HMAC, path parsing,
//! base58check, hash160, and secp material derivation all live here. Other
//! Bitcoin-family chains (Litecoin, Dogecoin, BCH, BSV, BTG, Dash, Zcash,
//! Decred, Kaspa) duplicate the same primitives in their own files.

use bech32::Hrp;
use bip39::{Language, Mnemonic};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use ripemd::Ripemd160;
use secp256k1::{All, PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::{Digest, Sha256, Sha512};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

// ── Hashing primitives ───────────────────────────────────────────────────

type HmacSha512 = Hmac<Sha512>;

// SHA-256 of the input bytes; used internally by hash160 and the Taproot tweak.
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut result = [0u8; 32];
    result.copy_from_slice(&out);
    result
}

/// RIPEMD160(SHA256(bytes)) — Bitcoin's Hash160 primitive.
pub(crate) fn hash160(bytes: &[u8]) -> [u8; 20] {
    let sha = sha256(bytes);
    let mut hasher = Ripemd160::new();
    hasher.update(sha);
    let out = hasher.finalize();
    let mut result = [0u8; 20];
    result.copy_from_slice(&out);
    result
}

// Base58Check-encode a payload (appends a 4-byte SHA-256² checksum before encoding).
pub(crate) fn base58check_encode(payload: &[u8]) -> String {
    bs58::encode(payload).with_check().into_string()
}

// Decode a Base58Check string, verify the 4-byte SHA-256² checksum, and return the payload.
pub(crate) fn base58check_decode(s: &str) -> Result<Vec<u8>, String> {
    bs58::decode(s)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("base58check decode: {e}"))
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
        "simplified-chinese"
        | "chinese-simplified"
        | "simplified_chinese"
        | "zh-hans"
        | "zh-cn"
        | "zh" => Ok(Language::SimplifiedChinese),
        "traditional-chinese"
        | "chinese-traditional"
        | "traditional_chinese"
        | "zh-hant"
        | "zh-tw" => Ok(Language::TraditionalChinese),
        other => Err(format!("Unsupported mnemonic wordlist: {other}")),
    }
}

// BIP-39 mnemonic → 64-byte seed via NFKD normalization and PBKDF2-HMAC-SHA512.
pub(crate) fn derive_bip39_seed(
    seed_phrase: &str,
    passphrase: &str,
    iteration_count: u32,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
) -> Result<Zeroizing<[u8; 64]>, String> {
    let language = resolve_bip39_language(mnemonic_wordlist)?;
    let mnemonic = Mnemonic::parse_in_normalized(language, seed_phrase)
        .map_err(|e| e.to_string())?;
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

// ── BIP-32 path parsing ──────────────────────────────────────────────────

pub(crate) const HARDENED_OFFSET: u32 = 0x80000000;

// Parse a BIP-32 derivation path string ("m/44'/0'/0'/0/0") into a list of child index integers.
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

// ── BIP-32 extended keys ─────────────────────────────────────────────────

pub(crate) const XPUB_VERSION_MAINNET: [u8; 4] = [0x04, 0x88, 0xB2, 0x1E];
pub(crate) const XPUB_VERSION_TESTNET: [u8; 4] = [0x04, 0x35, 0x87, 0xCF];

#[derive(Clone)]
pub(crate) struct ExtendedPrivateKey {
    pub depth: u8,
    pub parent_fingerprint: [u8; 4],
    pub child_number: u32,
    pub private_key: SecretKey,
    pub chain_code: [u8; 32],
}

#[derive(Clone)]
pub(crate) struct ExtendedPublicKey {
    pub depth: u8,
    pub parent_fingerprint: [u8; 4],
    pub child_number: u32,
    pub public_key: PublicKey,
    pub chain_code: [u8; 32],
}

impl ExtendedPrivateKey {
    /// Master key from BIP-39 seed. `hmac_key` is usually `b"Bitcoin seed"` but
    /// is caller-tunable so we can reuse this for SLIP-0010 if ever needed.
    pub fn master_from_seed(hmac_key: &[u8], seed: &[u8]) -> Result<Self, String> {
        let mut mac =
            HmacSha512::new_from_slice(hmac_key).map_err(|e| format!("HMAC init: {e}"))?;
        mac.update(seed);
        let tag = mac.finalize().into_bytes();
        let private_key = SecretKey::from_slice(&tag[..32])
            .map_err(|e| format!("Derived BIP-32 master key is invalid: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);
        Ok(Self {
            depth: 0,
            parent_fingerprint: [0u8; 4],
            child_number: 0,
            private_key,
            chain_code,
        })
    }

    // Compute the 4-byte fingerprint of this key's public key (first 4 bytes of hash160(pubkey)).
    pub fn fingerprint(&self, secp: &Secp256k1<All>) -> [u8; 4] {
        let pk = PublicKey::from_secret_key(secp, &self.private_key);
        let h = hash160(&pk.serialize());
        let mut fp = [0u8; 4];
        fp.copy_from_slice(&h[..4]);
        fp
    }

    // Derive a BIP-32 child key; hardened indices use private key as input, non-hardened use public key.
    pub fn derive_child(&self, secp: &Secp256k1<All>, index: u32) -> Result<Self, String> {
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
            tag[..32]
                .try_into()
                .map_err(|_| "BIP-32 tag slice".to_string())?,
        )
        .map_err(|_| "BIP-32 IL out of range — retry the derivation".to_string())?;
        let private_key = self
            .private_key
            .add_tweak(&tweak)
            .map_err(|e| format!("BIP-32 tweak failed: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);

        let parent_fingerprint = self.fingerprint(secp);
        Ok(Self {
            depth: self.depth.saturating_add(1),
            parent_fingerprint,
            child_number: index,
            private_key,
            chain_code,
        })
    }

    // Walk the full BIP-32 derivation path by applying derive_child for each index.
    pub fn derive_path(&self, secp: &Secp256k1<All>, path: &[u32]) -> Result<Self, String> {
        let mut key = self.clone();
        for &index in path {
            key = key.derive_child(secp, index)?;
        }
        Ok(key)
    }

    // Convert this extended private key to its corresponding extended public key (neutered).
    pub fn to_neutered(&self, secp: &Secp256k1<All>) -> ExtendedPublicKey {
        ExtendedPublicKey {
            depth: self.depth,
            parent_fingerprint: self.parent_fingerprint,
            child_number: self.child_number,
            public_key: PublicKey::from_secret_key(secp, &self.private_key),
            chain_code: self.chain_code,
        }
    }
}

impl ExtendedPublicKey {
    // Compute the 4-byte fingerprint of this extended public key (first 4 bytes of hash160(pubkey)).
    pub fn fingerprint(&self) -> [u8; 4] {
        let h = hash160(&self.public_key.serialize());
        let mut fp = [0u8; 4];
        fp.copy_from_slice(&h[..4]);
        fp
    }

    /// CKDpub for unhardened children. Hardened indices return an error — you
    /// can't walk a hardened level from just a public key, by design.
    pub fn derive_child(&self, secp: &Secp256k1<All>, index: u32) -> Result<Self, String> {
        if index >= HARDENED_OFFSET {
            return Err("cannot derive a hardened child from an xpub".to_string());
        }
        let mut mac = HmacSha512::new_from_slice(&self.chain_code)
            .map_err(|e| format!("HMAC init: {e}"))?;
        mac.update(&self.public_key.serialize());
        mac.update(&index.to_be_bytes());
        let tag = mac.finalize().into_bytes();

        let tweak = Scalar::from_be_bytes(
            tag[..32]
                .try_into()
                .map_err(|_| "BIP-32 tag slice".to_string())?,
        )
        .map_err(|_| "BIP-32 IL out of range — retry the derivation".to_string())?;
        let public_key = self
            .public_key
            .add_exp_tweak(secp, &tweak)
            .map_err(|e| format!("BIP-32 pubkey tweak failed: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);

        Ok(Self {
            depth: self.depth.saturating_add(1),
            parent_fingerprint: self.fingerprint(),
            child_number: index,
            public_key,
            chain_code,
        })
    }

    // Serialize this extended public key to an xpub base58check string with the given version bytes.
    pub fn to_xpub_string(&self, version: [u8; 4]) -> String {
        encode_extended_key(
            version,
            self.depth,
            self.parent_fingerprint,
            self.child_number,
            self.chain_code,
            &self.public_key.serialize(),
        )
    }

    /// Parse an xpub string and return the (key, observed version bytes). We
    /// don't auto-resolve version bytes to a network here — the caller knows
    /// which prefixes they accept.
    pub fn from_xpub_string(s: &str) -> Result<(Self, [u8; 4]), String> {
        let (version, depth, parent_fingerprint, child_number, chain_code, key_bytes) =
            decode_extended_key(s)?;
        let public_key = PublicKey::from_slice(&key_bytes)
            .map_err(|e| format!("xpub: invalid pubkey: {e}"))?;
        Ok((
            Self {
                depth,
                parent_fingerprint,
                child_number,
                public_key,
                chain_code,
            },
            version,
        ))
    }
}

// Serialize a BIP-32 extended key into a 78-byte base58check payload with version, depth, and child metadata.
fn encode_extended_key(
    version: [u8; 4],
    depth: u8,
    parent_fingerprint: [u8; 4],
    child_number: u32,
    chain_code: [u8; 32],
    key_bytes: &[u8],
) -> String {
    let mut payload = Vec::with_capacity(78);
    payload.extend_from_slice(&version);
    payload.push(depth);
    payload.extend_from_slice(&parent_fingerprint);
    payload.extend_from_slice(&child_number.to_be_bytes());
    payload.extend_from_slice(&chain_code);
    payload.extend_from_slice(key_bytes);
    base58check_encode(&payload)
}

// Decode an xpub/xprv base58check string into its component fields (version, depth, fingerprint, child number, chain code, key).
fn decode_extended_key(
    s: &str,
) -> Result<([u8; 4], u8, [u8; 4], u32, [u8; 32], [u8; 33]), String> {
    let payload = base58check_decode(s)?;
    if payload.len() != 78 {
        return Err(format!(
            "xpub/xprv payload must be 78 bytes, got {}",
            payload.len()
        ));
    }
    let mut version = [0u8; 4];
    version.copy_from_slice(&payload[0..4]);
    let depth = payload[4];
    let mut parent_fingerprint = [0u8; 4];
    parent_fingerprint.copy_from_slice(&payload[5..9]);
    let child_number = u32::from_be_bytes(payload[9..13].try_into().unwrap());
    let mut chain_code = [0u8; 32];
    chain_code.copy_from_slice(&payload[13..45]);
    let mut key_bytes = [0u8; 33];
    key_bytes.copy_from_slice(&payload[45..78]);
    Ok((
        version,
        depth,
        parent_fingerprint,
        child_number,
        chain_code,
        key_bytes,
    ))
}

// ── Address encoding ─────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub(crate) struct BitcoinNetworkParams {
    pub p2pkh_version: u8,
    pub p2sh_version: u8,
    pub bech32_hrp: &'static str,
}

pub(crate) const BTC_MAINNET: BitcoinNetworkParams = BitcoinNetworkParams {
    p2pkh_version: 0x00,
    p2sh_version: 0x05,
    bech32_hrp: "bc",
};

pub(crate) const BTC_TESTNET: BitcoinNetworkParams = BitcoinNetworkParams {
    p2pkh_version: 0x6f,
    p2sh_version: 0xc4,
    bech32_hrp: "tb",
};

// Encode a P2PKH address: version_byte || hash160(pubkey), base58check-encoded.
pub(crate) fn encode_p2pkh(params: &BitcoinNetworkParams, compressed_pubkey: &[u8]) -> String {
    let mut payload = Vec::with_capacity(21);
    payload.push(params.p2pkh_version);
    payload.extend_from_slice(&hash160(compressed_pubkey));
    base58check_encode(&payload)
}

// Encode a P2SH-P2WPKH (wrapped SegWit) address: hash160(redeemScript) with the P2SH version byte.
pub(crate) fn encode_p2sh_p2wpkh(params: &BitcoinNetworkParams, compressed_pubkey: &[u8]) -> String {
    // redeemScript = OP_0 <20-byte pubkey hash>
    let mut redeem = Vec::with_capacity(22);
    redeem.push(0x00);
    redeem.push(0x14);
    redeem.extend_from_slice(&hash160(compressed_pubkey));
    let mut payload = Vec::with_capacity(21);
    payload.push(params.p2sh_version);
    payload.extend_from_slice(&hash160(&redeem));
    base58check_encode(&payload)
}

// Encode a P2WPKH (native SegWit v0) bech32 address from a compressed public key.
pub(crate) fn encode_p2wpkh(
    params: &BitcoinNetworkParams,
    compressed_pubkey: &[u8],
) -> Result<String, String> {
    let program = hash160(compressed_pubkey);
    let hrp = Hrp::parse(params.bech32_hrp).map_err(|e| format!("bech32 hrp: {e}"))?;
    bech32::segwit::encode_v0(hrp, &program).map_err(|e| format!("bech32 encode v0: {e}"))
}

/// BIP-86 key-path-only Taproot.
pub(crate) fn encode_p2tr(
    params: &BitcoinNetworkParams,
    secp: &Secp256k1<All>,
    public_key: &PublicKey,
) -> Result<String, String> {
    let (x_only, _parity) = public_key.x_only_public_key();
    let tag_hash = sha256(b"TapTweak");
    let mut hasher = Sha256::new();
    hasher.update(tag_hash);
    hasher.update(tag_hash);
    hasher.update(x_only.serialize());
    let tweak_bytes: [u8; 32] = hasher.finalize().into();
    let tweak =
        Scalar::from_be_bytes(tweak_bytes).map_err(|_| "Taproot tweak out of range".to_string())?;
    let (tweaked, _parity) = x_only
        .add_tweak(secp, &tweak)
        .map_err(|e| format!("taproot tweak: {e}"))?;
    let hrp = Hrp::parse(params.bech32_hrp).map_err(|e| format!("bech32 hrp: {e}"))?;
    bech32::segwit::encode_v1(hrp, &tweaked.serialize())
        .map_err(|e| format!("bech32 encode v1: {e}"))
}

// ── Derivation pipeline (shared by Bitcoin-family chains) ────────────────

use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::SpectraBridgeError;

/// BIP-39 → BIP-32 path walk → (compressed pubkey, raw private bytes).
/// `pub(crate)` so BCH / BSV / LTC / DOGE / DASH / BTG chain files can reuse it.
pub(crate) fn derive_secp_keypair(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
) -> Result<(PublicKey, [u8; 32]), String> {
    let secp = Secp256k1::new();
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let master = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())?;
    let path = parse_bip32_path(derivation_path)?;
    let xpriv = master.derive_path(&secp, &path)?;
    let public_key = PublicKey::from_secret_key(&secp, &xpriv.private_key);
    Ok((public_key, xpriv.private_key.secret_bytes()))
}

// Dispatch to the correct address encoder (P2PKH / P2SH-P2WPKH / P2WPKH / P2TR) by script type.
fn encode_address_inner(
    params: BitcoinNetworkParams,
    script_type: BitcoinScriptType,
    public_key: &PublicKey,
) -> Result<String, String> {
    let compressed = public_key.serialize();
    match script_type {
        BitcoinScriptType::P2pkh => Ok(encode_p2pkh(&params, &compressed)),
        BitcoinScriptType::P2shP2wpkh => Ok(encode_p2sh_p2wpkh(&params, &compressed)),
        BitcoinScriptType::P2wpkh => encode_p2wpkh(&params, &compressed),
        BitcoinScriptType::P2tr => {
            let secp = Secp256k1::new();
            encode_p2tr(&params, &secp, public_key)
        }
    }
}

/// Tuple-returning form used by `chain_dispatch`.
pub(crate) fn derive_from_seed_phrase(
    params: BitcoinNetworkParams,
    script_type: BitcoinScriptType,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (public_key, private_bytes) =
        derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    let address = if want_address {
        Some(encode_address_inner(params, script_type, &public_key)?)
    } else {
        None
    };
    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

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

// Shared derivation logic for all Bitcoin networks; params selects mainnet/testnet version bytes.
fn bitcoin_export_internal(
    params: BitcoinNetworkParams,
    script_type: BitcoinScriptType,
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        params, script_type, &seed_phrase, &derivation_path, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

/// UniFFI export: derive Bitcoin mainnet wallet (P2PKH/P2SH-P2WPKH/P2WPKH/P2TR) from a seed phrase.
#[uniffi::export]
pub fn derive_bitcoin(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_export_internal(BTC_MAINNET, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Bitcoin testnet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_bitcoin_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_export_internal(BTC_TESTNET, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Bitcoin testnet4 wallet from a seed phrase.
#[uniffi::export]
pub fn derive_bitcoin_testnet4(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_export_internal(BTC_TESTNET, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive Bitcoin signet wallet from a seed phrase.
#[uniffi::export]
pub fn derive_bitcoin_signet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_export_internal(BTC_TESTNET, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

/// UniFFI export: derive a Bitcoin mainnet address and public key from a raw private key hex string.
#[uniffi::export]
pub fn derive_bitcoin_from_private_key(
    private_key_hex: String, script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let key_bytes = decode_privkey_hex(&private_key_hex)?;
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(&key_bytes).map_err(|e| e.to_string())?;
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);
    let address = if want_address {
        Some(encode_address_inner(BTC_MAINNET, script_type, &public_key)?)
    } else {
        None
    };
    Ok(DerivationResult {
        address,
        public_key_hex: want_public_key.then(|| hex::encode(public_key.serialize())),
        private_key_hex: None,
        account: 0, branch: 0, index: 0,
    })
}

// ── Bitcoin address parsing (structural, for the validator) ──────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BitcoinNetworkKind {
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone)]
pub(crate) enum ParsedBitcoinAddress {
    Legacy { network: BitcoinNetworkKind },
    SegWit { network: BitcoinNetworkKind },
}

// Parse a Bitcoin address as SegWit (bech32/bech32m) or legacy (base58check) and return its network kind.
pub(crate) fn parse_bitcoin_address(s: &str) -> Result<ParsedBitcoinAddress, String> {
    // SegWit (bech32 / bech32m) — HRP is lowercase "bc" or "tb".
    if let Ok((hrp, _witver, _program)) = bech32::segwit::decode(s) {
        let hrp_str = hrp.to_string().to_lowercase();
        let network = match hrp_str.as_str() {
            "bc" => BitcoinNetworkKind::Mainnet,
            "tb" | "bcrt" => BitcoinNetworkKind::Testnet,
            other => return Err(format!("unknown bech32 HRP: {other}")),
        };
        return Ok(ParsedBitcoinAddress::SegWit { network });
    }
    // Legacy base58check: 0x00/0x05 mainnet, 0x6f/0xc4 testnet.
    let payload = base58check_decode(s)?;
    if payload.len() != 21 {
        return Err(format!("legacy payload must be 21 bytes, got {}", payload.len()));
    }
    let network = match payload[0] {
        0x00 | 0x05 => BitcoinNetworkKind::Mainnet,
        0x6f | 0xc4 => BitcoinNetworkKind::Testnet,
        other => return Err(format!("unknown legacy version byte: 0x{other:02x}")),
    };
    Ok(ParsedBitcoinAddress::Legacy { network })
}
