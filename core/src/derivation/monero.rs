//! Monero: address derivation + chunked-base58 address encoding.
//!
//! A Monero wallet is restored from one of Monero's own phrases, never from
//! BIP-39, which no Monero wallet reads:
//!
//! **Monero seed (25 words)** — monero-wallet-cli, the GUI, Cake and every
//!   Monero wallet. The words encode the spend key itself
//!   ([`super::monero_words`]).
//!
//! **Polyseed (16 words)** — Feather, Cake and current wallets. The words
//!   carry a 150-bit secret the key is derived from ([`super::polyseed`]).
//!
//! Either way:
//!     private_spend = sc_reduce32(key)
//!     private_view  = sc_reduce32(Keccak256(private_spend))
//!
//! Address encoding uses Monero's chunked Base58 with the chain-specific
//! network byte (0x12 = mainnet, 0x18 = stagenet).

use crate::derivation::error::DerivationError;

use zeroize::Zeroizing;

/// Derive (private_spend, public_spend, private_view, public_view) from
/// a 32-byte BIP-39 seed prefix.
/// Expand a 32-byte spend seed into (private_spend, public_spend, private_view, public_view) via sc_reduce32 + Keccak256.
pub(crate) fn derive_monero_keys_from_spend_seed(
    spend_seed: &[u8; 32],
) -> Result<([u8; 32], [u8; 32], [u8; 32], [u8; 32]), DerivationError> {
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let private_spend = DalekScalar::from_bytes_mod_order(*spend_seed).to_bytes();

    use sha3::{Digest, Keccak256};
    let spend_hash: [u8; 32] = Keccak256::digest(private_spend).into();
    let private_view = DalekScalar::from_bytes_mod_order(spend_hash).to_bytes();

    let public_spend = (DalekScalar::from_bytes_mod_order(private_spend) * ED25519_BASEPOINT_POINT)
        .compress()
        .to_bytes();
    let public_view = (DalekScalar::from_bytes_mod_order(private_view) * ED25519_BASEPOINT_POINT)
        .compress()
        .to_bytes();

    Ok((private_spend, public_spend, private_view, public_view))
}

/// Encode a Monero standard address: `network_byte || public_spend (32) ||
/// public_view (32) || keccak256(prev)[0..4]`, then chunked Base58.
/// Network byte: 0x12 for `Chain::Monero`, 0x18 for `Chain::MoneroStagenet`.
/// Encode a Monero standard address from public spend + view keys with the given network byte.
pub(crate) fn encode_monero_main_address(
    public_spend: &[u8; 32],
    public_view: &[u8; 32],
    is_mainnet: bool,
) -> Result<String, DerivationError> {
    let network_byte: u8 = if is_mainnet { 0x12 } else { 0x18 };
    let mut payload = Vec::with_capacity(69);
    payload.push(network_byte);
    payload.extend_from_slice(public_spend);
    payload.extend_from_slice(public_view);
    use sha3::{Digest, Keccak256};
    let digest: [u8; 32] = Keccak256::digest(&payload).into();
    payload.extend_from_slice(&digest[..4]);
    Ok(monero_base58_encode(&payload))
}

/// Monero's chunked Base58: split the input into 8-byte blocks, each
/// encoding to a fixed-width 11-char chunk. The alphabet differs in
/// ordering from BIP-58 but uses the same 58 characters.
/// Monero chunked Base58: split input into 8-byte blocks, encode each to a fixed-width 11-char chunk.
pub(crate) fn monero_base58_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    const FULL_BLOCK_SIZE: usize = 8;
    const FULL_ENCODED_BLOCK_SIZE: usize = 11;
    const ENCODED_BLOCK_SIZES: [usize; FULL_BLOCK_SIZE + 1] = [0, 2, 3, 5, 6, 7, 9, 10, 11];

    let mut out = String::new();
    let full_blocks = data.len() / FULL_BLOCK_SIZE;
    let remainder = data.len() % FULL_BLOCK_SIZE;

    for i in 0..full_blocks {
        let start = i * FULL_BLOCK_SIZE;
        let block = &data[start..start + FULL_BLOCK_SIZE];
        let mut value: u64 = 0;
        for &b in block {
            value = (value << 8) | u64::from(b);
        }
        let mut chars = [b'1'; FULL_ENCODED_BLOCK_SIZE];
        for j in (0..FULL_ENCODED_BLOCK_SIZE).rev() {
            chars[j] = ALPHABET[(value % 58) as usize];
            value /= 58;
        }
        out.push_str(std::str::from_utf8(&chars).unwrap());
    }
    if remainder > 0 {
        let block = &data[full_blocks * FULL_BLOCK_SIZE..];
        let mut value: u64 = 0;
        for &b in block {
            value = (value << 8) | u64::from(b);
        }
        let encoded_len = ENCODED_BLOCK_SIZES[remainder];
        let mut chars = vec![b'1'; encoded_len];
        for j in (0..encoded_len).rev() {
            chars[j] = ALPHABET[(value % 58) as usize];
            value /= 58;
        }
        out.push_str(std::str::from_utf8(&chars).unwrap());
    }
    out
}

/// The spend key a Monero phrase stands for: 25 words are a Monero seed, 16
/// a Polyseed, and anything else is refused.
pub(crate) fn spend_key_from_phrase(
    seed_phrase: &str,
) -> Result<Zeroizing<[u8; 32]>, DerivationError> {
    match seed_phrase.split_whitespace().count() {
        25 => super::monero_words::decode(seed_phrase),
        16 => Ok(super::polyseed::decode(seed_phrase)?.monero_key()),
        count => Err(DerivationError::refused(
            "A Monero phrase has 25 words, or 16 for a Polyseed, not %@.",
            [count],
        )),
    }
}

pub(crate) fn derive_from_seed_phrase(
    is_mainnet: bool,
    seed_phrase: &str,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, DerivationError> {
    let spend_seed = spend_key_from_phrase(seed_phrase)?;
    let (private_spend, public_spend, private_view, public_view) =
        derive_monero_keys_from_spend_seed(&spend_seed)?;

    let address = if want_address {
        Some(encode_monero_main_address(
            &public_spend,
            &public_view,
            is_mainnet,
        )?)
    } else {
        None
    };

    let public_key_hex = if want_public_key {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&public_spend);
        both[32..].copy_from_slice(&public_view);
        Some(hex::encode(both))
    } else {
        None
    };

    let private_key_hex = if want_private_key {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&private_spend);
        both[32..].copy_from_slice(&private_view);
        Some(hex::encode(both))
    } else {
        None
    };

    Ok((address, public_key_hex, private_key_hex))
}

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::SpectraBridgeError;
use crate::derivation::types::DerivationResult;

/// Derive Monero mainnet keys from a Monero seed or Polyseed.
pub fn derive_monero(
    seed_phrase: String,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        true,
        &seed_phrase,
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex,
        account: 0,
        branch: 0,
        index: 0,
    })
}

/// Derive Monero stagenet keys from a Monero seed or Polyseed.
pub fn derive_monero_stagenet(
    seed_phrase: String,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        false,
        &seed_phrase,
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex,
        account: 0,
        branch: 0,
        index: 0,
    })
}
