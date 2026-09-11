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

/// Iterations [`create_verifier`] writes today — the OWASP figure for
/// PBKDF2-HMAC-SHA256.
const DEFAULT_ROUNDS: u32 = 210_000;

/// Weakest verifier [`verify`] will honour.
///
/// Separate from [`DEFAULT_ROUNDS`] on purpose: the default is what we write
/// now and may rise, while this is the floor below which a stored envelope is
/// not worth trusting. Keeping them apart is what lets the cost go up without
/// locking anyone out of a verifier sealed at the old one.
const MINIMUM_ROUNDS: u32 = 210_000;

/// Most work a stored envelope may ask for.
///
/// `rounds` is read back out of storage, and PBKDF2 does exactly as many
/// iterations as it is told. Unbounded, an envelope edited to `u32::MAX` makes
/// unlocking run for hours on the main thread — the app simply never comes
/// back. At twenty times the default this is still a fraction of a second on a
/// phone.
const MAXIMUM_ROUNDS: u32 = DEFAULT_ROUNDS * 20;

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

    // Everything below comes out of storage, so none of it is trusted to be
    // what `create_verifier` wrote. The version was checked and the work
    // factor was not: PBKDF2 runs exactly the iterations it is handed, so an
    // envelope edited to `u32::MAX` hangs the unlock, and one edited to `1`
    // turns the verifier into a single HMAC an attacker can precompute. A
    // short salt is the same weakening by another field.
    if !(MINIMUM_ROUNDS..=MAXIMUM_ROUNDS).contains(&envelope.rounds)
        || envelope.salt.len() != SALT_LENGTH
        || envelope.digest.len() != DERIVED_KEY_LENGTH
    {
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

/// The envelope is read back out of storage, so every field in it is input.
#[cfg(test)]
mod a_stored_envelope_is_not_trusted {
    use super::*;

    /// Rebuild a verifier with one field replaced, the way an edited keychain
    /// entry or a corrupted blob would present it.
    fn tampered(password: &str, edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let verifier = create_verifier(password).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&verifier).unwrap();
        edit(&mut value);
        serde_json::to_vec(&value).unwrap()
    }

    /// A work factor below the floor is a downgrade: at one iteration the
    /// verifier is a single HMAC, which is precomputable. The right password
    /// does not rescue it — the envelope itself is refused.
    #[test]
    fn a_downgraded_work_factor_is_refused_even_with_the_right_password() {
        for rounds in [0u32, 1, 1_000, MINIMUM_ROUNDS - 1] {
            let data = tampered("correct horse battery staple", |value| {
                value["rounds"] = serde_json::json!(rounds);
            });
            assert!(
                !verify("correct horse battery staple", &data),
                "rounds={rounds}"
            );
        }
    }

    /// The ceiling is a liveness bound, not a security one: PBKDF2 does as
    /// many iterations as it is told, on the thread that asked. Without this,
    /// unlocking an envelope edited to `u32::MAX` never returns.
    #[test]
    fn an_absurd_work_factor_is_refused_rather_than_run() {
        for rounds in [MAXIMUM_ROUNDS + 1, 100_000_000, u32::MAX] {
            let data = tampered("correct horse battery staple", |value| {
                value["rounds"] = serde_json::json!(rounds);
            });
            let start = std::time::Instant::now();
            assert!(!verify("correct horse battery staple", &data));
            assert!(
                start.elapsed() < std::time::Duration::from_secs(1),
                "rounds={rounds} was run rather than refused"
            );
        }
    }

    /// A salt or digest of the wrong length is the same weakening by another
    /// field — an empty salt makes the digest a plain password hash.
    #[test]
    fn a_salt_or_digest_of_the_wrong_size_is_refused() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        for field in ["salt", "digest"] {
            for bytes in [vec![], vec![0u8; 4], vec![0u8; 64]] {
                let data = tampered("correct horse battery staple", |value| {
                    value[field] = serde_json::json!(STANDARD.encode(&bytes));
                });
                assert!(
                    !verify("correct horse battery staple", &data),
                    "{field} of {} bytes",
                    bytes.len()
                );
            }
        }
    }

    /// What `create_verifier` writes still verifies — the bounds admit the
    /// real thing.
    #[test]
    fn the_envelope_we_write_is_inside_its_own_bounds() {
        assert!((MINIMUM_ROUNDS..=MAXIMUM_ROUNDS).contains(&DEFAULT_ROUNDS));
        let verifier = create_verifier("correct horse battery staple").unwrap();
        assert!(verify("correct horse battery staple", &verifier));
        assert!(!verify("wrong password", &verifier));
    }
}
