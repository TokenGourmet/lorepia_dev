use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AssetError>;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("asset storage I/O failed")]
    Io(#[source] io::Error),
    #[error("asset catalog operation failed")]
    Database(#[source] rusqlite::Error),
    #[error("invalid {field}: {reason}")]
    InvalidInput {
        field: &'static str,
        reason: &'static str,
    },
    #[error("asset {hash} was not found")]
    NotFound { hash: String },
    #[error("asset {hash} is not active (state: {state})")]
    NotActive { hash: String, state: String },
    #[error("declared MIME {declared} does not match detected MIME {detected}")]
    MimeMismatch { declared: String, detected: String },
    #[error("asset content has no supported, valid magic signature")]
    UnsupportedContent,
    #[error("asset exceeds {limit_name} ({limit} bytes)")]
    LimitExceeded {
        limit_name: &'static str,
        limit: u64,
    },
    #[error("asset operation was cancelled")]
    Cancelled,
    #[error("unsafe filesystem entry at {path}: {reason}")]
    UnsafeFilesystem { path: String, reason: String },
    #[error("asset catalog schema version {found} is unsupported (supported: {supported})")]
    SchemaVersion { found: i64, supported: i64 },
    #[error("asset catalog is incompatible: {reason}")]
    IncompatibleCatalog { reason: &'static str },
    #[error("asset catalog lock was poisoned")]
    LockPoisoned,
    #[error("another asset mutation is already active")]
    MutationBusy,
    #[error("online asset catalog snapshot was cancelled")]
    SnapshotCancelled,
    #[error("asset metadata conflicts with immutable content hash {hash}")]
    HashMetadataConflict { hash: String },
}

impl From<io::Error> for AssetError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for AssetError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}
