//! Exit codes are part of the interface: a script has to tell "core refused
//! this" from "the network was down".

use std::fmt;

/// Process exit codes. `2` is left to clap, which uses it for usage errors.
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
/// Core considered the request and said no. Distinct from a failure: the
/// command worked, the answer was no.
pub const EXIT_REJECTED: i32 = 3;

#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub code: i32,
}

impl CliError {
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_FAILURE,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_USAGE,
        }
    }

    /// Core refused the request.
    pub fn rejected(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: EXIT_REJECTED,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<spectra_core::error::SpectraBridgeError> for CliError {
    fn from(error: spectra_core::error::SpectraBridgeError) -> Self {
        use spectra_core::error::SpectraBridgeError as Bridge;
        match error {
            // Bad input is core saying no, not core falling over.
            Bridge::InvalidInput { message } => Self::rejected(message.to_string()),
            Bridge::StoreUnreadable { message } => Self::failure(format!(
                "this build cannot read the stored data ({message}); \
                 `spectra settings discard --yes` deletes it and starts over"
            )),
            other => Self::failure(other.to_string()),
        }
    }
}

/// Core's layer errors classify themselves on the way to the bridge; the CLI
/// reads the same classification.
macro_rules! via_bridge {
    ($($error:ty),* $(,)?) => {$(
        impl From<$error> for CliError {
            fn from(error: $error) -> Self {
                spectra_core::error::SpectraBridgeError::from(error).into()
            }
        }
    )*};
}

via_bridge!(
    spectra_core::api::error::ApiError,
    spectra_core::derivation::error::DerivationError,
    spectra_core::send::error::SendError,
    spectra_core::wallet_db::error::DbError,
    spectra_core::registry::RegistryError,
);

impl From<spectra_core::secrets::wallet::WalletSecretError> for CliError {
    fn from(error: spectra_core::secrets::wallet::WalletSecretError) -> Self {
        use spectra_core::secrets::wallet::WalletSecretError as Secret;
        match error {
            Secret::IncorrectPassword | Secret::NotSealed | Secret::EmptyPassword => {
                Self::rejected(error.to_string())
            }
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
