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
/// `Failure` remains as a catch-all for legacy / un-categorised errors and is
/// the target of the blanket `From<String>` / `From<&str>` impls so the 200+
/// existing `.map_err(SpectraBridgeError::from)?` sites keep compiling.
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
    /// Catch-all for legacy errors that haven't been categorised. New code
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

mod app_core;
pub use app_core::*;

pub mod chains;
pub mod derivation;
pub mod diagnostics;
pub mod fetch;
pub mod formatting;
pub mod receive;
pub mod registry;
pub mod send;
pub mod service;
pub mod staking;
pub mod store;
pub mod tokens;
pub mod tor;
pub mod validation;
pub mod wiki;

// Crate-root shortcuts for the heavily-used internal modules. Other paths use
// the folder-qualified `crate::fetch::http`, `crate::store::state`, etc.
pub use derivation::*;
pub use fetch::{history, http, price};
pub use send::ethereum as ethereum_send;
pub use send::preview_types as wallet_core;
pub use store::{state, wallet_db};
