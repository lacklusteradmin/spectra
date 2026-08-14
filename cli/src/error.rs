//! Failure, and what the shell learns from it.
//!
//! Exit codes are part of the interface: the acceptance script has to tell
//! "core refused this" from "the network was down", and a single non-zero code
//! cannot say which.

use std::fmt;

/// Process exit codes. `2` is left to clap, which uses it for usage errors.
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
/// Core considered the request and said no — an invalid address, a duplicate
/// contact, a wrong password. Distinct from a failure because the command
/// worked; the answer was no.
pub const EXIT_REJECTED: i32 = 3;

#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub code: i32,
    /// The command already printed its own JSON document.
    ///
    /// `--json` must produce exactly one object. A command that reports a
    /// result *and* fails — self-tests with failures, say — would otherwise
    /// print its results and then have `main` print an error object after it,
    /// which is two documents and parses as neither.
    pub already_emitted: bool,
}

impl CliError {
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_FAILURE,
            already_emitted: false,
        }
    }

    /// A failure whose JSON the command has already printed.
    pub fn reported(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_FAILURE,
            already_emitted: true,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_USAGE,
            already_emitted: false,
        }
    }

    /// Core refused the request.
    pub fn rejected(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_REJECTED,
            already_emitted: false,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<spectra_core::SpectraBridgeError> for CliError {
    fn from(error: spectra_core::SpectraBridgeError) -> Self {
        use spectra_core::SpectraBridgeError as Bridge;
        match error {
            // Bad input is core saying no, not core falling over.
            Bridge::InvalidInput { message } => Self::rejected(message),
            other => Self::failure(other.to_string()),
        }
    }
}

impl From<spectra_core::store::wallet_secrets::WalletSecretError> for CliError {
    fn from(error: spectra_core::store::wallet_secrets::WalletSecretError) -> Self {
        use spectra_core::store::wallet_secrets::WalletSecretError as Secret;
        match error {
            Secret::IncorrectPassword | Secret::NotSealed => Self::rejected(error.to_string()),
            other => Self::failure(other.to_string()),
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::failure(message)
    }
}

pub type CliResult<T> = Result<T, CliError>;
