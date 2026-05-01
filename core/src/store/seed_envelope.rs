//! AES-256-GCM envelope encryption for seed phrases.
//!
//! Produces a JSON envelope compatible with the Swift `SeedMaterialEnvelope`
//! format: `{"version":1,"ciphertext":"<base64>","nonce":"<base64>"}`.
//! The `ciphertext` field is AES-GCM ciphertext + 16-byte tag concatenated,
//! matching CryptoKit's `SealedBox.ciphertext + SealedBox.tag` layout.

#![allow(deprecated)] // from_slice is correct for aes-gcm 0.10; warning comes from generic-array version conflict with curve25519-dalek

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// On-disk envelope — field names and base64 encoding match the Swift
/// `SeedMaterialEnvelope.Envelope` struct exactly so existing keychain
/// data can be decrypted transparently.
#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u32,
    #[serde(with = "base64_serde")]
    ciphertext: Vec<u8>,
    #[serde(with = "base64_serde")]
    nonce: Vec<u8>,
}

/// Encrypt `plaintext` with AES-256-GCM using `master_key` (must be 32 bytes).
/// Returns JSON bytes matching the Swift envelope format.
pub fn encrypt(plaintext: &[u8], master_key: &[u8]) -> Result<Vec<u8>, String> {
    if master_key.len() != 32 {
        return Err("master key must be 32 bytes".into());
    }
    let key = Key::<Aes256Gcm>::from_slice(master_key);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // aes-gcm encrypt() returns ciphertext‖tag, same layout as CryptoKit.
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("AES-GCM encrypt failed: {e}"))?;

    let envelope = Envelope {
        version: 1,
        ciphertext,
        nonce: nonce_bytes.to_vec(),
    };

    serde_json::to_vec(&envelope).map_err(|e| format!("JSON encode failed: {e}"))
}

/// Decrypt an envelope produced by [`encrypt`] (or by Swift's
/// `SeedMaterialEnvelope.encode`). Returns the plaintext seed phrase.
pub fn decrypt(data: &[u8], master_key: &[u8]) -> Result<String, String> {
    if master_key.len() != 32 {
        return Err("master key must be 32 bytes".into());
    }
    let envelope: Envelope =
        serde_json::from_slice(data).map_err(|e| format!("JSON decode failed: {e}"))?;
    if envelope.version != 1 {
        return Err(format!("unsupported envelope version: {}", envelope.version));
    }
    if envelope.nonce.len() != 12 {
        return Err("invalid nonce length".into());
    }
    if envelope.ciphertext.len() < 16 {
        return Err("ciphertext too short (must include 16-byte tag)".into());
    }

    let key = Key::<Aes256Gcm>::from_slice(master_key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&envelope.nonce);

    let mut plaintext = cipher
        .decrypt(nonce, envelope.ciphertext.as_ref())
        .map_err(|_| "AES-GCM decrypt failed (bad key or corrupted data)".to_string())?;

    let result = String::from_utf8(plaintext.clone())
        .map_err(|_| "decrypted data is not valid UTF-8".to_string());
    plaintext.zeroize();
    result
}

/// Serde helper — encodes `Vec<u8>` as standard base64, matching Swift's
/// `JSONEncoder` treatment of `Data`.
pub(crate) mod base64_serde {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(data: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        STANDARD.encode(data).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
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

// ── FFI surface (relocated from ffi.rs) ──────────────────────────────────

/// Encrypt a seed phrase with AES-256-GCM. `master_key_bytes` must be exactly
/// 32 bytes. Returns the JSON envelope as `Data` (compatible with Swift's
/// existing `SeedMaterialEnvelope` keychain format).
#[uniffi::export]
pub fn encrypt_seed_envelope(
    plaintext: String,
    master_key_bytes: Vec<u8>,
) -> Result<Vec<u8>, crate::SpectraBridgeError> {
    encrypt(plaintext.as_bytes(), &master_key_bytes).map_err(crate::SpectraBridgeError::from)
}

/// Decrypt a seed envelope produced by [`encrypt_seed_envelope`] or by Swift's
/// `SeedMaterialEnvelope.encode`. Returns the plaintext seed phrase.
#[uniffi::export]
pub fn decrypt_seed_envelope(
    data: Vec<u8>,
    master_key_bytes: Vec<u8>,
) -> Result<String, crate::SpectraBridgeError> {
    decrypt(&data, &master_key_bytes).map_err(crate::SpectraBridgeError::from)
}
