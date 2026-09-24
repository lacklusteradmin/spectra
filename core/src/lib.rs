//! The FFI boundary is enforced by attribute, not visibility: only
//! `#[uniffi::export]` and the `uniffi::` derives cross to Swift. `pub` alone
//! is a crate-public Rust API and stays invisible there.
//!
//! Exporting an `impl` block exports **every method in it**.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

uniffi::setup_scaffolding!();

/// Bridge error returned to Swift across UniFFI. Variants describe the broad
/// failure category so Swift can branch on it (e.g. surface a "no internet"
/// banner for `Network`, vs. an inline validation error for `InvalidInput`).
/// `Failure` represents uncategorized errors from string-based providers.
/// `From<String>` and `From<&str>` map to that variant.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SpectraBridgeError {
    /// Network / RPC failure — connectivity, timeout, TLS, HTTP non-2xx, etc.
    #[error("{message}")]
    Network { message: String },
    /// Response decoding / parsing failure (malformed JSON, unexpected shape,
    /// hex decode error). Distinct from `Network` so the UI can blame the
    /// provider rather than the connection.
    #[error("{message}")]
    Decode { message: String },
    /// Bad caller input — empty seed phrase, invalid address, unsupported
    /// chain ID, etc. UI surfaces these inline against the offending field.
    #[error("{message}")]
    InvalidInput { message: String },
    /// Catch-all for errors without a more specific category. New code
    /// should prefer the specific variants above.
    #[error("{message}")]
    Failure { message: String },
}

impl From<String> for SpectraBridgeError {
    fn from(message: String) -> Self {
        Self::Failure { message }
    }
}

impl From<&str> for SpectraBridgeError {
    fn from(message: &str) -> Self {
        Self::Failure {
            message: message.to_string(),
        }
    }
}

impl From<serde_json::Error> for SpectraBridgeError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode {
            message: error.to_string(),
        }
    }
}

impl From<hex::FromHexError> for SpectraBridgeError {
    fn from(error: hex::FromHexError) -> Self {
        Self::Decode {
            message: error.to_string(),
        }
    }
}

impl From<reqwest::Error> for SpectraBridgeError {
    fn from(error: reqwest::Error) -> Self {
        // Network / TLS / DNS / timeout problems route to `Network` so the
        // UI can branch on them; everything else (notably body-decode
        // failures from `Response::json()`) lands in `Decode`. Without this
        // routing, every reqwest error fell into `Failure` and Swift had no
        // structured way to render "no internet" vs "provider returned bad
        // shape" — even though the underlying source already had the
        // distinction.
        let message = error.to_string();
        if error.is_decode() {
            Self::Decode { message }
        } else {
            Self::Network { message }
        }
    }
}

mod endpoint_api;
pub use endpoint_api::{endpoint_capability_options, EndpointApi};

mod app_core;
pub use app_core::*;

pub mod chains;
pub mod decimal;
pub mod derivation;
pub mod diagnostics;
pub mod fetch;
pub mod formatting;
pub mod registry;
pub mod send;
pub mod service;
pub mod staking;
pub mod store;
pub mod tokens;
pub mod tor;
pub mod validation;
pub mod wallet_db;
pub mod wiki;

#[cfg(test)]
mod app_boundary_tests;
