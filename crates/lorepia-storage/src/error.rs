use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage path is unavailable")]
    PathUnavailable(#[source] io::Error),
    #[error("storage database operation failed")]
    Database(#[source] rusqlite::Error),
    #[error("storage schema version {found} is newer than supported version {supported}")]
    FutureSchema { found: i64, supported: i64 },
    #[error("storage schema is incompatible: {reason}")]
    IncompatibleSchema { reason: &'static str },
    #[error("invalid {field}: {reason}")]
    InvalidInput {
        field: &'static str,
        reason: &'static str,
    },
    #[error("{entity} was not found")]
    NotFound { entity: &'static str },
    #[error("{entity} already exists or conflicts with active state")]
    Conflict { entity: &'static str },
    #[error("request is {actual}; expected {expected}")]
    InvalidState {
        expected: &'static str,
        actual: String,
    },
    #[error("request sequence mismatch: expected {expected}, found {actual}")]
    SequenceMismatch { expected: u64, actual: u64 },
    #[error("system clock is before the Unix epoch")]
    ClockBeforeEpoch,
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}
