//! What can go wrong reading and writing the wallet database.

/// A wallet database operation that did not complete.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("wallet_db open {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: rusqlite::Error,
    },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// A stored row no longer decodes into the shape this build writes.
    #[error("{0}")]
    Corrupt(String),
    /// The write was refused: a missing owner row or a reservation conflict.
    #[error("{0}")]
    Invalid(String),
}

impl From<serde_json::Error> for DbError {
    fn from(error: serde_json::Error) -> Self {
        Self::Corrupt(format!("json: {error}"))
    }
}

impl From<DbError> for crate::SpectraBridgeError {
    fn from(error: DbError) -> Self {
        let message = error.to_string();
        match error {
            DbError::Invalid(_) => Self::InvalidInput {
                message: message.into(),
            },
            DbError::Corrupt(_) => Self::Decode { message },
            DbError::Open { .. } | DbError::Sqlite(_) => Self::Failure {
                message: message.into(),
            },
        }
    }
}
