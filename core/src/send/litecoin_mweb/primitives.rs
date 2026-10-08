//! MWEB's primitives, as Litecoin Core's libmw defines them and its blocks
//! exercise them:
//!
//! - hashes are BLAKE3-256, under a one-byte tag where the protocol tags one;
//! - a Pedersen commitment is `blind·G + value·H`, with secp256k1-zkp's
//!   generator `H`, serialized as `0x08` or `0x09` and x — `0x08` when y is
//!   a quadratic residue;
//! - a switch commitment blinds with `blind + SHA-256(C ‖ blind·J)`, with
//!   Grin's generator `J`;
//! - a signature is the Schnorr scheme of secp256k1-zkp's first `schnorrsig`
//!   module: `R` has a quadratic-residue y, `e = SHA-256(R.x ‖ P ‖ m)` with
//!   the 33-byte key `P`, and `s = k + e·x`; the nonce is `SHA-256(x ‖ m)`.
//!
//! `core/tests/fixtures/mweb-mainnet.json` holds the mainnet blocks and the
//! ltcd vector these are checked against.

use std::sync::OnceLock;

use num_bigint::BigUint;
use secp256k1::{PublicKey, SECP256K1, Scalar, SecretKey};
use sha2::{Digest, Sha256};

pub(crate) use crate::api::litecoin_p2p::wire::{Commitment, RANGE_PROOF_SIZE, blake3};
use crate::send::error::SendError;

/// The one-byte tags libmw hashes under.
pub(crate) mod tag {
    pub const ADDRESS: u8 = b'A';
    pub const BLIND: u8 = b'B';
    pub const DERIVE: u8 = b'D';
    pub const NONCE: u8 = b'N';
    pub const OUT_KEY: u8 = b'O';
    pub const SEND_KEY: u8 = b'S';
    pub const TAG: u8 = b'T';
    pub const NONCE_MASK: u8 = b'X';
    pub const VALUE_MASK: u8 = b'Y';
}

/// The value generator `H` of secp256k1-zkp, uncompressed.
const GENERATOR_H: &str = "0450929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac031d3c6863973926e049e637cb1b5f40a36dac28af1766968c30c2313f3a38904";
/// The switch-commitment generator `J` of Grin's secp256k1-zkp.
const GENERATOR_J: &str = "02b860f56795fc03f3c21685383d1b5a2f2954f49b7e398b8d2a0193933621155f";

fn generator(hex: &str, cell: &'static OnceLock<PublicKey>) -> &'static PublicKey {
    cell.get_or_init(|| {
        PublicKey::from_slice(&hex::decode(hex).expect("a generator in hex"))
            .expect("a generator on the curve")
    })
}

fn generator_h() -> &'static PublicKey {
    static H: OnceLock<PublicKey> = OnceLock::new();
    generator(GENERATOR_H, &H)
}

fn generator_j() -> &'static PublicKey {
    static J: OnceLock<PublicKey> = OnceLock::new();
    generator(GENERATOR_J, &J)
}

/// BLAKE3 of the tag byte, then `data`.
pub(crate) fn hashed(tag: u8, data: &[u8]) -> [u8; 32] {
    let mut hasher = ::blake3::Hasher::new();
    hasher.update(&[tag]);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

fn field_prime() -> &'static BigUint {
    static P: OnceLock<BigUint> = OnceLock::new();
    P.get_or_init(|| {
        BigUint::parse_bytes(
            b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F",
            16,
        )
        .expect("the field prime")
    })
}

/// The group order `n`.
pub(crate) fn group_order() -> &'static BigUint {
    static N: OnceLock<BigUint> = OnceLock::new();
    N.get_or_init(|| {
        BigUint::parse_bytes(
            b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
            16,
        )
        .expect("the group order")
    })
}

/// Euler's criterion: `y` is a nonzero square modulo `p`.
fn is_quadratic_residue(y: &BigUint) -> bool {
    let p = field_prime();
    y.modpow(&((p - 1u32) >> 1), p) == BigUint::from(1u32)
}

fn y_of(point: &PublicKey) -> BigUint {
    BigUint::from_bytes_be(&point.serialize_uncompressed()[33..])
}

/// A 32-byte big-endian value below the group order, as a tweak.
pub(crate) fn scalar(bytes: [u8; 32]) -> Result<Scalar, SendError> {
    Scalar::from_be_bytes(bytes).map_err(|_| SendError::Internal("MWEB scalar out of range".into()))
}

/// A 32-byte big-endian value as a nonzero key below the group order.
pub(crate) fn secret(bytes: [u8; 32]) -> Result<SecretKey, SendError> {
    SecretKey::from_slice(&bytes).map_err(|_| SendError::Internal("MWEB key out of range".into()))
}

fn internal(error: impl std::fmt::Display) -> SendError {
    SendError::Internal(format!("MWEB: {error}"))
}

pub(crate) fn mul(point: &PublicKey, by: &[u8; 32]) -> Result<PublicKey, SendError> {
    point.mul_tweak(SECP256K1, &scalar(*by)?).map_err(internal)
}

pub(crate) fn add(a: &PublicKey, b: &PublicKey) -> Result<PublicKey, SendError> {
    a.combine(b).map_err(internal)
}

pub(crate) fn public(key: &SecretKey) -> PublicKey {
    PublicKey::from_secret_key(SECP256K1, key)
}

/// `a + b` modulo the group order.
pub(crate) fn add_secrets(a: &SecretKey, b: &SecretKey) -> Result<SecretKey, SendError> {
    a.add_tweak(&scalar(b.secret_bytes())?).map_err(internal)
}

/// `a · b` modulo the group order.
pub(crate) fn mul_secrets(a: &SecretKey, b: &[u8; 32]) -> Result<SecretKey, SendError> {
    a.mul_tweak(&scalar(*b)?).map_err(internal)
}

/// `a − b` modulo the group order.
pub(crate) fn sub_secrets(a: &SecretKey, b: &SecretKey) -> Result<SecretKey, SendError> {
    a.add_tweak(&scalar(b.negate().secret_bytes())?)
        .map_err(internal)
}

/// `key⁻¹ · point`: what the protocol writes `point / key`.
pub(crate) fn div(point: &PublicKey, key: &[u8; 32]) -> Result<PublicKey, SendError> {
    let n = group_order();
    let k = BigUint::from_bytes_be(key);
    if k == BigUint::default() || &k >= n {
        return Err(SendError::Internal("MWEB scalar out of range".into()));
    }
    let inverse = k.modpow(&(n - 2u32), n).to_bytes_be();
    let mut bytes = [0u8; 32];
    bytes[32 - inverse.len()..].copy_from_slice(&inverse);
    mul(point, &bytes)
}

/// A commitment to `point`: `0x08` when y is a quadratic residue, else
/// `0x09`, then x.
pub(crate) fn commitment_of(point: &PublicKey) -> Commitment {
    let raw = point.serialize_uncompressed();
    let mut out = [0u8; 33];
    out[0] = if is_quadratic_residue(&y_of(point)) {
        0x08
    } else {
        0x09
    };
    out[1..].copy_from_slice(&raw[1..33]);
    Commitment(out)
}

/// The point a commitment names: x, and the y its prefix names — the
/// quadratic-residue root for `0x08`, its negation for `0x09`.
pub(crate) fn commitment_point(commitment: &Commitment) -> Result<PublicKey, SendError> {
    let bytes = &commitment.0;
    if !matches!(bytes[0], 0x08 | 0x09) {
        return Err(SendError::invalid("Not an MWEB commitment"));
    }
    let p = field_prime();
    let x = BigUint::from_bytes_be(&bytes[1..]);
    if &x >= p {
        return Err(SendError::invalid("Not an MWEB commitment"));
    }
    let rhs = (x.modpow(&BigUint::from(3u32), p) + 7u32) % p;
    // p ≡ 3 (mod 4): rhs^((p+1)/4) is the square root that is itself a
    // square, when rhs has one.
    let mut y = rhs.modpow(&((p + 1u32) >> 2), p);
    if (&y * &y) % p != rhs {
        return Err(SendError::invalid("Not an MWEB commitment"));
    }
    if bytes[0] == 0x09 {
        y = p - y;
    }
    let mut raw = [0u8; 65];
    raw[0] = 0x04;
    raw[1..33].copy_from_slice(&bytes[1..]);
    let y = y.to_bytes_be();
    raw[65 - y.len()..].copy_from_slice(&y);
    PublicKey::from_slice(&raw).map_err(|_| SendError::invalid("Not an MWEB commitment"))
}

/// `value·H`, for a nonzero value.
pub(crate) fn value_point(value: u64) -> Result<PublicKey, SendError> {
    let mut v = [0u8; 32];
    v[24..].copy_from_slice(&value.to_be_bytes());
    mul(generator_h(), &v)
}

/// `blind·G + value·H`.
pub(crate) fn commit(blind: &SecretKey, value: u64) -> Result<Commitment, SendError> {
    let blinding = public(blind);
    let point = if value == 0 {
        blinding
    } else {
        add(&blinding, &value_point(value)?)?
    };
    Ok(commitment_of(&point))
}

/// The switch-commitment blinding: `blind + SHA-256(commit(blind, v) ‖ blind·J)`.
pub(crate) fn blind_switch(blind: &SecretKey, value: u64) -> Result<SecretKey, SendError> {
    let mut hash = Sha256::new();
    hash.update(commit(blind, value)?.0);
    hash.update(mul(generator_j(), &blind.secret_bytes())?.serialize());
    add_secrets(&secret(hash.finalize().into())?, blind)
}

/// `commit(blind_switch(blind, value), value)`.
pub(crate) fn switch_commit(blind: &SecretKey, value: u64) -> Result<Commitment, SendError> {
    commit(&blind_switch(blind, value)?, value)
}

fn challenge(r: &[u8], key: &PublicKey, message: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(r);
    hash.update(key.serialize());
    hash.update(message);
    hash.finalize().into()
}

/// Sign `message` with `key`, deterministically.
pub(crate) fn sign(key: &SecretKey, message: &[u8; 32]) -> Result<[u8; 64], SendError> {
    let mut nonce = Sha256::new();
    nonce.update(key.secret_bytes());
    nonce.update(message);
    let mut k = secret(nonce.finalize().into())?;
    let r = public(&k);
    if !is_quadratic_residue(&y_of(&r)) {
        k = k.negate();
    }
    let r_x = &r.serialize_uncompressed()[1..33];
    let e = challenge(r_x, &public(key), message);
    let s = add_secrets(&mul_secrets(key, &e)?, &k)?;
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(r_x);
    signature[32..].copy_from_slice(&s.secret_bytes());
    Ok(signature)
}

/// Whether `signature` signs `message` under `key`.
pub(crate) fn verify(key: &PublicKey, message: &[u8; 32], signature: &[u8; 64]) -> bool {
    let Ok(s) = SecretKey::from_slice(&signature[32..]) else {
        return false;
    };
    let e = challenge(&signature[..32], key, message);
    let Ok(e_key) = mul(key, &e) else {
        return false;
    };
    let Ok(r) = public(&s).combine(&e_key.negate(SECP256K1)) else {
        return false;
    };
    r.serialize_uncompressed()[1..33] == signature[..32] && is_quadratic_residue(&y_of(&r))
}

/// secp256k1-zkp, for its bulletproofs.
fn zkp() -> &'static secp256k1zkp::Secp256k1 {
    static CONTEXT: OnceLock<secp256k1zkp::Secp256k1> = OnceLock::new();
    CONTEXT.get_or_init(|| secp256k1zkp::Secp256k1::with_caps(secp256k1zkp::ContextFlag::Commit))
}

/// A bulletproof that the commitment to `value` under `blind` hides a value
/// below 2^64, committing to `extra` (the output's message).
pub(crate) fn prove_range(
    value: u64,
    blind: &SecretKey,
    extra: &[u8],
) -> Result<[u8; RANGE_PROOF_SIZE], SendError> {
    let context = zkp();
    let key = |bytes: [u8; 32]| {
        secp256k1zkp::SecretKey::from_slice(context, &bytes)
            .map_err(|error| internal(format!("{error:?}")))
    };
    let mut nonces = [[0u8; 32]; 2];
    for nonce in &mut nonces {
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, nonce);
    }
    let proof = context
        .bullet_proof(
            value,
            key(blind.secret_bytes())?,
            key(nonces[0])?,
            key(nonces[1])?,
            Some(extra.to_vec()),
            None,
        )
        .map_err(|error| internal(format!("{error:?}")))?;
    proof.proof[..proof.plen]
        .try_into()
        .map_err(|_| internal("a range proof of an unexpected size"))
}

/// Whether `proof` proves `commitment` hides a value below 2^64, committing
/// to `extra`.
pub(crate) fn verify_range(
    commitment: &Commitment,
    proof: &[u8; RANGE_PROOF_SIZE],
    extra: &[u8],
) -> bool {
    let mut range = secp256k1zkp::pedersen::RangeProof {
        proof: [0; secp256k1zkp::constants::MAX_PROOF_SIZE],
        plen: RANGE_PROOF_SIZE,
    };
    range.proof[..RANGE_PROOF_SIZE].copy_from_slice(proof);
    zkp()
        .verify_bullet_proof(
            secp256k1zkp::pedersen::Commitment(commitment.0),
            range,
            Some(extra.to_vec()),
        )
        .is_ok()
}
