//! What can go wrong building, signing and submitting a transaction.

use crate::api::error::ApiError;
use crate::derivation::error::DerivationError;

/// A send that could not be built, signed or submitted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// A chain service did not give the answer the send needed.
    #[error(transparent)]
    Api(#[from] ApiError),
    /// The signing key or an address could not be derived or decoded.
    #[error(transparent)]
    Derivation(#[from] DerivationError),
    /// The spendable balance does not cover the amount plus the fee. The
    /// message names what fell short where a chain has more than one balance.
    #[error("{0}")]
    InsufficientFunds(String),
    /// The send as requested cannot be built: an amount, address, fee or
    /// reviewed plan that is out of range or no longer matches.
    #[error("{0}")]
    Invalid(String),
    /// Building or signing failed on a valid request.
    #[error("{0}")]
    Internal(String),
}

impl SendError {
    pub(crate) fn invalid(message: impl std::fmt::Display) -> Self {
        Self::Invalid(message.to_string())
    }

    pub(crate) fn insufficient_funds() -> Self {
        Self::InsufficientFunds("Insufficient funds for the amount plus the network fee.".into())
    }
}

impl From<crate::registry::RegistryError> for SendError {
    fn from(error: crate::registry::RegistryError) -> Self {
        Self::Invalid(error.to_string())
    }
}

impl From<crate::store::seed_envelope::EnvelopeError> for SendError {
    fn from(error: crate::store::seed_envelope::EnvelopeError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<serde_json::Error> for SendError {
    fn from(error: serde_json::Error) -> Self {
        Self::Invalid(format!("json: {error}"))
    }
}

impl From<hex::FromHexError> for SendError {
    fn from(error: hex::FromHexError) -> Self {
        Self::Invalid(format!("hex decode: {error}"))
    }
}

impl From<SendError> for crate::SpectraBridgeError {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Api(error) => error.into(),
            SendError::Derivation(error) => error.into(),
            SendError::InsufficientFunds(_) | SendError::Invalid(_) => Self::InvalidInput {
                message: error.to_string(),
            },
            SendError::Internal(message) => Self::Failure { message },
        }
    }
}
