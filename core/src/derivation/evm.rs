//! EVM address derivation (keccak256 over uncompressed pubkey) + EIP-55
//! checksum formatting + address validation.

use crate::derivation::error::DerivationError;

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

use secp256k1::{PublicKey, Secp256k1, SecretKey};

/// Derive an EVM wallet from a seed phrase.
///
/// One function for the family: an EVM address does not depend on which EVM
/// chain it is for.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let secp = Secp256k1::new();
    let private_key =
        crate::derivation::bip32::phrase_key(seed_phrase, passphrase, derivation_path)?;
    let public_key = PublicKey::from_secret_key(&secp, &private_key);
    let private_bytes = private_key.secret_bytes();

    let address = if wants.address {
        let uncompressed = public_key.serialize_uncompressed();
        let hash = keccak256(&uncompressed[1..]);
        Some(eip55_checksum(&hash[12..]))
    } else {
        None
    };

    Ok(DerivationResult {
        address,
        public_key_hex: wants.public_key_hex(&public_key.serialize()),
        private_key_hex: wants.private_key_hex(&private_bytes),
    })
}

// Derive an EVM address and public key directly from a 32-byte raw private key.
pub(crate) fn derive_from_private_key_bytes(
    key_bytes: &[u8; 32],
    want_address: bool,
    want_public_key: bool,
) -> Result<(Option<String>, Option<String>), DerivationError> {
    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(key_bytes)
        .map_err(|e| DerivationError::Invalid(format!("invalid private key: {e}").into()))?;
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

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::derivation::types::{DerivationResult, Wants};
use crate::error::SpectraBridgeError;

// Validate and decode a 64-character hex private key string into 32 raw bytes.
fn decode_privkey_hex(hex_str: &str) -> Result<[u8; 32], SpectraBridgeError> {
    let trimmed = hex_str.trim();
    if trimmed.len() != 64 {
        return Err(SpectraBridgeError::InvalidInput {
            reason: "Private key hex must be exactly 64 characters.".into(),
        });
    }
    let bytes = hex::decode(trimmed)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Derive an EVM address and public key from a raw private key hex string.
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
    })
}
