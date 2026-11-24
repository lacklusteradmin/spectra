//! BIP32-Ed25519 extended keys (Khovratovich and Law): one child step, the
//! public key of an extended key, and an Ed25519 signature made with one.
//!
//! An extended key is `kL ‖ kR`: `kL` is the signing scalar itself, not a
//! seed hashed into one, and `kR` the prefix the signature's nonce is
//! hashed from. Cardano's keys are these, and so are the accounts a BIP-39
//! phrase derives along Algorand's ARC-52.

use zeroize::Zeroizing;

use crate::derivation::error::DerivationError;
use crate::derivation::primitives::hmac_sha512;

/// How many low bits of each step's `ZL` the standard derivation keeps (28
/// bytes): Cardano's.
pub(crate) const KHOVRATOVICH_BITS: u32 = 224;

/// How many it keeps under Peikert's amendment (`g = 9`), which ARC-52 uses:
/// more of each step's entropy, still safe for the eight levels a BIP-44
/// path does not reach.
pub(crate) const PEIKERT_BITS: u32 = 247;

/// The child at `index` of the 96-byte extended key `kL ‖ kR ‖ chain code`,
/// adding `8 · ZL` truncated to its low `kept_bits`.
///
/// ```text
/// hardened (index >= 2^31):
///   Z  = HMAC-SHA512(c, 0x00 ‖ kL ‖ kR ‖ index_le)
///   c' = HMAC-SHA512(c, 0x01 ‖ kL ‖ kR ‖ index_le)[32..64]
/// soft:
///   A  = kL · B
///   Z  = HMAC-SHA512(c, 0x02 ‖ A ‖ index_le)
///   c' = HMAC-SHA512(c, 0x03 ‖ A ‖ index_le)[32..64]
/// kL' = kL + 8 · trunc(ZL, kept_bits)
/// kR' = kR + ZR  (mod 2^256)
/// ```
///
/// A `kL'` of 2^255 or more is refused rather than wrapped, as ARC-52
/// refuses it; neither derivation reaches it along the paths wallets use.
pub(crate) fn derive_child(
    xprv: &[u8; 96],
    index: u32,
    kept_bits: u32,
) -> Result<Zeroizing<[u8; 96]>, DerivationError> {
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar;

    let (kl, kr, cc) = (&xprv[0..32], &xprv[32..64], &xprv[64..96]);
    let index_le = index.to_le_bytes();
    let (z, chain) = if index >= 0x8000_0000 {
        (
            hmac_sha512(cc, &[&[0x00], kl, kr, &index_le])?,
            hmac_sha512(cc, &[&[0x01], kl, kr, &index_le])?,
        )
    } else {
        let mut scalar = Zeroizing::new([0u8; 32]);
        scalar.copy_from_slice(kl);
        let public = (Scalar::from_bytes_mod_order(*scalar) * ED25519_BASEPOINT_POINT)
            .compress()
            .to_bytes();
        (
            hmac_sha512(cc, &[&[0x02], &public, &index_le])?,
            hmac_sha512(cc, &[&[0x03], &public, &index_le])?,
        )
    };
    // ZL's low `kept_bits`, little-endian.
    let mut zl = Zeroizing::new([0u8; 32]);
    zl.copy_from_slice(&z[0..32]);
    for bit in kept_bits..256 {
        zl[(bit / 8) as usize] &= !(1 << (bit % 8));
    }
    let mut child = Zeroizing::new([0u8; 96]);
    // kL + 8 · ZL, one byte at a time: each byte of 8 · ZL is its own
    // byte's low five bits and the previous byte's top three.
    let (mut carry, mut previous) = (0u16, 0u8);
    for i in 0..32 {
        let eight_zl = (zl[i] << 3) | (previous >> 5);
        previous = zl[i];
        let sum = u16::from(kl[i]) + u16::from(eight_zl) + carry;
        child[i] = sum as u8;
        carry = sum >> 8;
    }
    if previous >> 5 != 0 || carry != 0 || child[31] & 0x80 != 0 {
        return Err(DerivationError::invalid(
            "BIP32-Ed25519 child scalar overflows 2^255",
        ));
    }
    let mut carry: u16 = 0;
    for i in 0..32 {
        let sum = u16::from(kr[i]) + u16::from(z[32 + i]) + carry;
        child[32 + i] = sum as u8;
        carry = sum >> 8;
    }
    child[64..96].copy_from_slice(&chain[32..64]);
    Ok(child)
}

/// The public key of the extended key `kL ‖ kR`: `kL · B`. Refused unless
/// `kL` has the shape every BIP32-Ed25519 key keeps, a multiple of eight
/// with bit 254 set and bit 255 clear.
pub(crate) fn public_key(key: &[u8; 64]) -> Result<[u8; 32], DerivationError> {
    use curve25519_dalek::{constants::ED25519_BASEPOINT_POINT, scalar::Scalar};
    let scalar_bytes =
        Zeroizing::new(<[u8; 32]>::try_from(&key[..32]).expect("fixed extended key"));
    if scalar_bytes[0] & 7 != 0 || scalar_bytes[31] & 0xc0 != 0x40 {
        return Err(DerivationError::invalid("Invalid extended Ed25519 scalar"));
    }
    let scalar = Zeroizing::new(Scalar::from_bytes_mod_order(*scalar_bytes));
    Ok((*scalar * ED25519_BASEPOINT_POINT).compress().to_bytes())
}

/// An Ed25519 signature of `message` by the extended key `kL ‖ kR`, whose
/// public key must be `public`: the nonce is SHA-512 of `kR ‖ message`, and
/// `kL` signs as it is, unhashed. Ed25519 verifiers check it as any other.
pub(crate) fn sign(
    key: &[u8; 64],
    public: &[u8; 32],
    message: &[u8],
) -> Result<[u8; 64], DerivationError> {
    use curve25519_dalek::{constants::ED25519_BASEPOINT_POINT, scalar::Scalar};
    use sha2::{Digest, Sha512};
    if public_key(key)? != *public {
        return Err(DerivationError::invalid(
            "The extended key is not the signing public key's",
        ));
    }
    let scalar_bytes =
        Zeroizing::new(<[u8; 32]>::try_from(&key[..32]).expect("fixed extended key"));
    let scalar = Zeroizing::new(Scalar::from_bytes_mod_order(*scalar_bytes));
    let mut nonce_hash = Sha512::new();
    nonce_hash.update(&key[32..]);
    nonce_hash.update(message);
    let nonce_digest = Zeroizing::new(<[u8; 64]>::from(nonce_hash.finalize()));
    let nonce = Zeroizing::new(Scalar::from_bytes_mod_order_wide(&nonce_digest));
    let r = (*nonce * ED25519_BASEPOINT_POINT).compress().to_bytes();
    let mut challenge = Sha512::new();
    challenge.update(r);
    challenge.update(public);
    challenge.update(message);
    let h = Scalar::from_bytes_mod_order_wide(&challenge.finalize().into());
    let s = Zeroizing::new(*nonce + h * *scalar);
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(&r);
    signature[32..].copy_from_slice(&s.to_bytes());
    Ok(signature)
}
