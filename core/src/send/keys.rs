//! Secret material stays redacted and zeroizing across the internal send boundary.
use std::{fmt, ops::Deref};
use zeroize::Zeroizing;

pub struct SecretHex(Zeroizing<String>);
impl From<String> for SecretHex {
    fn from(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
}
impl Deref for SecretHex {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SecretHex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretHex([REDACTED])")
    }
}

/// A 32-byte Ed25519 seed, never a 64-byte keypair or an expanded scalar.
/// dalek owns and zeroizes its signing material; callers cannot print it.
pub struct Ed25519Seed(ed25519_dalek::SigningKey);
impl Ed25519Seed {
    pub fn from_hex(value: &str) -> Result<Self, String> {
        let bytes = Zeroizing::new(hex::decode(value).map_err(|_| "invalid Ed25519 seed hex")?);
        let seed: &[u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| "Ed25519 seed must be 32 bytes")?;
        Ok(Self(ed25519_dalek::SigningKey::from_bytes(seed)))
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn require_public_key(&self, expected: &[u8; 32]) -> Result<(), String> {
        if &self.public_key() != expected {
            return Err("sender public key does not match signing seed".into());
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_redacted_and_keypair_bytes_are_not_seeds() {
        let hex = "01".repeat(32);
        assert!(!format!("{:?}", SecretHex::from(hex.clone())).contains(&hex));
        assert!(!format!("{:?}", Ed25519Seed::from_hex(&hex).unwrap()).contains(&hex));
        for len in [0, 31, 33, 64] {
            assert!(Ed25519Seed::from_hex(&"01".repeat(len)).is_err());
        }
    }
}
