//! PBKDF2, the one key-stretching function core uses: BIP-39, Cardano and
//! TON phrases, Polyseed, the app password and the password that seals a
//! wallet.
//!
//! It runs through ring's implementation, which is not generic: the code is
//! compiled, optimized, inside ring. `pbkdf2::pbkdf2_hmac::<D>` is generic,
//! so it was compiled into core instead and ran unoptimized in every debug
//! build and test, where sealing a wallet took seconds. The output is the
//! same standard PBKDF2.

use std::num::NonZeroU32;

fn derive(
    algorithm: ring::pbkdf2::Algorithm,
    secret: &[u8],
    salt: &[u8],
    rounds: u32,
    out: &mut [u8],
) {
    // Every caller passes a fixed count or substitutes its default for zero.
    let rounds = NonZeroU32::new(rounds).expect("PBKDF2 runs at least one round");
    ring::pbkdf2::derive(algorithm, rounds, salt, secret, out);
}

/// PBKDF2-HMAC-SHA256 of `secret` and `salt` over `rounds`, filling `out`.
pub(crate) fn pbkdf2_sha256(secret: &[u8], salt: &[u8], rounds: u32, out: &mut [u8]) {
    derive(ring::pbkdf2::PBKDF2_HMAC_SHA256, secret, salt, rounds, out);
}

/// PBKDF2-HMAC-SHA512 of `secret` and `salt` over `rounds`, filling `out`.
pub(crate) fn pbkdf2_sha512(secret: &[u8], salt: &[u8], rounds: u32, out: &mut [u8]) {
    derive(ring::pbkdf2::PBKDF2_HMAC_SHA512, secret, salt, rounds, out);
}

#[cfg(test)]
mod tests {
    /// RFC 6070's SHA-1 vectors have SHA-256 and SHA-512 counterparts; these
    /// are the widely published ones for "password" / "salt".
    #[test]
    fn matches_published_pbkdf2_vectors() {
        let mut out = [0u8; 32];
        super::pbkdf2_sha256(b"password", b"salt", 4096, &mut out);
        assert_eq!(
            hex::encode(out),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
        let mut out = [0u8; 64];
        super::pbkdf2_sha512(b"password", b"salt", 1, &mut out);
        assert_eq!(
            hex::encode(out),
            "867f70cf1ade02cff3752599a3a53dc4af34c7a669815ae5d513554e1c8cf252\
             c02d470a285a0501bad999bfe943c08f050235d7d68b1da55e63f73b60a57fce"
        );
    }
}
