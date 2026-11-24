//! Filecoin addresses, as go-address writes them: the network's letter (`f`
//! on mainnet), the protocol's digit, then the payload and a BLAKE2b-32
//! checksum of the address's bytes in unpadded lowercase base32. An ID
//! address writes its number in decimal instead, and a delegated address
//! writes its namespace in decimal and an `f` before the rest.
//!
//! A secp256k1 key's own account is protocol 1 (`f1…`), whose payload is
//! BLAKE2b-160 of the key's 65-byte uncompressed point.

use std::sync::LazyLock;

use secp256k1::{PublicKey, Secp256k1};

use crate::derivation::error::DerivationError;
use crate::derivation::primitives::blake2b;
use crate::derivation::types::{DerivationResult, Wants};
use crate::registry::Chain;

/// What an address names, by its protocol digit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FilecoinProtocol {
    /// `f0`: an actor by its number in the state tree.
    Id,
    /// `f1`: the account of a secp256k1 key.
    Secp256k1,
    /// `f2`: an actor another actor created, such as a multisig.
    Actor,
    /// `f3`: the account of a BLS key.
    Bls,
    /// `f410f`: an Ethereum-style account or contract, under the Ethereum
    /// Address Manager's namespace.
    Delegated,
}

/// The Ethereum Address Manager's actor ID, which is the namespace of every
/// delegated address an actor can be created at.
const EAM_NAMESPACE: u64 = 10;

/// The longest ID go-address reads: a signed 64-bit number.
const MAX_ID: u64 = i64::MAX as u64;

/// An address a message can carry, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilecoinAddress {
    protocol: FilecoinProtocol,
    /// The protocol byte, then the payload: what a message carries.
    bytes: Vec<u8>,
}

impl FilecoinAddress {
    pub(crate) fn protocol(&self) -> FilecoinProtocol {
        self.protocol
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Unpadded base32 in go-address's lowercase alphabet. A last character
/// with stray low bits is refused, as go-address's re-encoding check does.
static BASE32: LazyLock<data_encoding::Encoding> = LazyLock::new(|| {
    let mut spec = data_encoding::Specification::new();
    spec.symbols.push_str("abcdefghijklmnopqrstuvwxyz234567");
    spec.encoding().expect("a base32 alphabet")
});

fn checksum(bytes: &[u8]) -> [u8; 4] {
    blake2b::<4>(&[bytes])
}

/// `text` as a decimal written the one way go-address writes it: digits
/// alone, without a leading zero, at most a signed 64-bit number.
fn canonical_decimal(text: &str) -> Option<u64> {
    if text.is_empty()
        || !text.bytes().all(|byte| byte.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse().ok().filter(|value| *value <= MAX_ID)
}

/// The address a secp256k1 key holds on `chain`.
pub(crate) fn address_from_public_key(
    chain: Chain,
    key: &PublicKey,
) -> Result<String, DerivationError> {
    let network = chain.filecoin_address_network()?;
    let mut bytes = vec![1];
    bytes.extend(blake2b::<20>(&[&key.serialize_uncompressed()]));
    let mut body = bytes[1..].to_vec();
    body.extend(checksum(&bytes));
    Ok(format!("{network}1{}", BASE32.encode(&body)))
}

/// An address on `chain`, checked as go-address checks it and refused where
/// funds sent to it could reach no one: a BLS payload that is not a key, a
/// delegated address outside the Ethereum namespace or of another length
/// than an Ethereum address, and an Ethereum address that only masks an ID.
pub(crate) fn decode_address(chain: Chain, text: &str) -> Result<FilecoinAddress, DerivationError> {
    let invalid = || {
        DerivationError::refused(
            "Not a %@ address.",
            [chain.chain_display_name().to_string()],
        )
    };
    let network = chain.filecoin_address_network()?;
    let mut chars = text.chars();
    if chars.next() != Some(network) {
        return Err(invalid());
    }
    let (protocol, byte) = match chars.next() {
        Some('0') => (FilecoinProtocol::Id, 0u8),
        Some('1') => (FilecoinProtocol::Secp256k1, 1),
        Some('2') => (FilecoinProtocol::Actor, 2),
        Some('3') => (FilecoinProtocol::Bls, 3),
        Some('4') => (FilecoinProtocol::Delegated, 4),
        _ => return Err(invalid()),
    };
    let rest = chars.as_str();
    let mut bytes = vec![byte];
    match protocol {
        FilecoinProtocol::Id => {
            crate::send::leb128::write(canonical_decimal(rest).ok_or_else(invalid)?, &mut bytes);
            return Ok(FilecoinAddress { protocol, bytes });
        }
        FilecoinProtocol::Delegated => {
            let (namespace, body) = rest.split_once('f').ok_or_else(invalid)?;
            let namespace = canonical_decimal(namespace).ok_or_else(invalid)?;
            let decoded = BASE32.decode(body.as_bytes()).map_err(|_| invalid())?;
            let split = decoded.len().checked_sub(4).ok_or_else(invalid)?;
            let (subaddress, sum) = decoded.split_at(split);
            crate::send::leb128::write(namespace, &mut bytes);
            bytes.extend_from_slice(subaddress);
            if checksum(&bytes) != sum {
                return Err(invalid());
            }
            if namespace != EAM_NAMESPACE || subaddress.len() != 20 {
                return Err(DerivationError::refused(
                    "%@ is a delegated address no account can be created at; only Ethereum-style f410f addresses are.",
                    [text],
                ));
            }
            // The Ethereum spelling of an ID address, `0xff`, eleven zero
            // bytes and the ID, is that actor's; no key holds it.
            if subaddress[0] == 0xff && subaddress[1..12].iter().all(|byte| *byte == 0) {
                return Err(DerivationError::refused(
                    "%@ masks an ID address; send to that actor's own address.",
                    [text],
                ));
            }
        }
        FilecoinProtocol::Secp256k1 | FilecoinProtocol::Actor | FilecoinProtocol::Bls => {
            let decoded = BASE32.decode(rest.as_bytes()).map_err(|_| invalid())?;
            let length = if protocol == FilecoinProtocol::Bls {
                48
            } else {
                20
            };
            if decoded.len() != length + 4 {
                return Err(invalid());
            }
            let (payload, sum) = decoded.split_at(length);
            bytes.extend_from_slice(payload);
            if checksum(&bytes) != sum {
                return Err(invalid());
            }
            if protocol == FilecoinProtocol::Bls {
                let point: [u8; 48] = payload.try_into().expect("48 bytes");
                let key = bls12_381::G1Affine::from_compressed(&point);
                if !bool::from(key.is_some()) || bool::from(key.unwrap().is_identity()) {
                    return Err(DerivationError::refused(
                        "%@ is not a BLS public key's address.",
                        [text],
                    ));
                }
            }
        }
    }
    Ok(FilecoinAddress { protocol, bytes })
}

/// The bytes every message CID starts with: CID version 1, the DAG-CBOR
/// codec, and a BLAKE2b-256 multihash of 32 bytes.
const MESSAGE_CID_PREFIX: [u8; 6] = [0x01, 0x71, 0xa0, 0xe4, 0x02, 0x20];

/// The CID of the message serialized as `cbor`, as bytes and as the text
/// Lotus writes: multibase `b`, then lowercase base32.
pub(crate) fn message_cid(cbor: &[u8]) -> (Vec<u8>, String) {
    let mut bytes = MESSAGE_CID_PREFIX.to_vec();
    bytes.extend(blake2b::<32>(&[cbor]));
    let text = format!("b{}", BASE32.encode(&bytes));
    (bytes, text)
}

/// Whether `text` is a message's CID as Lotus writes one.
pub(crate) fn is_message_cid(text: &str) -> bool {
    text.strip_prefix('b')
        .and_then(|body| BASE32.decode(body.as_bytes()).ok())
        .is_some_and(|bytes| bytes.len() == 38 && bytes.starts_with(&MESSAGE_CID_PREFIX))
}

/// Derive a Filecoin account from a seed phrase along a BIP-32 path: the
/// secp256k1 key at the path and its `f1` address, as Glif, Ledger and
/// Lotus derive it.
pub(crate) fn derive_from_seed_phrase(
    chain: Chain,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    wants: Wants,
) -> Result<DerivationResult, DerivationError> {
    let private_key =
        crate::derivation::bip32::phrase_key(seed_phrase, passphrase, derivation_path)?;
    let public_key = PublicKey::from_secret_key(&Secp256k1::new(), &private_key);
    Ok(DerivationResult {
        address: if wants.address {
            Some(address_from_public_key(chain, &public_key)?)
        } else {
            None
        },
        public_key_hex: wants.public_key_hex(&public_key.serialize_uncompressed()),
        private_key_hex: wants.private_key_hex(&private_key.secret_bytes()),
    })
}

#[cfg(test)]
#[path = "tests/filecoin.rs"]
mod tests;
