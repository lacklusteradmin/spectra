//! What can go wrong asking a chain service, by who is at fault.

/// A request to a chain service that did not produce a usable answer.
///
/// The variant says what a caller can do about it: `Transport` and `Status`
/// may succeed on another endpoint or later, `Decode` means the service is not
/// speaking the contract this adapter implements, `Rejected` is the service's
/// considered answer, and `InvalidInput` never left the device.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// No answer: connection, TLS, proxy, timeout, the kill switch, or every
    /// retry spent on 429 and 5xx.
    #[error("{0}")]
    Transport(String),
    /// The service answered with a non-success status.
    #[error("HTTP {status}: {body}")]
    Status { status: u16, body: String },
    /// The answer did not have the shape this adapter expects.
    #[error("{0}")]
    Decode(String),
    /// The service understood the request and refused it: an RPC error
    /// object, a rejected broadcast, an unknown account.
    #[error("{0}")]
    Rejected(String),
    /// Nothing to ask: no endpoint serves this request.
    #[error("no endpoints configured")]
    NoEndpoint,
    /// The request cannot be formed from what the caller supplied.
    #[error("{0}")]
    InvalidInput(String),
}

impl ApiError {
    pub(crate) fn decode(message: impl std::fmt::Display) -> Self {
        Self::Decode(message.to_string())
    }

    pub(crate) fn rejected(message: impl std::fmt::Display) -> Self {
        Self::Rejected(message.to_string())
    }

    pub(crate) fn invalid(message: impl std::fmt::Display) -> Self {
        Self::InvalidInput(message.to_string())
    }

    /// Whether another endpoint, or the same one later, might answer.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Transport(_) => true,
            Self::Status { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(format!("json decode: {error}"))
    }
}

impl From<hex::FromHexError> for ApiError {
    fn from(error: hex::FromHexError) -> Self {
        Self::Decode(format!("hex decode: {error}"))
    }
}

/// A field the answer should have carried and did not.
pub(crate) trait OrDecode<T> {
    fn or_decode(self, what: &str) -> Result<T, ApiError>;
}

impl<T> OrDecode<T> for Option<T> {
    fn or_decode(self, what: &str) -> Result<T, ApiError> {
        self.ok_or_else(|| ApiError::Decode(what.to_string()))
    }
}

impl From<ApiError> for crate::SpectraBridgeError {
    fn from(error: ApiError) -> Self {
        let message = error.to_string();
        match error {
            ApiError::Transport(_) | ApiError::NoEndpoint => Self::Network { message },
            ApiError::Status { .. } if error.is_transient() => Self::Network { message },
            ApiError::Decode(_) => Self::Decode { message },
            ApiError::InvalidInput(_) => Self::InvalidInput {
                message: message.into(),
            },
            ApiError::Status { .. } | ApiError::Rejected(_) => Self::Failure {
                message: message.into(),
            },
        }
    }
}
