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

mod app_core;
pub use app_core::*;

pub mod derivation;
pub mod diagnostics;
pub mod fetch;
pub mod ffi;
pub mod platform;
pub mod receive;
pub mod registry;
pub mod send;
pub mod service;
pub mod staking;
pub mod store;

// Crate-root shortcuts for the heavily-used internal modules. Other paths use
// the folder-qualified `crate::fetch::http`, `crate::store::state`, etc.
pub use derivation::*;
pub use fetch::{history, http, price};
pub use send::ethereum as ethereum_send;
pub use store::{state, wallet_core, wallet_db};
