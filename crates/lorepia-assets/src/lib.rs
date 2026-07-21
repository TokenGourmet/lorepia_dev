#![forbid(unsafe_code)]

mod catalog;
mod error;
mod mime;
mod model;
mod secure_fs;
mod store;

pub use error::{AssetError, Result};
pub use model::{
    AssetHash, AssetLimits, AssetMime, AssetObject, AssetObjectPage, AssetOwner, AssetReference,
    AssetState, AssetStats, BACKUP_SNAPSHOT_LEASE_TIMEOUT_MS, BackupSnapshotCleanup,
    BackupSnapshotLease, CleanupPage, ExportedObject, IngestOutcome, IngestRequest,
    MAX_BACKUP_SNAPSHOT_SESSIONS, MAX_OWNER_ID_BYTES, MAX_OWNER_TYPE_BYTES, MAX_PAGE_SIZE,
    MAX_SOURCE_NAME_BYTES, MAX_TEMPORARY_OWNER_SESSIONS, MarkSweepPage, ReconcileFinding,
    ReconcileFindingKind, ReconcilePage, ShardReconcilePage, TEMPORARY_OWNER_LEASE_TIMEOUT_MS,
};
pub use store::{AssetReader, AssetStore, ShardReconciler};

/// Current on-disk schema version of the asset catalog.
pub const CURRENT_ASSET_SCHEMA_VERSION: i64 = catalog::CURRENT_SCHEMA_VERSION;
