//! The FFI boundary is enforced by attribute, not visibility: only
//! `#[uniffi::export]` and the `uniffi::` derives cross to Swift. `pub` alone
//! is a crate-public Rust API and stays invisible there.
//!
//! Exporting an `impl` block exports **every method in it**.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

uniffi::setup_scaffolding!();

/// A sentence core says to a person, in a form each front end can translate.
///
/// `template` is the English sentence with a `%@` where each of `args` goes,
/// in order. It is also the key a front end looks up in its string tables, so
/// the CLI prints the English and the app shows the reader's language from
/// the same value. An interpolated `format!` string cannot be looked up: a
/// sentence a person should read in their language names its values as `args`.
///
/// Text from a library or a node carries no template of its own; it arrives
/// as a template with no `args` that no table names, and reads as it is.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreMessage {
    pub template: String,
    pub args: Vec<String>,
}

impl CoreMessage {
    /// A sentence with values. Every `%@` in `template` takes one of `args`.
    pub fn new(
        template: &'static str,
        args: impl IntoIterator<Item = impl std::fmt::Display>,
    ) -> Self {
        let args: Vec<String> = args.into_iter().map(|arg| arg.to_string()).collect();
        debug_assert_eq!(
            template.matches("%@").count(),
            args.len(),
            "{template:?} takes one argument per %@"
        );
        Self {
            template: template.to_string(),
            args,
        }
    }
}

impl std::fmt::Display for CoreMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut args = self.args.iter();
        let mut pieces = self.template.split("%@");
        if let Some(first) = pieces.next() {
            f.write_str(first)?;
        }
        for piece in pieces {
            f.write_str(args.next().map(String::as_str).unwrap_or_default())?;
            f.write_str(piece)?;
        }
        Ok(())
    }
}

impl From<String> for CoreMessage {
    fn from(text: String) -> Self {
        Self {
            template: text,
            args: Vec::new(),
        }
    }
}

impl From<&str> for CoreMessage {
    fn from(text: &str) -> Self {
        text.to_string().into()
    }
}

/// Bridge error returned to Swift across UniFFI. Variants describe the broad
/// failure category so Swift can branch on it (e.g. surface a "no internet"
/// banner for `Network`, vs. an inline validation error for `InvalidInput`).
/// Each layer's typed error converts into the variant that fits it; a bare
/// string does not, so a new error picks its category where it is raised.
///
/// `Network` and `Decode` carry transport and parser detail for the log; a
/// front end words those itself. `InvalidInput` and `Failure` carry a
/// [`CoreMessage`] a person reads.
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
    InvalidInput { message: CoreMessage },
    /// Core could not do what was asked: storage, signing, or a state that
    /// changed underneath the request.
    #[error("{message}")]
    Failure { message: CoreMessage },
}

impl SpectraBridgeError {
    /// Core refusing a request: the caller asked for something it cannot have.
    pub fn invalid(message: impl std::fmt::Display) -> Self {
        Self::InvalidInput {
            message: message.to_string().into(),
        }
    }

    /// Core unable to do what was asked of it.
    pub fn failure(message: impl std::fmt::Display) -> Self {
        Self::Failure {
            message: message.to_string().into(),
        }
    }

    /// A refusal a person reads, with its values named.
    pub fn refused(
        template: &'static str,
        args: impl IntoIterator<Item = impl std::fmt::Display>,
    ) -> Self {
        Self::InvalidInput {
            message: CoreMessage::new(template, args),
        }
    }

    /// A failure a person reads, with its values named.
    pub fn failed(
        template: &'static str,
        args: impl IntoIterator<Item = impl std::fmt::Display>,
    ) -> Self {
        Self::Failure {
            message: CoreMessage::new(template, args),
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

#[cfg(test)]
mod core_message_tests {
    use super::*;

    /// The English a CLI prints and the key an app looks up are one value.
    #[test]
    fn a_message_renders_its_values_into_its_template() {
        let message = CoreMessage::new("Insufficient %@ for %@", ["ETH", "the fee"]);
        assert_eq!(message.to_string(), "Insufficient ETH for the fee");
        assert_eq!(message.template, "Insufficient %@ for %@");
        let plain = CoreMessage::from("Wallet not found");
        assert_eq!(plain.to_string(), "Wallet not found");
        assert!(plain.args.is_empty());
    }
}
