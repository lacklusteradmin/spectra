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
/// Each layer's typed error converts into the variant that fits it; a bare
/// string does not, so a new error picks its category where it is raised.
#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
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
    /// Core could not do what was asked: storage, signing, or a state that
    /// changed underneath the request.
    #[error("{message}")]
    Failure { message: String },
}

impl SpectraBridgeError {
    /// Core refusing a request: the caller asked for something it cannot have.
    pub fn invalid(message: impl std::fmt::Display) -> Self {
        Self::InvalidInput {
            message: message.to_string(),
        }
    }

    /// Core unable to do what was asked of it.
    pub fn failure(message: impl std::fmt::Display) -> Self {
        Self::Failure {
            message: message.to_string(),
        }
    }
}

/// A background task that panicked or was cancelled before it answered.
impl From<tokio::task::JoinError> for SpectraBridgeError {
    fn from(error: tokio::task::JoinError) -> Self {
        Self::failure(error)
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
        // failures from `Response::json()`) lands in `Decode`.
        let message = error.to_string();
        if error.is_decode() {
            Self::Decode { message }
        } else {
            Self::Network { message }
        }
    }
}

mod endpoint_api;
pub use endpoint_api::{
    Endpoint, EndpointApi, EndpointCapability, endpoint_capability_id, endpoint_capability_options,
};

pub mod endpoints;

mod donations;
pub use donations::{DonationDestination, donation_destinations};

mod explorers;
pub use explorers::{
    TransactionExplorer, TransactionExplorerLink, transaction_explorer_link, transaction_explorers,
};

pub mod api;
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
mod worker;

#[cfg(test)]
#[path = "tests/app_boundary.rs"]
mod app_boundary_tests;
