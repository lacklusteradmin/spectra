//! Bitcoin Cash: address validation, BIP-39 + BIP-32 derivation, legacy P2PKH
//! base58check encoding.

/// Blockbook accepts CashAddr payloads without their network prefix.
pub(crate) fn normalize_bch_address(address: &str) -> String {
    address
        .strip_prefix("bitcoincash:")
        .or_else(|| address.strip_prefix("bchtest:"))
        .unwrap_or(address)
        .to_string()
}

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::{derive_legacy_p2pkh, encode_p2pkh};
use crate::derivation::types::DerivationResult;
use secp256k1::{PublicKey, Secp256k1, SecretKey};

pub(crate) const BCH_MAINNET_VERSION: u8 = 0x00;
pub(crate) const BCH_TESTNET_VERSION: u8 = 0x6f;

/// Derive Bitcoin Cash mainnet keys (P2PKH only).
pub fn derive_bitcoin_cash(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: crate::derivation::types::BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        BCH_MAINNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive Bitcoin Cash testnet keys (P2PKH only).
pub fn derive_bitcoin_cash_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: crate::derivation::types::BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        BCH_TESTNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive Bitcoin Cash address/pubkey directly from a hex private key.
pub fn derive_bitcoin_cash_from_private_key(
    private_key_hex: String,
    want_address: bool,
    want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let trimmed = private_key_hex.trim();
    if trimmed.len() != 64 {
        return Err(SpectraBridgeError::InvalidInput {
            message: "Private key hex must be exactly 64 characters.".into(),
        });
    }
    let bytes = hex::decode(trimmed)?;
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&bytes);
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(&key_bytes).map_err(SpectraBridgeError::failure)?;
    let pk = PublicKey::from_secret_key(&secp, &secret_key);
    Ok(DerivationResult {
        address: want_address.then(|| encode_p2pkh(BCH_MAINNET_VERSION, &pk.serialize())),
        public_key_hex: want_public_key.then(|| hex::encode(pk.serialize())),
        private_key_hex: None,
        account: 0,
        branch: 0,
        index: 0,
    })
}
