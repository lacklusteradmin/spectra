//! PBKDF2-HMAC-SHA256 password verifier for seed-phrase passwords.
//!
//! Produces a JSON envelope compatible with the Swift
//! `SecureSeedPasswordStore.PasswordVerifierEnvelope` format:
//! `{"version":1,"salt":"<base64>","rounds":210000,"digest":"<base64>"}`.

use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroize;

const CURRENT_VERSION: u32 = 1;
const DEFAULT_ROUNDS: u32 = 210_000;
const DERIVED_KEY_LENGTH: usize = 32;
const SALT_LENGTH: usize = 16;

/// On-disk verifier envelope — field names and base64 encoding match the
/// Swift `SecureSeedPasswordStore.PasswordVerifierEnvelope` exactly.
#[derive(Serialize, Deserialize)]
struct PasswordVerifierEnvelope {
    version: u32,
    #[serde(with = "super::seed_envelope::base64_serde")]
    salt: Vec<u8>,
    rounds: u32,
    #[serde(with = "super::seed_envelope::base64_serde")]
    digest: Vec<u8>,
}

/// Create a PBKDF2-HMAC-SHA256 verifier for `password`.
/// Returns JSON bytes compatible with Swift's format.
pub fn create_verifier(password: &str) -> Result<Vec<u8>, String> {
    let normalized = password.trim();
    if normalized.is_empty() {
        return Err("empty password".into());
    }

    let mut salt = vec![0u8; SALT_LENGTH];
    rand::thread_rng().fill_bytes(&mut salt);

    let mut digest = vec![0u8; DERIVED_KEY_LENGTH];
    pbkdf2_hmac::<Sha256>(normalized.as_bytes(), &salt, DEFAULT_ROUNDS, &mut digest);

    let envelope = PasswordVerifierEnvelope {
        version: CURRENT_VERSION,
        salt,
        rounds: DEFAULT_ROUNDS,
        digest,
    };

    let result = serde_json::to_vec(&envelope).map_err(|e| format!("JSON encode failed: {e}"));
    // digest is moved into envelope, no separate zeroize needed
    result
}

/// Verify `password` against a verifier envelope produced by
/// [`create_verifier`] (or by Swift's `SecureSeedPasswordStore.save`).
pub fn verify(password: &str, verifier_data: &[u8]) -> bool {
    let normalized = password.trim();
    if normalized.is_empty() {
        return false;
    }

    let envelope: PasswordVerifierEnvelope = match serde_json::from_slice(verifier_data) {
        Ok(e) => e,
        Err(_) => return false,
    };

    if envelope.version != CURRENT_VERSION {
        return false;
    }

    let mut candidate = vec![0u8; DERIVED_KEY_LENGTH];
    pbkdf2_hmac::<Sha256>(
        normalized.as_bytes(),
        &envelope.salt,
        envelope.rounds,
        &mut candidate,
    );

    let result = constant_time_eq(&candidate, &envelope.digest);
    candidate.zeroize();
    result
}

/// Constant-time byte comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_create_and_verify() {
        let password = "correct horse battery staple";
        let verifier = create_verifier(password).unwrap();
        assert!(verify(password, &verifier));
        assert!(!verify("wrong password", &verifier));
    }

    #[test]
    fn whitespace_trimmed() {
        let verifier = create_verifier("  mypassword  ").unwrap();
        assert!(verify("mypassword", &verifier));
        assert!(verify("  mypassword  ", &verifier));
    }

    #[test]
    fn empty_password_rejected() {
        assert!(create_verifier("").is_err());
        assert!(create_verifier("   ").is_err());
        let verifier = create_verifier("test").unwrap();
        assert!(!verify("", &verifier));
        assert!(!verify("   ", &verifier));
    }
}
