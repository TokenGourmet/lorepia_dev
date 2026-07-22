#![forbid(unsafe_code)]

mod error;
mod export;
mod fsutil;
mod journal;
mod manifest;
mod model;
mod restore;
mod secret;
mod sqlite;

pub use error::{BackupError, Result};
pub use export::{abandon_export, export_backup, partial_path_for};
pub use model::{
    BACKUP_FIXED_MANIFEST_ENTRIES, BACKUP_FORMAT_VERSION, BackupEntry, BackupManifest,
    BackupProgress, CompatibilityReceipt, Control, ExportOptions, ExportReport, ExportRequest,
    FreeSpaceAssessment, FreeSpaceDisposition, FsSpaceProbe, MAX_BACKUP_ASSET_OBJECTS,
    MAX_BACKUP_JOURNAL_BYTES, MAX_BACKUP_MANIFEST_BYTES, MAX_BACKUP_MANIFEST_ENTRIES,
    MAX_BACKUP_MANIFEST_PATH_BYTES, ManifestDatabase, Operation, Phase, RestoreOptions,
    RestorePolicy, RestoreReport, RestoreRequest, SnapshotContract, SpaceProbe, UnknownSpacePolicy,
};
pub use restore::{restore_backup, restore_journal_path_for};
pub use secret::{SecretScanReport, SecretSentinel, scan_paths_for_secrets};
