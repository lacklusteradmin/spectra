//! What can go wrong deriving keys and addresses.

/// A derivation that produced no key or address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DerivationError {
    /// The mnemonic, key, path or address supplied is not valid for the chain.
    #[error("{0}")]
    Invalid(crate::CoreMessage),
    /// A step failed on valid input: a cryptography library refused, or a
    /// BIP-32 child landed outside the curve order (probability below 2^-127).
    #[error("{0}")]
    Internal(String),
}

impl DerivationError {
    pub(crate) fn invalid(message: impl std::fmt::Display) -> Self {
        Self::Invalid(message.to_string().into())
    }

    /// A refusal a person reads, with its values named.
    pub(crate) fn refused(
        template: &'static str,
        args: impl IntoIterator<Item = impl std::fmt::Display>,
    ) -> Self {
        Self::Invalid(crate::CoreMessage::new(template, args))
    }
}

impl From<crate::registry::RegistryError> for DerivationError {
    fn from(error: crate::registry::RegistryError) -> Self {
        Self::Invalid(error.to_string().into())
    }
}

impl From<DerivationError> for crate::SpectraBridgeError {
    fn from(error: DerivationError) -> Self {
        match error {
            DerivationError::Invalid(message) => Self::InvalidInput { message },
            DerivationError::Internal(message) => Self::Failure {
                message: message.into(),
            },
        }
    }
}

impl From<DerivationError> for crate::api::error::ApiError {
    fn from(error: DerivationError) -> Self {
        Self::InvalidInput(error.to_string())
    }
}
