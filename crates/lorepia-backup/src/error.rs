use std::{io, path::PathBuf};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, BackupError>;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("backup I/O operation failed")]
    Io(#[source] io::Error),
    #[error("backup SQLite operation failed")]
    Database(#[source] rusqlite::Error),
    #[error("product storage snapshot failed")]
    Storage(#[source] lorepia_storage::StorageError),
    #[error("asset snapshot operation failed")]
    Assets(#[source] lorepia_assets::AssetError),
    #[error("backup JSON is invalid")]
    Json(#[source] serde_json::Error),
    #[error("invalid {field}: {reason}")]
    InvalidInput {
        field: &'static str,
        reason: &'static str,
    },
    #[error("unsafe or non-portable backup path: {path}")]
    UnsafePath { path: String },
    #[error("backup destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("restore target already contains data: {0}")]
    ExistingData(PathBuf),
    #[error("backup operation was cancelled; its verified partial state is resumable")]
    Cancelled,
    #[error("insufficient free space: required {required} bytes, available {available} bytes")]
    InsufficientSpace { required: u64, available: u64 },
    #[error("free space could not be determined and policy is fail-closed")]
    FreeSpaceUnknown,
    #[error("backup format version {found} is newer than supported version {supported}")]
    FutureVersion { found: u32, supported: u32 },
    #[error("backup format version {0} is unsupported")]
    UnsupportedVersion(u32),
    #[error("backup manifest is incompatible: {reason}")]
    InvalidManifest { reason: &'static str },
    #[error("backup entry {path} failed {kind} verification")]
    EntryMismatch { path: String, kind: &'static str },
    #[error("SQLite validation failed for {database}: {reason}")]
    InvalidDatabase {
        database: &'static str,
        reason: &'static str,
    },
    #[error("secret sentinel was found in exported entry {entry}")]
    SecretFound { entry: String },
    #[error("backup progress journal conflicts with this request")]
    JournalConflict,
    #[error("backup snapshot lease expired; the partial export must restart")]
    SnapshotLeaseExpired,
    #[error("backup arithmetic overflowed")]
    SizeOverflow,
    #[error("restore rollback failed after publish validation failure")]
    RollbackFailed,
}

impl From<io::Error> for BackupError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<rusqlite::Error> for BackupError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value)
    }
}

impl From<lorepia_storage::StorageError> for BackupError {
    fn from(value: lorepia_storage::StorageError) -> Self {
        match value {
            lorepia_storage::StorageError::SnapshotCancelled => Self::Cancelled,
            other => Self::Storage(other),
        }
    }
}

impl From<lorepia_assets::AssetError> for BackupError {
    fn from(value: lorepia_assets::AssetError) -> Self {
        match value {
            lorepia_assets::AssetError::SnapshotCancelled
            | lorepia_assets::AssetError::Cancelled => Self::Cancelled,
            other => Self::Assets(other),
        }
    }
}

impl From<serde_json::Error> for BackupError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
