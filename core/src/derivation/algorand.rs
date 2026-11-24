//! Algorand: addresses, the 25-word mnemonic, and BIP-39 accounts along
//! ARC-52.
//!
//! An address is the account's 32-byte Ed25519 public key and the last four
//! bytes of its SHA-512/256, in RFC 4648 base32 without padding: 58 letters.
//!
//! Algorand's own wallets (Pera, Defly, Lute, `goal`) write one account's key
//! as its 25-word mnemonic, go-algorand's `mnemonic` package: the 32-byte
//! Ed25519 seed as 24 eleven-bit words of the BIP-39 English list, least
//! significant bit first, then a checksum word, the first eleven bits of
//! SHA-512/256 of the seed. The mnemonic is the key: it holds one account
//! and walks no path.
//!
//! Pera's Universal Wallet writes a BIP-39 phrase instead, whose accounts
//! derive along ARC-52 at `m/44'/283'/{account}'/0/0`: BIP32-Ed25519 from a
//! root of SHA-512 over the BIP-39 seed, keeping 247 bits of each step's
//! `ZL` (Peikert's amendment). Such an account's key is the extended key
//! `kL ‖ kR`, which signs as it is rather than as a seed.
//!
//! Vectors from algosdk and the Algorand Foundation's xhd-wallet-api are in
//! `core/tests/fixtures/algorand-vectors.json`.

use sha2::{Digest, Sha256, Sha512, Sha512_256};
use zeroize::Zeroizing;

use crate::derivation::ed25519_extended;
use crate::derivation::error::DerivationError;
use crate::derivation::types::{DerivationResult, Wants};
use crate::registry::Chain;

/// The words of a mnemonic: 24 for the key and one checksum.
pub(crate) const MNEMONIC_WORDS: usize = 25;

/// The most levels an ARC-52 path walks: Peikert's amendment at `g = 9`
/// keeps child keys safe to depth 2^3.
const MAX_PATH_DEPTH: usize = 8;

fn not_an_address() -> DerivationError {
    DerivationError::refused("Not a %@ address.", [Chain::Algorand.chain_display_name()])
}

/// The address of `public_key`.
pub(crate) fn address_from_public_key(public_key: &[u8; 32]) -> String {
    let checksum = Sha512_256::digest(public_key);
    let mut bytes = [0u8; 36];
    bytes[..32].copy_from_slice(public_key);
    bytes[32..].copy_from_slice(&checksum[28..]);
    data_encoding::BASE32_NOPAD.encode(&bytes)
}

/// The public key an address names. Read in either case, as base32 is;
/// refused unless it is 58 letters whose trailing bits are zero and whose
/// checksum is its key's.
pub(crate) fn decode_address(address: &str) -> Result<[u8; 32], DerivationError> {
    let canonical = address.trim().to_ascii_uppercase();
    if canonical.len() != 58 {
        return Err(not_an_address());
    }
    let bytes = data_encoding::BASE32_NOPAD
        .decode(canonical.as_bytes())
        .map_err(|_| not_an_address())?;
    let public: [u8; 32] = bytes
        .get(..32)
        .filter(|_| bytes.len() == 36)
        .and_then(|key| key.try_into().ok())
        .ok_or_else(not_an_address)?;
    if address_from_public_key(&public) != canonical {
        return Err(not_an_address());
    }
    Ok(public)
}

/// `bytes` as 11-bit numbers, least significant bit first, the last padded
/// with zeros.
fn to_eleven_bits(bytes: &[u8]) -> Vec<u16> {
    let (mut out, mut buffer, mut bits) = (Vec::new(), 0u32, 0u32);
    for &byte in bytes {
        buffer |= u32::from(byte) << bits;
        bits += 8;
        if bits >= 11 {
            out.push((buffer & 0x7ff) as u16);
            buffer >>= 11;
            bits -= 11;
        }
    }
    if bits != 0 {
        out.push((buffer & 0x7ff) as u16);
    }
    out
}

/// The checksum word's index: the first 11 bits of SHA-512/256 of `seed`.
fn checksum_word(seed: &[u8; 32]) -> u16 {
    to_eleven_bits(&Sha512_256::digest(seed)[..2])[0]
}

fn wordlist() -> &'static [&'static str; 2048] {
    bip39::Language::English.word_list()
}

/// The 25-word mnemonic of the 32-byte key `seed`.
pub(crate) fn mnemonic_from_seed(seed: &[u8; 32]) -> Zeroizing<String> {
    let words = wordlist();
    let mut indices = Zeroizing::new(to_eleven_bits(seed));
    indices.push(checksum_word(seed));
    Zeroizing::new(
        indices
            .iter()
            .map(|&index| words[usize::from(index)])
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The 32-byte key a 25-word mnemonic holds. Refused, as go-algorand and
/// algosdk refuse it, unless every word is on the BIP-39 English list, the
/// 24 key words leave the bits past the key zero, and the checksum word is
/// the key's.
pub(crate) fn seed_from_mnemonic(mnemonic: &str) -> Result<Zeroizing<[u8; 32]>, DerivationError> {
    let invalid = || {
        DerivationError::refused(
            "Not an %@ mnemonic.",
            [Chain::Algorand.chain_display_name()],
        )
    };
    let words: Vec<&str> = mnemonic.split_whitespace().collect();
    if words.len() != MNEMONIC_WORDS {
        return Err(invalid());
    }
    let indices = Zeroizing::new(
        words
            .iter()
            .map(|word| bip39::Language::English.find_word(word))
            .collect::<Option<Vec<u16>>>()
            .ok_or_else(invalid)?,
    );
    // 24 words are 264 bits: the key's 256 and eight that must be zero.
    let mut bytes = Zeroizing::new(Vec::with_capacity(33));
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &index in &indices[..24] {
        buffer |= u32::from(index) << bits;
        bits += 11;
        while bits >= 8 {
            bytes.push((buffer & 0xff) as u8);
            buffer >>= 8;
            bits -= 8;
        }
    }
    if bytes.len() != 33 || bytes[32] != 0 {
        return Err(invalid());
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    seed.copy_from_slice(&bytes[..32]);
    if checksum_word(&seed) != indices[24] {
        return Err(invalid());
    }
    Ok(seed)
}

/// A new 25-word mnemonic, of a key from the operating system's randomness.
pub(crate) fn generate_mnemonic() -> Zeroizing<String> {
    use rand::RngCore;
    let mut seed = Zeroizing::new([0u8; 32]);
    rand::rngs::OsRng.fill_bytes(&mut *seed);
    mnemonic_from_seed(&seed)
}

/// Whether `phrase` is written as a 25-word mnemonic rather than a BIP-39
/// phrase, by its length, as the setup verdict tells them apart.
pub(crate) fn is_mnemonic(phrase: &str) -> bool {
    phrase.split_whitespace().count() == MNEMONIC_WORDS
}

/// ARC-52's root extended key of a BIP-39 seed: SHA-512 of the seed,
/// rehashed until bit 253 of `kL` is clear, then clamped; the chain code is
/// SHA-256 of `0x01 ‖ seed`.
fn arc52_root(seed: &[u8; 64]) -> Result<Zeroizing<[u8; 96]>, DerivationError> {
    let mut k = Zeroizing::new(<[u8; 64]>::from(Sha512::digest(seed)));
    while k[31] & 0b0010_0000 != 0 {
        let next = crate::derivation::primitives::hmac_sha512(&k[..32], &[&k[32..]])?;
        k.copy_from_slice(&*next);
    }
    k[0] &= 0b1111_1000;
    k[31] &= 0b0111_1111;
    k[31] |= 0b0100_0000;
    let mut root = Zeroizing::new([0u8; 96]);
    root[..64].copy_from_slice(&*k);
    root[64..].copy_from_slice(
        &Sha256::new()
            .chain_update([0x01])
            .chain_update(seed)
            .finalize(),
    );
    Ok(root)
}

/// The ARC-52 extended key `kL ‖ kR` a BIP-39 phrase holds at `path`, and
/// its public key.
fn arc52_key(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
) -> Result<(Zeroizing<[u8; 64]>, [u8; 32]), DerivationError> {
    let path =
        crate::derivation::bip32::indices(&crate::derivation::bip32::parse_path(derivation_path)?);
    if path.is_empty() || path.len() > MAX_PATH_DEPTH {
        return Err(DerivationError::refused(
            "An %@ account from a BIP-39 phrase derives along a path of one to eight levels.",
            [Chain::Algorand.chain_display_name()],
        ));
    }
    let seed = crate::derivation::primitives::derive_bip39_seed(
        seed_phrase,
        passphrase.unwrap_or(""),
        0,
        None,
        None,
    )?;
    let mut node = arc52_root(&seed)?;
    for index in path {
        node = ed25519_extended::derive_child(&node, index, ed25519_extended::PEIKERT_BITS)?;
    }
    let mut key = Zeroizing::new([0u8; 64]);
    key.copy_from_slice(&node[..64]);
    let public = ed25519_extended::public_key(&key)?;
    Ok((key, public))
}

/// Derive an Algorand account from a phrase: a 25-word mnemonic's one
/// account, which takes no path and no passphrase, or a BIP-39 phrase's
/// account along `derivation_path` by ARC-52. The private key is the
/// mnemonic's 32-byte seed or the BIP-39 account's 64-byte extended key.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    if is_mnemonic(seed_phrase) {
        if !derivation_path.trim().is_empty() {
            return Err(DerivationError::refused(
                "An %@ 25-word mnemonic is one account's key and derives without a path.",
                [Chain::Algorand.chain_display_name()],
            ));
        }
        if passphrase.is_some_and(|passphrase| !passphrase.is_empty()) {
            return Err(DerivationError::refused(
                "An %@ 25-word mnemonic takes no passphrase.",
                [Chain::Algorand.chain_display_name()],
            ));
        }
        let seed = seed_from_mnemonic(seed_phrase)?;
        let public = ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes();
        return Ok(DerivationResult {
            address: wants.address.then(|| address_from_public_key(&public)),
            public_key_hex: wants.public_key_hex(&public),
            private_key_hex: wants.private_key_hex(&*seed),
        });
    }
    let (key, public) = arc52_key(seed_phrase, derivation_path, passphrase)?;
    Ok(DerivationResult {
        address: wants.address.then(|| address_from_public_key(&public)),
        public_key_hex: wants.public_key_hex(&public),
        private_key_hex: wants.private_key_hex(&*key),
    })
}

#[cfg(test)]
#[path = "tests/algorand.rs"]
mod tests;
