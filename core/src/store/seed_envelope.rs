//! AES-256-GCM envelope encryption for seed phrases and private keys.
//!
//! Core-owned versioned JSON envelope with base64 ciphertext and nonce. A
//! wallet password seals under a key derived from it; the device seal, under
//! [`device_key`](super::device_key).

#![allow(deprecated)] // from_slice is correct for aes-gcm 0.10; warning comes from generic-array version conflict with curve25519-dalek

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Current on-disk seed envelope.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    #[serde(with = "base64_serde")]
    ciphertext: Vec<u8>,
    #[serde(with = "base64_serde")]
    nonce: Vec<u8>,
}

/// An envelope that could not be sealed or opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("master key must be 32 bytes")]
    KeyLength,
    /// The bytes are not an envelope this module writes.
    #[error("{0}")]
    Malformed(String),
    /// AES-GCM refused the ciphertext: the key is wrong or the data changed.
    #[error("AES-GCM decrypt failed (bad key or corrupted data)")]
    Decrypt,
    #[error("AES-GCM encrypt failed: {0}")]
    Encrypt(String),
}

impl From<EnvelopeError> for crate::SpectraBridgeError {
    fn from(error: EnvelopeError) -> Self {
        let message = error.to_string();
        match error {
            EnvelopeError::KeyLength => Self::InvalidInput {
                message: message.into(),
            },
            EnvelopeError::Malformed(_) => Self::Decode { message },
            EnvelopeError::Decrypt | EnvelopeError::Encrypt(_) => Self::Failure {
                message: message.into(),
            },
        }
    }
}

/// Length of the envelope's master key. AES-256, so 32 bytes; the number is
/// the cipher's, not a caller's choice, and both halves check it.
pub const MASTER_KEY_LEN: usize = 32;

/// Encrypt `plaintext` with AES-256-GCM using `master_key` (must be
/// [`MASTER_KEY_LEN`] bytes). Returns JSON bytes matching the envelope format.
pub fn encrypt(plaintext: &[u8], master_key: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    if master_key.len() != MASTER_KEY_LEN {
        return Err(EnvelopeError::KeyLength);
    }
    let key = Key::<Aes256Gcm>::from_slice(master_key);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // aes-gcm encrypt() returns ciphertext‖tag, same layout as CryptoKit.
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| EnvelopeError::Encrypt(e.to_string()))?;

    let envelope = Envelope {
        version: 1,
        ciphertext,
        nonce: nonce_bytes.to_vec(),
    };

    Ok(serde_json::to_vec(&envelope).expect("an envelope of bytes and a version serializes"))
}

/// Decrypt an envelope produced by [`encrypt`]. Returns the plaintext seed phrase.
pub fn decrypt(data: &[u8], master_key: &[u8]) -> Result<String, EnvelopeError> {
    let malformed = |message: String| EnvelopeError::Malformed(message);
    if master_key.len() != MASTER_KEY_LEN {
        return Err(EnvelopeError::KeyLength);
    }
    let envelope: Envelope =
        serde_json::from_slice(data).map_err(|e| malformed(format!("JSON decode failed: {e}")))?;
    if envelope.version != 1 {
        return Err(malformed(format!(
            "unsupported envelope version: {}",
            envelope.version
        )));
    }
    if envelope.nonce.len() != 12 {
        return Err(malformed("invalid nonce length".into()));
    }
    if envelope.ciphertext.len() < 16 {
        return Err(malformed(
            "ciphertext too short (must include 16-byte tag)".into(),
        ));
    }

    let key = Key::<Aes256Gcm>::from_slice(master_key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&envelope.nonce);

    let plaintext = cipher
        .decrypt(nonce, envelope.ciphertext.as_ref())
        .map_err(|_| EnvelopeError::Decrypt)?;

    // Consume `plaintext` directly (no clone). On the error path the raw bytes
    // are still recovered from the error and wiped before returning.
    String::from_utf8(plaintext).map_err(|e| {
        let mut bytes = e.into_bytes();
        bytes.zeroize();
        malformed("decrypted data is not valid UTF-8".into())
    })
}

/// Serde helper — encodes `Vec<u8>` as standard base64, matching Swift's
/// `JSONEncoder` treatment of `Data`.
pub(crate) mod base64_serde {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(data: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        STANDARD.encode(data).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(deserializer)?;
        STANDARD.decode(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_encrypt_decrypt() {
        let key = [0xABu8; 32];
        let plaintext = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let envelope = encrypt(plaintext.as_bytes(), &key).unwrap();
        let decrypted = decrypt(&envelope, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn valid_length_tampering_fails_authentication() {
        let key = [7; 32];
        let sealed = encrypt(b"public test fixture", &key).unwrap();
        let original: Envelope = serde_json::from_slice(&sealed).unwrap();
        // Change message bytes, the appended authentication tag, and the nonce
        // independently. All still pass structural validation.
        for target in [0, original.ciphertext.len() - 1, original.ciphertext.len()] {
            let mut envelope: Envelope = serde_json::from_slice(&sealed).unwrap();
            if target == envelope.ciphertext.len() {
                envelope.nonce[0] ^= 1;
            } else {
                envelope.ciphertext[target] ^= 1;
            }
            let tampered = serde_json::to_vec(&envelope).unwrap();
            assert_eq!(
                decrypt(&tampered, &key).unwrap_err(),
                EnvelopeError::Decrypt
            );
        }
    }

    #[test]
    fn malformed_nonce_is_refused_before_decryption() {
        let sealed = encrypt(b"fixture", &[7; 32]).unwrap();
        let mut envelope: Envelope = serde_json::from_slice(&sealed).unwrap();
        envelope.nonce = vec![0];
        assert_eq!(
            decrypt(&serde_json::to_vec(&envelope).unwrap(), &[7; 32]).unwrap_err(),
            EnvelopeError::Malformed("invalid nonce length".into())
        );
    }

    #[test]
    fn wrong_key_fails() {
        let key = [0xABu8; 32];
        let wrong_key = [0xCDu8; 32];
        let envelope = encrypt(b"secret seed", &key).unwrap();
        assert!(decrypt(&envelope, &wrong_key).is_err());
    }

    #[test]
    fn invalid_key_length_rejected() {
        assert!(encrypt(b"test", &[0u8; 16]).is_err());
        assert!(decrypt(b"{}", &[0u8; 16]).is_err());
    }
}
