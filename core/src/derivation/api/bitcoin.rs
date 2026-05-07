#![allow(dead_code)]
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};

use crate::derivation::chains::bitcoin::{
    base58check_encode, derive_bip39_seed, encode_p2pkh, encode_p2sh_p2wpkh, encode_p2tr,
    encode_p2wpkh, hash160, parse_bip32_path, BitcoinNetworkParams, ExtendedPrivateKey,
    BTC_MAINNET, BTC_TESTNET,
};
use crate::derivation::chains::{decred, kaspa};
use crate::SpectraBridgeError;
use super::types::{BitcoinScriptType, DerivationResult, parse_path_metadata};

// ── Shared internal helpers ───────────────────────────────────────────────────

fn derive_secp_keypair(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
) -> Result<(PublicKey, [u8; 32], Secp256k1<All>), String> {
    let secp = Secp256k1::new();
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let master = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())?;
    let path = parse_bip32_path(derivation_path)?;
    let xpriv = master.derive_path(&secp, &path)?;
    let public_key = PublicKey::from_secret_key(&secp, &xpriv.private_key);
    let private_bytes = xpriv.private_key.secret_bytes();
    Ok((public_key, private_bytes, secp))
}

fn bitcoin_encode_address(
    params: BitcoinNetworkParams,
    script_type: BitcoinScriptType,
    public_key: &PublicKey,
    secp: &Secp256k1<All>,
) -> Result<String, String> {
    let compressed = public_key.serialize();
    match script_type {
        BitcoinScriptType::P2pkh => Ok(encode_p2pkh(&params, &compressed)),
        BitcoinScriptType::P2shP2wpkh => Ok(encode_p2sh_p2wpkh(&params, &compressed)),
        BitcoinScriptType::P2wpkh => encode_p2wpkh(&params, &compressed),
        BitcoinScriptType::P2tr => encode_p2tr(&params, secp, public_key),
    }
}

/// Full Bitcoin family: P2PKH / P2SH-P2WPKH / P2WPKH / P2TR.
fn bitcoin_family_internal(
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
    let (public_key, private_bytes, secp) =
        derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    let address = if want_address {
        Some(bitcoin_encode_address(params, script_type, &public_key, &secp)?)
    } else {
        None
    };
    Ok(DerivationResult {
        address,
        public_key_hex: want_public_key.then(|| hex::encode(public_key.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(private_bytes)),
        account,
        branch,
        index,
    })
}

/// P2PKH-only chains (BCH, BSV, LTC, DOGE, DASH, BTG) — 1-byte version prefix.
/// Returns `InvalidInput` if the caller requests a non-P2PKH script type.
fn simple_p2pkh_internal(
    p2pkh_version: u8,
    script_type: BitcoinScriptType,
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
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
    let (public_key, private_bytes, _secp) =
        derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    let address = if want_address {
        let mut payload = vec![p2pkh_version];
        payload.extend_from_slice(&hash160(&public_key.serialize()));
        Some(base58check_encode(&payload))
    } else {
        None
    };
    Ok(DerivationResult {
        address,
        public_key_hex: want_public_key.then(|| hex::encode(public_key.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(private_bytes)),
        account,
        branch,
        index,
    })
}

/// Zcash transparent P2PKH — 2-byte version prefix.
fn zcash_internal(
    version: [u8; 2],
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (public_key, private_bytes, _secp) =
        derive_secp_keypair(&seed_phrase, &derivation_path, passphrase.as_deref())?;
    let address = if want_address {
        let mut payload = vec![version[0], version[1]];
        payload.extend_from_slice(&hash160(&public_key.serialize()));
        Some(base58check_encode(&payload))
    } else {
        None
    };
    Ok(DerivationResult {
        address,
        public_key_hex: want_public_key.then(|| hex::encode(public_key.serialize())),
        private_key_hex: want_private_key.then(|| hex::encode(private_bytes)),
        account,
        branch,
        index,
    })
}

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

fn bitcoin_privkey_address(
    key_bytes: &[u8; 32],
    script_type: BitcoinScriptType,
    params: BitcoinNetworkParams,
) -> Result<String, SpectraBridgeError> {
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(key_bytes).map_err(|e| e.to_string())?;
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);
    Ok(bitcoin_encode_address(params, script_type, &public_key, &secp)?)
}

fn simple_privkey_address(
    key_bytes: &[u8; 32],
    p2pkh_version: u8,
) -> Result<String, SpectraBridgeError> {
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(key_bytes).map_err(|e| e.to_string())?;
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);
    let mut payload = vec![p2pkh_version];
    payload.extend_from_slice(&hash160(&public_key.serialize()));
    Ok(base58check_encode(&payload))
}

fn decred_privkey_address(key_bytes: &[u8; 32]) -> Result<String, SpectraBridgeError> {
    use crate::derivation::chains::decred::{dcr_hash160, encode_dcr_p2pkh};
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(key_bytes).map_err(|e| e.to_string())?;
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);
    let hash = dcr_hash160(&public_key.serialize());
    Ok(encode_dcr_p2pkh(&hash))
}

// ── Bitcoin ───────────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_bitcoin(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_family_internal(BTC_MAINNET, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bitcoin_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_family_internal(BTC_TESTNET, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bitcoin_testnet4(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    bitcoin_family_internal(BTC_TESTNET, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bitcoin_signet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    // Signet uses the same bech32 HRP and version bytes as testnet.
    bitcoin_family_internal(BTC_TESTNET, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

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
        Some(bitcoin_encode_address(BTC_MAINNET, script_type, &public_key, &secp)?)
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

// ── pub(crate) tuple-returning helpers used by chain_dispatch ─────────────────

pub(crate) fn derive_from_seed_phrase(
    params: BitcoinNetworkParams,
    script_type: BitcoinScriptType,
    seed_phrase: &str, derivation_path: &str, passphrase: Option<&str>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (public_key, private_bytes, secp) = derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    let address = if want_address {
        Some(bitcoin_encode_address(params, script_type, &public_key, &secp).map_err(|e| e.to_string())?)
    } else { None };
    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

pub(crate) fn derive_simple_p2pkh(
    p2pkh_version: u8,
    seed_phrase: &str, derivation_path: &str, passphrase: Option<&str>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (public_key, private_bytes, _secp) = derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    let address = if want_address {
        let mut payload = vec![p2pkh_version];
        payload.extend_from_slice(&hash160(&public_key.serialize()));
        Some(base58check_encode(&payload))
    } else { None };
    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

pub(crate) fn derive_zcash_internal(
    version: [u8; 2],
    seed_phrase: &str, derivation_path: &str, passphrase: Option<&str>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let (public_key, private_bytes, _secp) = derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    let address = if want_address {
        let mut payload = vec![version[0], version[1]];
        payload.extend_from_slice(&hash160(&public_key.serialize()));
        Some(base58check_encode(&payload))
    } else { None };
    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

// ── Version constants (pub(crate) so chain_dispatch can reference them) ───────

pub(crate) const BCH_MAINNET_VERSION: u8 = 0x00;
pub(crate) const BCH_TESTNET_VERSION: u8 = 0x6f;
pub(crate) const BSV_MAINNET_VERSION: u8 = 0x00;
pub(crate) const BSV_TESTNET_VERSION: u8 = 0x6f;
pub(crate) const LTC_MAINNET_VERSION: u8 = 0x30;
pub(crate) const LTC_TESTNET_VERSION: u8 = 0x6f;
pub(crate) const DOGE_MAINNET_VERSION: u8 = 0x1e;
pub(crate) const DOGE_TESTNET_VERSION: u8 = 0x71;
pub(crate) const DASH_MAINNET_VERSION: u8 = 0x4C;
pub(crate) const DASH_TESTNET_VERSION: u8 = 0x8C;
pub(crate) const BTG_MAINNET_VERSION: u8 = 0x26;
pub(crate) const ZCASH_MAINNET_VERSION: [u8; 2] = [0x1C, 0xB8];
pub(crate) const ZCASH_TESTNET_VERSION: [u8; 2] = [0x1D, 0x25];

// ── Bitcoin Cash ──────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_bitcoin_cash(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(BCH_MAINNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bitcoin_cash_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(BCH_TESTNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bitcoin_cash_from_private_key(
    private_key_hex: String, want_address: bool, want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let key_bytes = decode_privkey_hex(&private_key_hex)?;
    let address = if want_address { Some(simple_privkey_address(&key_bytes, BCH_MAINNET_VERSION)?) } else { None };
    let secp = Secp256k1::new();
    let pk = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&key_bytes).map_err(|e| e.to_string())?);
    Ok(DerivationResult { address, public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())), private_key_hex: None, account: 0, branch: 0, index: 0 })
}

// ── Bitcoin SV ────────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_bitcoin_sv(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(BSV_MAINNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_bitcoin_sv_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(BSV_TESTNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

// ── Litecoin ──────────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_litecoin(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(LTC_MAINNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_litecoin_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(LTC_TESTNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_litecoin_from_private_key(
    private_key_hex: String, want_address: bool, want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let key_bytes = decode_privkey_hex(&private_key_hex)?;
    let address = if want_address { Some(simple_privkey_address(&key_bytes, LTC_MAINNET_VERSION)?) } else { None };
    let secp = Secp256k1::new();
    let pk = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&key_bytes).map_err(|e| e.to_string())?);
    Ok(DerivationResult { address, public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())), private_key_hex: None, account: 0, branch: 0, index: 0 })
}

// ── Dogecoin ──────────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_dogecoin(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(DOGE_MAINNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_dogecoin_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(DOGE_TESTNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_dogecoin_from_private_key(
    private_key_hex: String, want_address: bool, want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let key_bytes = decode_privkey_hex(&private_key_hex)?;
    let address = if want_address { Some(simple_privkey_address(&key_bytes, DOGE_MAINNET_VERSION)?) } else { None };
    let secp = Secp256k1::new();
    let pk = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&key_bytes).map_err(|e| e.to_string())?);
    Ok(DerivationResult { address, public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())), private_key_hex: None, account: 0, branch: 0, index: 0 })
}

// ── Dash ──────────────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_dash(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(DASH_MAINNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_dash_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(DASH_TESTNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

// ── Bitcoin Gold ──────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn derive_bitcoin_gold(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    simple_p2pkh_internal(BTG_MAINNET_VERSION, script_type, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

// ── Zcash (transparent P2PKH — 2-byte version prefix) ────────────────────────

#[uniffi::export]
pub fn derive_zcash(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    zcash_internal(ZCASH_MAINNET_VERSION, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_zcash_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    zcash_internal(ZCASH_TESTNET_VERSION, seed_phrase, derivation_path, passphrase, want_address, want_public_key, want_private_key)
}

// ── Decred (BLAKE-256 + custom base58 encoding) ───────────────────────────────

#[uniffi::export]
pub fn derive_decred(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = decred::derive_from_seed_phrase(
        &seed_phrase, &derivation_path, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

#[uniffi::export]
pub fn derive_decred_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = decred::derive_from_seed_phrase_testnet(
        &seed_phrase, &derivation_path, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

#[uniffi::export]
pub fn derive_decred_from_private_key(
    private_key_hex: String, want_address: bool, want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let key_bytes = decode_privkey_hex(&private_key_hex)?;
    let address = if want_address { Some(decred_privkey_address(&key_bytes)?) } else { None };
    let secp = Secp256k1::new();
    let pk = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&key_bytes).map_err(|e| e.to_string())?);
    Ok(DerivationResult { address, public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())), private_key_hex: None, account: 0, branch: 0, index: 0 })
}

// ── Kaspa (Schnorr P2PK — no script_type) ────────────────────────────────────

#[uniffi::export]
pub fn derive_kaspa(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = kaspa::derive_from_seed_phrase(
        kaspa::KASPA_HRP, &seed_phrase, &derivation_path, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}

#[uniffi::export]
pub fn derive_kaspa_testnet(
    seed_phrase: String, derivation_path: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = kaspa::derive_from_seed_phrase(
        kaspa::KASPA_TESTNET_HRP, &seed_phrase, &derivation_path, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account, branch, index })
}
