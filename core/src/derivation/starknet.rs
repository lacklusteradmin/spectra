//! Starknet accounts as Ready (formerly Argent X) derives them, and the field
//! elements Starknet writes everything in.
//!
//! A phrase's Stark key comes from two BIP-32 trees. The first is the
//! phrase's own: its Ethereum key at `m/44'/60'/0'/0/0`, which Ready keeps as
//! the wallet's secret. That key, without its leading zero bytes, seeds the
//! second, whose child at `m/44'/9004'/0'/0/{index}` is ground into a Stark
//! private key: the first SHA-256 of the child and a counter that falls below
//! the largest multiple of the curve order, reduced by it (StarkWare's
//! `grind_key`). The public key is the x-coordinate of the key's point.
//!
//! The account is a contract at an address that hashes its class, its
//! constructor's arguments and a salt, deployed by no one (deployer zero):
//! Argent account 0.4.0, owned by the key as a Starknet signer with no
//! guardian, salted with the public key. It holds funds before it exists,
//! and its first transaction deploys it.

use num_bigint::BigUint;
use starknet_crypto::Felt;

use crate::derivation::error::DerivationError;
use crate::derivation::types::{DerivationResult, Wants};
use crate::registry::Chain;

/// The field's prime, `2^251 + 17·2^192 + 1`, big-endian.
const FIELD_PRIME: [u8; 32] =
    hex_literal(b"0800000000000011000000000000000000000000000000000000000000000001");

/// The Stark curve's order, big-endian: a private key is below it.
const CURVE_ORDER: [u8; 32] =
    hex_literal(b"0800000000000010ffffffffffffffffb781126dcae7b2321e66a241adc64d2f");

/// The bound below which a contract address lies, `2^251 - 256`, big-endian.
const ADDRESS_BOUND: [u8; 32] =
    hex_literal(b"07ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff00");

/// Where the phrase's Ethereum key sits: what ethers' `HDNodeWallet`
/// reads by default, and Ready keeps as the wallet's secret.
const ETHEREUM_KEY_PATH: &str = "m/44'/60'/0'/0/0";

/// 32 bytes written as 64 hex digits, at compile time.
const fn hex_literal(text: &[u8; 64]) -> [u8; 32] {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("not a lowercase hex digit"),
        }
    }
    let mut bytes = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        bytes[i] = digit(text[2 * i]) << 4 | digit(text[2 * i + 1]);
        i += 1;
    }
    bytes
}

/// `text` as 32 big-endian bytes: hex with or without `0x`, one to 64
/// digits, in either case.
fn hex_word(text: &str) -> Option<[u8; 32]> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if digits.is_empty() || digits.len() > 64 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let padded = format!("{digits:0>64}");
    let mut word = [0u8; 32];
    hex::decode_to_slice(padded, &mut word).ok()?;
    Some(word)
}

/// `text` as a field element: hex with or without `0x`, at most 64 digits,
/// below the field's prime. Nothing is reduced, so one value has one
/// spelling.
pub(crate) fn parse_felt(text: &str) -> Option<Felt> {
    let word = hex_word(text.trim())?;
    (word < FIELD_PRIME).then(|| Felt::from_bytes_be(&word))
}

/// A field element as an address or hash is written: `0x` and 64 lowercase
/// digits.
pub(crate) fn felt_hex(felt: &Felt) -> String {
    format!("0x{}", hex::encode(felt.to_bytes_be()))
}

/// A felt in a stored artifact, as its `0x`-padded hex: `#[serde(with =
/// "felt_serde")]`, and `felt_serde::list` for a list of them.
pub(crate) mod felt_serde {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};
    use starknet_crypto::Felt;

    pub fn serialize<S: Serializer>(felt: &Felt, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::felt_hex(felt))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Felt, D::Error> {
        let text = String::deserialize(deserializer)?;
        super::parse_felt(&text).ok_or_else(|| D::Error::custom("not a felt"))
    }

    pub mod list {
        use super::*;

        pub fn serialize<S: Serializer>(felts: &[Felt], serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_seq(felts.iter().map(super::super::felt_hex))
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<Felt>, D::Error> {
            Vec::<String>::deserialize(deserializer)?
                .iter()
                .map(|text| {
                    super::super::parse_felt(text).ok_or_else(|| D::Error::custom("not a felt"))
                })
                .collect()
        }
    }
}

/// Starknet's Keccak: Keccak-256 of `bytes` with its top six bits cleared,
/// which an entry point's selector is of its name.
pub(crate) fn starknet_keccak(bytes: &[u8]) -> Felt {
    let mut hash = crate::derivation::evm::keccak256(bytes);
    hash[0] &= 0x03;
    Felt::from_bytes_be(&hash)
}

/// The selector of the entry point `name`.
pub(crate) fn selector(name: &str) -> Felt {
    starknet_keccak(name.as_bytes())
}

/// An address in the mixed case starknet.js's `getChecksumAddress` writes:
/// each hex letter upper case where the matching nibble of the Starknet
/// Keccak of the address's minimal bytes is 8 or more.
pub(crate) fn checksum_address(address: &Felt) -> String {
    let bytes = address.to_bytes_be();
    let start = bytes.iter().position(|b| *b != 0).unwrap_or(31);
    let hash = starknet_keccak(&bytes[start..]).to_bytes_be();
    let digits = hex::encode(bytes);
    let mut out = String::from("0x");
    for (i, c) in digits.chars().enumerate() {
        let nibble = if i % 2 == 0 {
            hash[i / 2] >> 4
        } else {
            hash[i / 2] & 0x0f
        };
        out.push(if nibble >= 8 {
            c.to_ascii_uppercase()
        } else {
            c
        });
    }
    out
}

/// The account an address on `chain` names. Refused unless it is hex of at
/// most 64 digits below the address bound, not zero, and — when written in
/// mixed case — in the case its checksum gives it: a mistyped digit of a
/// checksummed address is caught rather than paid.
pub(crate) fn decode_address(chain: Chain, address: &str) -> Result<Felt, DerivationError> {
    chain.starknet_network()?;
    let invalid = || {
        DerivationError::refused(
            "Not a %@ address.",
            [chain.chain_display_name().to_string()],
        )
    };
    let text = address.trim();
    let word = hex_word(text).ok_or_else(invalid)?;
    if word >= ADDRESS_BOUND || word == [0u8; 32] {
        return Err(invalid());
    }
    let felt = Felt::from_bytes_be(&word);
    let digits = text.trim_start_matches("0x").trim_start_matches("0X");
    let mixed = digits.bytes().any(|b| b.is_ascii_uppercase())
        && digits.bytes().any(|b| b.is_ascii_lowercase());
    if mixed && !checksum_address(&felt).ends_with(digits) {
        return Err(invalid());
    }
    Ok(felt)
}

/// A Stark private key from its 32 big-endian bytes: above zero and below
/// the curve order.
pub(crate) fn private_key(bytes: &[u8]) -> Result<Felt, DerivationError> {
    let word: [u8; 32] = bytes
        .try_into()
        .map_err(|_| DerivationError::invalid("A Stark private key is 32 bytes"))?;
    if word == [0u8; 32] || word >= CURVE_ORDER {
        return Err(DerivationError::refused(
            "This is not a private key %@ takes.",
            [Chain::Starknet.chain_display_name()],
        ));
    }
    Ok(Felt::from_bytes_be(&word))
}

/// StarkWare's `grind_key`: the first `SHA-256(seed ‖ i)` below the largest
/// multiple of the curve order that fits 256 bits, reduced by the order.
/// `i` is written in its fewest big-endian bytes, `0` as one zero byte.
pub(crate) fn grind_key(seed: &[u8; 32]) -> Felt {
    use sha2::Digest as _;
    let order = BigUint::from_bytes_be(&CURVE_ORDER);
    let space: BigUint = BigUint::from(1u8) << 256usize;
    let remainder: BigUint = &space % &order;
    let limit: BigUint = space - remainder;
    let mut index: u64 = 0;
    loop {
        let counter = index.to_be_bytes();
        let start = counter
            .iter()
            .position(|b| *b != 0)
            .unwrap_or(counter.len() - 1);
        let mut hasher = sha2::Sha256::new();
        hasher.update(seed);
        hasher.update(&counter[start..]);
        let key = BigUint::from_bytes_be(&hasher.finalize());
        if key < limit {
            let reduced = (key % &order).to_bytes_be();
            let mut word = [0u8; 32];
            word[32 - reduced.len()..].copy_from_slice(&reduced);
            return Felt::from_bytes_be(&word);
        }
        index += 1;
    }
}

/// The public key of a Stark private key: its point's x-coordinate.
pub(crate) fn public_key(private_key: &Felt) -> Felt {
    starknet_crypto::get_public_key(private_key)
}

/// The Stark private key a phrase holds at `path` in Ready's second tree.
pub(crate) fn phrase_private_key(
    seed_phrase: &str,
    passphrase: Option<&str>,
    path: &str,
) -> Result<Felt, DerivationError> {
    let ethereum =
        crate::derivation::bip32::phrase_key(seed_phrase, passphrase, ETHEREUM_KEY_PATH)?;
    let secret = zeroize::Zeroizing::new(ethereum.secret_bytes());
    // ArgentSigner writes the secret through `num.toHex`, a number's
    // fewest digits: a leading zero byte is not part of the seed.
    let start = secret.iter().position(|b| *b != 0).unwrap_or(secret.len());
    let seed = &secret[start..];
    // @scure/bip32 refuses a seed shorter than 16 bytes; no key that short
    // occurs but by a 2^-128 chance.
    if seed.len() < 16 {
        return Err(DerivationError::Internal(
            "The phrase's Ethereum key is too short to seed a Starknet key".into(),
        ));
    }
    let child =
        crate::derivation::bip32::derive(seed, &crate::derivation::bip32::parse_path(path)?)?;
    let child = zeroize::Zeroizing::new(child.private_key.secret_bytes());
    Ok(grind_key(&child))
}

/// Pedersen's hash on an array, as Starknet hashes an address's parts: the
/// chain of `h(h(…h(0, a₀)…), aₙ)` closed with the length.
fn pedersen_array(elements: &[Felt]) -> Felt {
    let folded = elements.iter().fold(Felt::ZERO, |hash, element| {
        starknet_crypto::pedersen_hash(&hash, element)
    });
    starknet_crypto::pedersen_hash(&folded, &Felt::from(elements.len() as u64))
}

/// The account's constructor arguments: the owner as a Starknet signer
/// (variant 0, its public key) and no guardian (`Option::None`, variant 1).
pub(crate) fn constructor_calldata(public_key: &Felt) -> [Felt; 3] {
    [Felt::ZERO, *public_key, Felt::ONE]
}

/// The address of the account `public_key` owns on `chain`: Starknet's
/// contract address of the account class, the public key as salt, the
/// constructor arguments and deployer zero, reduced below the address bound.
pub(crate) fn account_address(chain: Chain, public_key: &Felt) -> Result<Felt, DerivationError> {
    let network = chain.starknet_network()?;
    let class_hash = parse_felt(network.account_class_hash)
        .ok_or_else(|| DerivationError::Internal("Invalid Starknet account class".into()))?;
    let prefix = Felt::from_bytes_be_slice(b"STARKNET_CONTRACT_ADDRESS");
    let hash = pedersen_array(&[
        prefix,
        Felt::ZERO,
        *public_key,
        class_hash,
        pedersen_array(&constructor_calldata(public_key)),
    ]);
    let bound = BigUint::from_bytes_be(&ADDRESS_BOUND);
    let reduced = (BigUint::from_bytes_be(&hash.to_bytes_be()) % bound).to_bytes_be();
    Ok(Felt::from_bytes_be_slice(&reduced))
}

/// Derive a Ready account on `chain` from a phrase along `derivation_path`,
/// a path in Ready's second tree.
pub(crate) fn derive_from_seed_phrase(
    chain: Chain,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let private = phrase_private_key(seed_phrase, passphrase, derivation_path)?;
    let public = public_key(&private);
    Ok(DerivationResult {
        address: if wants.address {
            Some(felt_hex(&account_address(chain, &public)?))
        } else {
            None
        },
        public_key_hex: wants.public_key_hex(&public.to_bytes_be()),
        private_key_hex: wants.private_key_hex(&private.to_bytes_be()),
    })
}

#[cfg(test)]
#[path = "tests/starknet.rs"]
mod tests;
