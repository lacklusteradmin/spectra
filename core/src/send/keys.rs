//! Secret material stays redacted and zeroizing across the internal send boundary.

use crate::send::error::SendError;
use std::fmt;
use zeroize::Zeroizing;

/// A 32-byte Ed25519 seed, never a 64-byte keypair or an expanded scalar.
/// dalek owns and zeroizes its signing material; callers cannot print it.
pub struct Ed25519Seed(ed25519_dalek::SigningKey);
impl Ed25519Seed {
    pub fn from_hex(value: &str) -> Result<Self, SendError> {
        let bytes = Zeroizing::new(
            hex::decode(value)
                .map_err(|_| SendError::Invalid("invalid Ed25519 seed hex".into()))?,
        );
        let seed: &[u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| SendError::Invalid("Ed25519 seed must be 32 bytes".into()))?;
        Ok(Self(ed25519_dalek::SigningKey::from_bytes(seed)))
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn require_public_key(&self, expected: &[u8; 32]) -> Result<(), SendError> {
        if &self.public_key() != expected {
            return Err(SendError::Invalid(
                "sender public key does not match signing seed".into(),
            ));
        }
        Ok(())
    }
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        self.0.sign(message).to_bytes()
    }
}
impl fmt::Debug for Ed25519Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ed25519Seed([REDACTED])")
    }
}

/// A secp256k1 scalar, validated, signing the SHA-256 of a message as 64
/// bytes `r || s` with an RFC 6979 nonce and a low S: the ECDSA signature
/// the Internet Computer verifies for a secp256k1 sender.
pub struct Secp256k1Key(secp256k1::SecretKey);
impl Secp256k1Key {
    pub fn from_hex(value: &str) -> Result<Self, SendError> {
        let bytes = Zeroizing::new(
            hex::decode(value)
                .map_err(|_| SendError::Invalid("invalid secp256k1 key hex".into()))?,
        );
        secp256k1::SecretKey::from_slice(&bytes)
            .map(Self)
            .map_err(|_| SendError::Invalid("invalid secp256k1 private key".into()))
    }
    /// A fresh key from the operating system's randomness.
    pub fn random() -> Self {
        Self(secp256k1::SecretKey::new(&mut rand::rngs::OsRng))
    }
    pub fn public_key(&self) -> secp256k1::PublicKey {
        secp256k1::PublicKey::from_secret_key(secp256k1::SECP256K1, &self.0)
    }
    pub fn sign_sha256(&self, message: &[u8]) -> [u8; 64] {
        use sha2::{Digest, Sha256};
        let digest = secp256k1::Message::from_digest(Sha256::digest(message).into());
        secp256k1::SECP256K1
            .sign_ecdsa(&digest, &self.0)
            .serialize_compact()
    }
}
impl Drop for Secp256k1Key {
    fn drop(&mut self) {
        self.0.non_secure_erase();
    }
}
impl fmt::Debug for Secp256k1Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secp256k1Key([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_redacted_and_keypair_bytes_are_not_seeds() {
        let hex = "01".repeat(32);
        assert!(!format!("{:?}", Ed25519Seed::from_hex(&hex).unwrap()).contains(&hex));
        for len in [0, 31, 33, 64] {
            assert!(Ed25519Seed::from_hex(&"01".repeat(len)).is_err());
        }
        assert!(!format!("{:?}", Secp256k1Key::from_hex(&hex).unwrap()).contains(&hex));
        for invalid in ["00".repeat(32), "ff".repeat(32), "01".repeat(31)] {
            assert!(Secp256k1Key::from_hex(&invalid).is_err());
        }
    }
}
