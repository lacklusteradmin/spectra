//! Substrate derivation paths: hard (`//name`) and soft (`/name`) junctions
//! under a phrase's sr25519 root key, read as subkey (sp-core) and
//! polkadot.js read the path of a secret URI.
//!
//! A junction's chain code is its number as a little-endian `u64`, or else
//! the SCALE encoding of its text (compact length, then UTF-8), each padded
//! to 32 bytes, or hashed with BLAKE2b-256 when longer. Text the two
//! implementations read differently is refused rather than read one way: a
//! number past `u64`, which polkadot.js reads as a 256-bit number and sp-core
//! as text, and `0x` hex, which polkadot.js reads as bytes. A secret URI's
//! `///password` is the derivation passphrase, entered as one.

use zeroize::Zeroizing;

use crate::derivation::error::DerivationError;

/// One step of a path, with its 32-byte chain code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Junction {
    Hard([u8; 32]),
    Soft([u8; 32]),
}

/// The junctions `path` names, in order; none for an empty path, which is
/// the root key.
pub(crate) fn parse(path: &str) -> Result<Vec<Junction>, DerivationError> {
    let path = path.trim();
    let not_a_path = || DerivationError::refused("Not a Substrate derivation path: %@", [path]);
    let mut junctions = Vec::new();
    let mut rest = path;
    while !rest.is_empty() {
        let after = rest.strip_prefix('/').ok_or_else(not_a_path)?;
        let (hard, code_and_rest) = match after.strip_prefix('/') {
            Some(after) => (true, after),
            None => (false, after),
        };
        if code_and_rest.starts_with('/') {
            return Err(DerivationError::refused(
                "A Substrate path takes no ///password: %@. Enter the password as the passphrase.",
                [path],
            ));
        }
        let end = code_and_rest.find('/').unwrap_or(code_and_rest.len());
        let code = &code_and_rest[..end];
        if code.is_empty() {
            return Err(not_a_path());
        }
        let chain_code = chain_code(code)?;
        junctions.push(if hard {
            Junction::Hard(chain_code)
        } else {
            Junction::Soft(chain_code)
        });
        rest = &code_and_rest[end..];
    }
    Ok(junctions)
}

/// The path as an import stores it: trimmed, and `None` for the root key.
/// Refuses one that does not parse.
pub(crate) fn normalized(path: &str) -> Result<Option<String>, DerivationError> {
    let path = path.trim();
    parse(path)?;
    Ok((!path.is_empty()).then(|| path.to_string()))
}

/// Whether the key `path` derives is a seed a raw-key import reads back as
/// the same account: every junction hard, since a soft one leaves no seed,
/// and, under a root expanded in Uniform mode, at least one, since the root
/// seed itself expands to another key in the Ed25519 mode an import uses.
pub(crate) fn derives_importable_seed(path: &str, uniform_root: bool) -> bool {
    parse(path).is_ok_and(|junctions| {
        junctions
            .iter()
            .all(|junction| matches!(junction, Junction::Hard(_)))
            && !(uniform_root && junctions.is_empty())
    })
}

fn chain_code(code: &str) -> Result<[u8; 32], DerivationError> {
    let mut chain_code = [0u8; 32];
    if code.bytes().all(|byte| byte.is_ascii_digit()) {
        let number: u64 = code.parse().map_err(|_| {
            DerivationError::refused("A Substrate junction number must fit 64 bits: %@", [code])
        })?;
        chain_code[..8].copy_from_slice(&number.to_le_bytes());
        return Ok(chain_code);
    }
    // polkadot.js's `isHex`: `0x`, then hex digits, an even count in all.
    if code.strip_prefix("0x").is_some_and(|digits| {
        digits.bytes().all(|b| b.is_ascii_hexdigit()) && code.len().is_multiple_of(2)
    }) {
        return Err(DerivationError::refused(
            "Substrate wallets read a 0x junction differently: %@",
            [code],
        ));
    }
    let encoded = parity_scale_codec::Encode::encode(code);
    if encoded.len() > chain_code.len() {
        use blake2::Digest as _;
        chain_code = blake2::Blake2b::<blake2::digest::consts::U32>::digest(&encoded).into();
    } else {
        chain_code[..encoded.len()].copy_from_slice(&encoded);
    }
    Ok(chain_code)
}

/// The key `junctions` derive from the root `seed` expanded in `mode`, and
/// its own seed when every junction is hard: sp-core's "secret seed", which
/// expands to the key in Ed25519 mode and which a raw-key import reads. A
/// soft junction derives a key that no seed expands to.
pub(crate) fn derive(
    seed: &[u8; 32],
    mode: schnorrkel::ExpansionMode,
    junctions: &[Junction],
) -> Result<(schnorrkel::SecretKey, Option<Zeroizing<[u8; 32]>>), DerivationError> {
    let root = schnorrkel::MiniSecretKey::from_bytes(seed)
        .map_err(|e| DerivationError::Internal(format!("Invalid sr25519 mini-secret: {e}")))?;
    let mut secret = root.expand(mode);
    let mut derived_seed = Some(Zeroizing::new(*seed));
    for junction in junctions {
        match junction {
            Junction::Hard(code) => {
                let (mini, _) = secret
                    .hard_derive_mini_secret_key(Some(schnorrkel::derive::ChainCode(*code)), b"");
                secret = mini.expand(schnorrkel::ExpansionMode::Ed25519);
                derived_seed = derived_seed.map(|_| Zeroizing::new(mini.to_bytes()));
            }
            Junction::Soft(code) => {
                use schnorrkel::derive::Derivation as _;
                (secret, _) = secret.derived_key_simple(schnorrkel::derive::ChainCode(*code), b"");
                derived_seed = None;
            }
        }
    }
    Ok((secret, derived_seed))
}

/// The keypair a Substrate signing key holds, checked against the account
/// it signs for. The key is a 32-byte seed, expanded the way that gives
/// `public` (Ed25519 mode, as every Substrate wallet does, or the Uniform
/// mode a power-user override asks for), or a soft junction's 64-byte
/// expanded secret key.
pub(crate) fn signing_keypair(
    key: &[u8],
    public: &[u8; 32],
) -> Result<schnorrkel::Keypair, DerivationError> {
    let pair = match key.len() {
        32 => {
            let mini = schnorrkel::MiniSecretKey::from_bytes(key)
                .map_err(|e| DerivationError::invalid(format!("sr25519 key: {e}")))?;
            [
                schnorrkel::ExpansionMode::Ed25519,
                schnorrkel::ExpansionMode::Uniform,
            ]
            .into_iter()
            .map(|mode| mini.expand_to_keypair(mode))
            .find(|pair| pair.public.to_bytes() == *public)
        }
        64 => {
            let pair = schnorrkel::SecretKey::from_bytes(key)
                .map_err(|e| DerivationError::invalid(format!("sr25519 key: {e}")))?
                .to_keypair();
            (pair.public.to_bytes() == *public).then_some(pair)
        }
        _ => None,
    };
    pair.ok_or_else(|| DerivationError::invalid("sr25519 key does not match the account"))
}

#[cfg(test)]
#[path = "tests/substrate_path.rs"]
mod tests;
