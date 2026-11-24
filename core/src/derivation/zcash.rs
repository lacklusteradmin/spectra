//! Zcash transparent: t1… P2PKH base58check encoding
//! (2-byte version prefix). Every Zcash address form, shielded ones too, is
//! validated by `validation::address` through librustzcash.

use crate::derivation::bitcoin::{base58check_encode, hash160};

/// A Zcash transparent P2PKH address from its 2-byte version prefix and a
/// compressed public key.
pub(crate) fn zcash_p2pkh_addr(version: [u8; 2], pubkey: &secp256k1::PublicKey) -> String {
    let mut payload = vec![version[0], version[1]];
    payload.extend_from_slice(&hash160(&pubkey.serialize()));
    base58check_encode(&payload)
}
