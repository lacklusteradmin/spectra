//! SEP-53: Ed25519 over SHA-256 of the prefix and the message, in base64.

use super::*;

fn digest(message: &str) -> [u8; 32] {
    Sha256::digest([b"Stellar Signed Message:\n".as_slice(), message.as_bytes()].concat()).into()
}

pub(super) fn sign(seed: &[u8; 32], message: &str) -> String {
    use ed25519_dalek::Signer as _;
    let key = ed25519_dalek::SigningKey::from_bytes(seed);
    base64_encode(&key.sign(&digest(message)).to_bytes())
}

pub(super) fn verify(address: &str, message: &str, signature: &str) -> bool {
    let (Ok(public), Some(signature)) = (
        crate::derivation::stellar::decode_stellar_address(address),
        base64_decode(signature),
    ) else {
        return false;
    };
    ed25519_verify(&public, &digest(message), &signature)
}
