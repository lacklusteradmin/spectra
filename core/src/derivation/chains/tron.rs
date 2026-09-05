//! Tron: address validation, BIP-32 derivation, base58check encoding with
//! 0x41 prefix
//!
//! Tron's address derivation:
//!   keccak256(uncompressed_pubkey[1..])[12..32]  → 20-byte EVM-style hash
//!   prepend 0x41                                  → 21-byte payload
//!   base58check (default alphabet)                → "T…" address

use crate::derivation::primitives::{derive_bip39_seed, parse_bip32_path};
use secp256k1::{PublicKey, Secp256k1};

// ── Address validation + helpers (preserved) ─────────────────────────────

/// Derive a Tron address from a 65-byte uncompressed pubkey: keccak256 → last 20 bytes → 0x41 prefix → base58check.
pub fn pubkey_to_tron_address(pubkey_uncompressed: &[u8]) -> Result<String, String> {
    if pubkey_uncompressed.len() != 65 || pubkey_uncompressed[0] != 0x04 {
        return Err("expected 65-byte uncompressed public key".to_string());
    }
    let hash = keccak256(&pubkey_uncompressed[1..]);
    let addr_bytes = &hash[12..];
    let mut versioned = vec![0x41u8];
    versioned.extend_from_slice(addr_bytes);
    Ok(bs58::encode(&versioned).with_check().into_string())
}

/// Decode a Tron base58check address and return the 20-byte EVM-style hex account hash (without 0x41 prefix).
pub fn tron_base58_to_evm_hex(address: &str) -> Result<String, String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("base58 decode: {e}"))?;
    if decoded.len() != 21 || decoded[0] != 0x41 {
        return Err(format!(
            "invalid Tron address length/prefix: len={}",
            decoded.len()
        ));
    }
    Ok(hex::encode(&decoded[1..]))
}

// Keccak-256 hash; used for Tron address derivation.
fn keccak256(data: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    Keccak256::digest(data).into()
}


// Derive Tron address, public key, and private key from a mnemonic via BIP-39 + BIP-32 secp256k1.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, String> {
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
        let mut payload = vec![0x41u8];
        payload.extend_from_slice(&hash[12..]);
        Some(bs58::encode(&payload).with_check().into_string())
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

use crate::derivation::types::{parse_path_metadata, DerivationResult};
use crate::SpectraBridgeError;
use crate::derivation::primitives::ExtendedPrivateKey;

// Shared derivation logic for all Tron networks.
fn tron_internal(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
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

/// UniFFI export: derive Tron mainnet wallet from a seed phrase.
pub fn derive_tron(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    tron_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// UniFFI export: derive Tron Nile testnet wallet from a seed phrase.
pub fn derive_tron_nile(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    tron_internal(
        seed_phrase,
        derivation_path,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}
