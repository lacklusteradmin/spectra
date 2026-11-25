//! Monero's `SigV2`: wallet2's Schnorr signature with the spend key over a
//! hash bound to the address's keys, `SigV2` and Monero base58 of `c ‖ r`.

use super::*;
use curve25519_dalek::{
    constants::ED25519_BASEPOINT_TABLE, edwards::CompressedEdwardsY, scalar::Scalar,
};
use monero_wallet::address::MoneroAddress;

/// Signed with the spend key (0) or the view key (1).
fn message_hash(message: &str, spend: &[u8; 32], view: &[u8; 32], mode: u8) -> [u8; 32] {
    let mut data = b"MoneroMessageSignature\0".to_vec();
    data.extend(spend);
    data.extend(view);
    data.push(mode);
    crate::send::leb128::write(message.len() as u64, &mut data);
    data.extend(message.as_bytes());
    monero_wallet::primitives::keccak256(&data)
}

fn challenge(hash: &[u8; 32], public: &[u8; 32], commitment: &[u8; 32]) -> Scalar {
    Scalar::from_bytes_mod_order(monero_wallet::primitives::keccak256(
        [hash.as_slice(), public, commitment].concat(),
    ))
}

fn keys(address: &str) -> Option<([u8; 32], [u8; 32])> {
    let address = MoneroAddress::from_str_with_unchecked_network(address).ok()?;
    Some((
        address.spend().compress().to_bytes(),
        address.view().compress().to_bytes(),
    ))
}

pub(super) fn signs(address: &str) -> bool {
    keys(address).is_some()
}

pub(super) fn sign(
    spend_secret: &[u8; 32],
    address: &str,
    message: &str,
) -> Result<String, SendError> {
    let (spend, view) =
        keys(address).ok_or_else(|| SendError::Invalid("Not a Monero address".into()))?;
    let secret = Option::<Scalar>::from(Scalar::from_canonical_bytes(*spend_secret))
        .ok_or_else(|| SendError::Invalid("Not a Monero spend key".into()))?;
    if (&secret * ED25519_BASEPOINT_TABLE).compress().to_bytes() != spend {
        return Err(SendError::Invalid(
            "This key is not the address's spend key".into(),
        ));
    }
    let hash = message_hash(message, &spend, &view, 0);
    let mut wide = zeroize::Zeroizing::new([0u8; 64]);
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, wide.as_mut());
    let nonce = Scalar::from_bytes_mod_order_wide(&wide);
    let commitment = (&nonce * ED25519_BASEPOINT_TABLE).compress().to_bytes();
    let c = challenge(&hash, &spend, &commitment);
    let r = nonce - c * secret;
    let signature = [c.to_bytes(), r.to_bytes()].concat();
    Ok(format!(
        "SigV2{}",
        crate::derivation::monero::monero_base58_encode(&signature)
    ))
}

pub(super) fn verify(address: &str, message: &str, signature: &str) -> bool {
    let (Some((spend, view)), Some(bytes)) = (
        keys(address),
        signature
            .strip_prefix("SigV2")
            .and_then(crate::derivation::monero::monero_base58_decode),
    ) else {
        return false;
    };
    let Ok::<[u8; 64], _>(bytes) = bytes.try_into() else {
        return false;
    };
    let (Some(c), Some(r)) = (
        Option::<Scalar>::from(Scalar::from_canonical_bytes(
            bytes[..32].try_into().unwrap(),
        )),
        Option::<Scalar>::from(Scalar::from_canonical_bytes(
            bytes[32..].try_into().unwrap(),
        )),
    ) else {
        return false;
    };
    // Either key may have signed; the hash names which.
    [(0u8, spend), (1u8, view)]
        .into_iter()
        .any(|(mode, public)| {
            let Some(point) = CompressedEdwardsY(public).decompress() else {
                return false;
            };
            let hash = message_hash(message, &spend, &view, mode);
            let commitment = (&r * ED25519_BASEPOINT_TABLE + c * point)
                .compress()
                .to_bytes();
            challenge(&hash, &public, &commitment) == c
        })
}
