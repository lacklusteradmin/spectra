//! Tezos: tz1 accounts from Ed25519 keys, and the base58check strings Tezos
//! writes addresses, keys, signatures and hashes in.
//!
//! An implicit account's address is base58check of a three-byte prefix and
//! the BLAKE2b-160 of its public key: `tz1` for Ed25519, `tz2` secp256k1,
//! `tz3` P-256 and `tz4` BLS12-381. An originated contract's (`KT1`) is a
//! hash of the operation that created it. Spectra's Tezos wallets hold
//! Ed25519 keys, derived from a phrase by SLIP-10 at
//! `m/44'/1729'/{account}'/0'` as Temple, Kukai, Umami and Taquito derive
//! them; a transfer may pay any of the five. `tz5` (ML-DSA-44) accounts are
//! not enabled on mainnet (`tz5_account_enable` is false) and are no
//! address here.

use ed25519_dalek::SigningKey;
use zeroize::Zeroizing;

use crate::derivation::error::DerivationError;
use crate::derivation::primitives::{blake2b, derive_bip39_seed, derive_slip10_ed25519_key};
use crate::derivation::types::{DerivationResult, Wants};

/// The base58check prefixes Tezos spells its values with, from Octez's
/// `base58.ml` (and Taquito's `prefixV2`).
pub(crate) mod prefix {
    pub const TZ1: [u8; 3] = [6, 161, 159];
    pub const TZ2: [u8; 3] = [6, 161, 161];
    pub const TZ3: [u8; 3] = [6, 161, 164];
    pub const TZ4: [u8; 3] = [6, 161, 166];
    pub const KT1: [u8; 3] = [2, 90, 121];
    /// An Ed25519 public key.
    pub const EDPK: [u8; 4] = [13, 15, 37, 217];
    /// An Ed25519 seed, 32 bytes.
    pub const EDSK_SEED: [u8; 4] = [13, 15, 58, 7];
    /// An Ed25519 secret key: the seed and its public key, 64 bytes.
    pub const EDSK_SECRET: [u8; 4] = [43, 246, 78, 7];
    pub const EDSIG: [u8; 5] = [9, 245, 205, 134, 18];
    pub const BLOCK: [u8; 2] = [1, 52];
    pub const OPERATION: [u8; 2] = [5, 116];
}

/// The signature scheme of an implicit account, as its address's prefix and
/// its one-byte tag in an operation name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TezosCurve {
    Ed25519,
    Secp256k1,
    P256,
    Bls12_381,
}

impl TezosCurve {
    const ALL: [Self; 4] = [Self::Ed25519, Self::Secp256k1, Self::P256, Self::Bls12_381];

    fn prefix(self) -> [u8; 3] {
        match self {
            Self::Ed25519 => prefix::TZ1,
            Self::Secp256k1 => prefix::TZ2,
            Self::P256 => prefix::TZ3,
            Self::Bls12_381 => prefix::TZ4,
        }
    }

    /// The tag a public key hash of this scheme carries in an operation.
    fn tag(self) -> u8 {
        match self {
            Self::Ed25519 => 0,
            Self::Secp256k1 => 1,
            Self::P256 => 2,
            Self::Bls12_381 => 3,
        }
    }
}

/// An account a Tezos operation names: an implicit account, a key's, or an
/// originated contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TezosAddress {
    Implicit { curve: TezosCurve, hash: [u8; 20] },
    Originated([u8; 20]),
}

impl TezosAddress {
    /// The address as wallets write it.
    pub(crate) fn encode(&self) -> String {
        match self {
            Self::Implicit { curve, hash } => base58check(&curve.prefix(), hash),
            Self::Originated(hash) => base58check(&prefix::KT1, hash),
        }
    }

    pub(crate) fn is_implicit(&self) -> bool {
        matches!(self, Self::Implicit { .. })
    }

    /// The 21 bytes an implicit account is in an operation's `source`: its
    /// scheme's tag and its hash. `None` for a contract, which signs nothing.
    pub(crate) fn public_key_hash_bytes(&self) -> Option<[u8; 21]> {
        match self {
            Self::Implicit { curve, hash } => {
                let mut bytes = [0u8; 21];
                bytes[0] = curve.tag();
                bytes[1..].copy_from_slice(hash);
                Some(bytes)
            }
            Self::Originated(_) => None,
        }
    }

    /// The 22 bytes the account is as a transaction's `destination`: `00`
    /// and its public key hash, or `01`, the contract's hash and a padding
    /// byte.
    pub(crate) fn contract_bytes(&self) -> [u8; 22] {
        let mut bytes = [0u8; 22];
        match self {
            Self::Implicit { .. } => {
                bytes[1..].copy_from_slice(
                    &self
                        .public_key_hash_bytes()
                        .expect("an implicit account has a key hash"),
                );
            }
            Self::Originated(hash) => {
                bytes[0] = 1;
                bytes[1..21].copy_from_slice(hash);
            }
        }
        bytes
    }
}

/// `payload` under `prefix`, with Bitcoin's four-byte double-SHA-256
/// checksum, as every Tezos string is written.
pub(crate) fn base58check(prefix: &[u8], payload: &[u8]) -> String {
    let mut bytes = Vec::with_capacity(prefix.len() + payload.len());
    bytes.extend_from_slice(prefix);
    bytes.extend_from_slice(payload);
    bs58::encode(bytes).with_check().into_string()
}

/// The `len` bytes `input` carries under `prefix`, or `None` when it is not
/// that kind of value or its checksum fails.
pub(crate) fn decode_base58check(input: &str, prefix: &[u8], len: usize) -> Option<Vec<u8>> {
    let bytes = bs58::decode(input).with_check(None).into_vec().ok()?;
    (bytes.len() == prefix.len() + len && bytes.starts_with(prefix))
        .then(|| bytes[prefix.len()..].to_vec())
}

/// The account `address` names: a `tz1`–`tz4` implicit account or a `KT1`
/// contract, with an intact checksum and written exactly (base58 is case
/// sensitive, so nothing is folded).
pub(crate) fn decode_address(address: &str) -> Result<TezosAddress, DerivationError> {
    let hash = |prefix: &[u8]| -> Option<[u8; 20]> {
        decode_base58check(address, prefix, 20)?.try_into().ok()
    };
    TezosCurve::ALL
        .into_iter()
        .find_map(|curve| {
            Some(TezosAddress::Implicit {
                curve,
                hash: hash(&curve.prefix())?,
            })
        })
        .or_else(|| hash(&prefix::KT1).map(TezosAddress::Originated))
        .ok_or_else(|| DerivationError::refused("Not a %@ address.", ["Tezos".to_string()]))
}

/// The tz1 account an Ed25519 public key holds: BLAKE2b-160 of the key.
pub(crate) fn address_from_public_key(public_key: &[u8; 32]) -> String {
    TezosAddress::Implicit {
        curve: TezosCurve::Ed25519,
        hash: blake2b::<20>(&[public_key]),
    }
    .encode()
}

/// An Ed25519 public key as a reveal names it: `edpk…`.
pub(crate) fn public_key_string(public_key: &[u8; 32]) -> String {
    base58check(&prefix::EDPK, public_key)
}

/// An Ed25519 seed as Temple and octez-client export it: the 54-character
/// `edsk…` of the 32-byte seed.
pub(crate) fn secret_key_string(seed: &[u8; 32]) -> Zeroizing<String> {
    Zeroizing::new(base58check(&prefix::EDSK_SEED, seed))
}

/// The Ed25519 seed an `edsk…` string holds, in either form wallets export:
/// the 32-byte seed, or the 64-byte secret key whose second half must be the
/// seed's own public key. `None` when `input` is neither.
pub(crate) fn read_secret_key(input: &str) -> Option<Result<Zeroizing<[u8; 32]>, DerivationError>> {
    if !input.starts_with("edsk") {
        return None;
    }
    if let Some(seed) = decode_base58check(input, &prefix::EDSK_SEED, 32) {
        let seed = Zeroizing::new(seed);
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&seed);
        return Some(Ok(key));
    }
    let pair = Zeroizing::new(decode_base58check(input, &prefix::EDSK_SECRET, 64)?);
    let mut seed = Zeroizing::new([0u8; 32]);
    seed.copy_from_slice(&pair[..32]);
    Some(
        if SigningKey::from_bytes(&seed).verifying_key().as_bytes() == &pair[32..] {
            Ok(seed)
        } else {
            Err(DerivationError::invalid(
                "This Tezos secret key's public half does not belong to it.",
            ))
        },
    )
}

/// Derive a Tezos account from a seed phrase along a SLIP-10 Ed25519 path.
pub(crate) fn derive_from_seed_phrase(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let private_key = derive_slip10_ed25519_key(seed.as_ref(), derivation_path, None)?;
    let public_key = SigningKey::from_bytes(&private_key)
        .verifying_key()
        .to_bytes();
    Ok(DerivationResult {
        address: wants.address.then(|| address_from_public_key(&public_key)),
        public_key_hex: wants.public_key_hex(&public_key),
        private_key_hex: wants.private_key_hex(&*private_key),
    })
}

#[cfg(test)]
#[path = "tests/tezos.rs"]
mod tests;
