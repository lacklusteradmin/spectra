//! Wallet secrets: the `SecretStore` the platform implements, its native file
//! and in-memory backends, the device key and envelopes that seal what it
//! holds, and the password stretching that opens them.

pub mod backends;
mod device_key;
pub(crate) mod kdf;
pub mod password_verifier;
pub mod seed_envelope;
pub mod store;
pub mod wallet;
