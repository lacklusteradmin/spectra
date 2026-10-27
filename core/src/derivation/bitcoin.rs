//! Bitcoin: address validation, BIP-32 derivation, P2PKH / P2SH-P2WPKH /
//! P2WPKH / P2TR encoding, and the full BIP-39 → BIP-32 → secp256k1 →
//! address pipeline.
//!
//! Bitcoin-family chains share the address encoders here and the derivation
//! primitives in `primitives`; this module also owns extended-key serialization.

use crate::derivation::error::DerivationError;

use bech32::Hrp;
use ripemd::Ripemd160;
use secp256k1::{All, PublicKey, Scalar, Secp256k1};
use sha2::{Digest, Sha256};

// ── Hashing primitives ───────────────────────────────────────────────────

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
pub(crate) fn base58check_decode(s: &str) -> Result<Vec<u8>, DerivationError> {
    bs58::decode(s)
        .with_check(None)
        .into_vec()
        .map_err(|e| DerivationError::Invalid(format!("base58check decode: {e}").into()))
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
pub(crate) fn encode_p2pkh(version: u8, compressed_pubkey: &[u8]) -> String {
    let mut payload = Vec::with_capacity(21);
    payload.push(version);
    payload.extend_from_slice(&hash160(compressed_pubkey));
    base58check_encode(&payload)
}

// Encode a P2SH-P2WPKH (wrapped SegWit) address: hash160(redeemScript) with the P2SH version byte.
pub(crate) fn encode_p2sh_p2wpkh(
    params: &BitcoinNetworkParams,
    compressed_pubkey: &[u8],
) -> String {
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
) -> Result<String, DerivationError> {
    let program = hash160(compressed_pubkey);
    let hrp = Hrp::parse(params.bech32_hrp)
        .map_err(|e| DerivationError::Invalid(format!("bech32 hrp: {e}").into()))?;
    bech32::segwit::encode_v0(hrp, &program)
        .map_err(|e| DerivationError::Internal(format!("bech32 encode v0: {e}")))
}

/// BIP-86 key-path-only Taproot.
pub(crate) fn encode_p2tr(
    params: &BitcoinNetworkParams,
    secp: &Secp256k1<All>,
    public_key: &PublicKey,
) -> Result<String, DerivationError> {
    let (x_only, _parity) = public_key.x_only_public_key();
    let tag_hash = sha256(b"TapTweak");
    let mut hasher = Sha256::new();
    hasher.update(tag_hash);
    hasher.update(tag_hash);
    hasher.update(x_only.serialize());
    let tweak_bytes: [u8; 32] = hasher.finalize().into();
    let tweak = Scalar::from_be_bytes(tweak_bytes)
        .map_err(|_| DerivationError::Internal("Taproot tweak out of range".into()))?;
    let (tweaked, _parity) = x_only
        .add_tweak(secp, &tweak)
        .map_err(|e| DerivationError::Internal(format!("taproot tweak: {e}")))?;
    let hrp = Hrp::parse(params.bech32_hrp)
        .map_err(|e| DerivationError::Invalid(format!("bech32 hrp: {e}").into()))?;
    bech32::segwit::encode_v1(hrp, &tweaked.serialize())
        .map_err(|e| DerivationError::Internal(format!("bech32 encode v1: {e}")))
}

// ── Derivation pipeline (shared by Bitcoin-family chains) ────────────────

use crate::derivation::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};
use crate::error::SpectraBridgeError;

/// BIP-39 → BIP-32 path walk → (compressed pubkey, raw private bytes).
/// `pub(crate)` so BCH / BSV / LTC / DOGE / DASH / BTG chain files can reuse it.
pub(crate) fn derive_secp_keypair(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
) -> Result<(PublicKey, [u8; 32]), DerivationError> {
    let secp = Secp256k1::new();
    let private_key =
        crate::derivation::bip32::phrase_key(seed_phrase, passphrase, derivation_path)?;
    let public_key = PublicKey::from_secret_key(&secp, &private_key);
    Ok((public_key, private_key.secret_bytes()))
}

// Dispatch to the correct address encoder (P2PKH / P2SH-P2WPKH / P2WPKH / P2TR) by script type.
pub(crate) fn encode_address_inner(
    params: BitcoinNetworkParams,
    script_type: BitcoinScriptType,
    public_key: &PublicKey,
) -> Result<String, DerivationError> {
    let compressed = public_key.serialize();
    match script_type {
        BitcoinScriptType::P2pkh => Ok(encode_p2pkh(params.p2pkh_version, &compressed)),
        BitcoinScriptType::P2shP2wpkh => Ok(encode_p2sh_p2wpkh(&params, &compressed)),
        BitcoinScriptType::P2wpkh => encode_p2wpkh(&params, &compressed),
        BitcoinScriptType::P2tr => {
            let secp = Secp256k1::new();
            encode_p2tr(&params, &secp, public_key)
        }
    }
}

/// Tuple-returning form of the seed derivation.
pub(crate) fn derive_from_seed_phrase(
    params: BitcoinNetworkParams,
    script_type: BitcoinScriptType,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, DerivationError> {
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

// ── Derivation entry points ────────────────────────────────────────────────────────

// Validate and decode a 64-character hex private key string into 32 raw bytes.
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
        params,
        script_type,
        &seed_phrase,
        &derivation_path,
        passphrase.as_deref(),
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

/// Derive Bitcoin mainnet wallet (P2PKH/P2SH-P2WPKH/P2WPKH/P2TR) from a seed phrase.
pub fn derive_bitcoin(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_export_internal(
        BTC_MAINNET,
        script_type,
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive Bitcoin testnet wallet from a seed phrase.
pub fn derive_bitcoin_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_export_internal(
        BTC_TESTNET,
        script_type,
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive Bitcoin testnet4 wallet from a seed phrase.
pub fn derive_bitcoin_testnet4(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_export_internal(
        BTC_TESTNET,
        script_type,
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive Bitcoin signet wallet from a seed phrase.
pub fn derive_bitcoin_signet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_export_internal(
        BTC_TESTNET,
        script_type,
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
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
pub(crate) fn parse_bitcoin_address(s: &str) -> Result<ParsedBitcoinAddress, DerivationError> {
    // SegWit (bech32 / bech32m) — HRP is lowercase "bc" or "tb".
    if let Ok((hrp, _witver, _program)) = bech32::segwit::decode(s) {
        let hrp_str = hrp.to_string().to_lowercase();
        let network = match hrp_str.as_str() {
            "bc" => BitcoinNetworkKind::Mainnet,
            "tb" | "bcrt" => BitcoinNetworkKind::Testnet,
            other => {
                return Err(DerivationError::Invalid(
                    format!("unknown bech32 HRP: {other}").into(),
                ));
            }
        };
        return Ok(ParsedBitcoinAddress::SegWit { network });
    }
    // Legacy base58check: 0x00/0x05 mainnet, 0x6f/0xc4 testnet.
    let payload = base58check_decode(s)?;
    if payload.len() != 21 {
        return Err(DerivationError::Invalid(
            format!("legacy payload must be 21 bytes, got {}", payload.len()).into(),
        ));
    }
    let network = match payload[0] {
        0x00 | 0x05 => BitcoinNetworkKind::Mainnet,
        0x6f | 0xc4 => BitcoinNetworkKind::Testnet,
        other => {
            return Err(DerivationError::Invalid(
                format!("unknown legacy version byte: 0x{other:02x}").into(),
            ));
        }
    };
    Ok(ParsedBitcoinAddress::Legacy { network })
}

/// Derive a P2PKH-only Bitcoin-family wallet for the supplied network version.
pub(crate) fn derive_legacy_p2pkh(
    version: u8,
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    if !matches!(script_type, BitcoinScriptType::P2pkh) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "This chain only supports P2PKH addresses.".into(),
        });
    }
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (pk, priv_bytes) =
        derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    Ok(DerivationResult {
        address: want_address.then(|| encode_p2pkh(version, &pk.serialize())),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(priv_bytes)),
        account,
        branch,
        index,
    })
}
