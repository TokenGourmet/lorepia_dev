use std::{io, path::Path};

use lorepia_assets::AssetStore;
use lorepia_storage::Store;
use serde::{Deserialize, Serialize};

use crate::SecretSentinel;

pub const BACKUP_FORMAT_VERSION: u32 = 1;
/// Maximum canonical `manifest.json` size accepted or emitted by this format implementation.
pub const MAX_BACKUP_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
/// Includes the two databases and two product receipts, leaving room for 100,000 asset objects.
pub const MAX_BACKUP_MANIFEST_ENTRIES: usize = 100_004;
/// The two databases and two receipts are mandatory in every v1 package.
pub const BACKUP_FIXED_MANIFEST_ENTRIES: usize = 4;
/// Maximum number of asset objects representable without displacing mandatory entries.
pub const MAX_BACKUP_ASSET_OBJECTS: usize =
    MAX_BACKUP_MANIFEST_ENTRIES - BACKUP_FIXED_MANIFEST_ENTRIES;
/// Maximum sum of UTF-8 path bytes across all manifest entries.
pub const MAX_BACKUP_MANIFEST_PATH_BYTES: usize = 16 * 1024 * 1024;
/// Progress journals are small fixed-shape control records, never bulk data.
pub const MAX_BACKUP_JOURNAL_BYTES: u64 = 16 * 1024;
pub(crate) const COPY_BUFFER_BYTES: usize = 64 * 1024;
pub(crate) const SPACE_RESERVE_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_PORTABLE_PATH_BYTES: usize = 240;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Export,
    Restore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    ProductSnapshot,
    AssetCatalogSnapshot,
    Objects,
    Copied,
    Verified,
    ReadyToPublish,
    OldMoved,
    NewPublished,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupProgress {
    pub journal_version: u32,
    pub operation: Operation,
    pub session_id: String,
    pub phase: Phase,
    pub destination_name: String,
    pub source_manifest_sha256: Option<String>,
    pub last_verified_object_hash: Option<String>,
    pub verified_objects: u64,
    pub verified_bytes: u64,
    pub required_bytes: u64,
    pub available_bytes: Option<u64>,
    #[serde(default)]
    pub replaced_existing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestDatabase {
    pub path: String,
    pub schema_version: i64,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotContract {
    pub database_order: String,
    pub asset_selection: String,
    pub concurrent_asset_add: String,
    pub concurrent_asset_delete: String,
}

impl Default for SnapshotContract {
    fn default() -> Self {
        Self {
            database_order:
                "sequential non-atomic cuts: product snapshot, then pinned asset-catalog snapshot"
                    .to_owned(),
            asset_selection: "objects are exactly active rows in the asset-catalog snapshot"
                .to_owned(),
            concurrent_asset_add: "assets committed after the catalog snapshot are excluded"
                .to_owned(),
            concurrent_asset_delete:
                "persisted export pins retain snapshotted objects until verification".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityReceipt {
    pub check_id: String,
    pub disposition: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub format: String,
    pub format_version: u32,
    pub session_id: String,
    pub product_database: ManifestDatabase,
    pub asset_catalog: ManifestDatabase,
    pub entries: Vec<BackupEntry>,
    pub total_entry_bytes: u64,
    pub snapshot_contract: SnapshotContract,
    pub compatibility_receipts: Vec<CompatibilityReceipt>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Control {
    Continue,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownSpacePolicy {
    FailClosed,
    ProceedWithExplicitUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportOptions {
    pub unknown_space_policy: UnknownSpacePolicy,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        }
    }
}

pub trait SpaceProbe {
    fn available_bytes(&self, path: &Path) -> io::Result<Option<u64>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FsSpaceProbe;

impl SpaceProbe for FsSpaceProbe {
    fn available_bytes(&self, path: &Path) -> io::Result<Option<u64>> {
        fs2::available_space(path).map(Some)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreeSpaceDisposition {
    Enough,
    Insufficient,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FreeSpaceAssessment {
    pub required_bytes: u64,
    pub available_bytes: Option<u64>,
    pub disposition: FreeSpaceDisposition,
}

pub struct ExportRequest<'a> {
    pub product: &'a Store,
    pub assets: &'a AssetStore,
    pub destination: &'a Path,
    pub secret_sentinels: &'a [SecretSentinel],
    pub space_probe: &'a dyn SpaceProbe,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportReport {
    pub destination: std::path::PathBuf,
    pub session_id: String,
    pub manifest_sha256: String,
    pub object_count: u64,
    pub total_entry_bytes: u64,
    pub free_space: FreeSpaceAssessment,
    pub resumed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestorePolicy {
    FailIfPresent,
    Replace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreOptions {
    pub policy: RestorePolicy,
    pub unknown_space_policy: UnknownSpacePolicy,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            policy: RestorePolicy::FailIfPresent,
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        }
    }
}

pub struct RestoreRequest<'a> {
    pub package: &'a Path,
    pub destination: &'a Path,
    pub space_probe: &'a dyn SpaceProbe,
    pub secret_sentinels: &'a [SecretSentinel],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreReport {
    pub destination: std::path::PathBuf,
    pub session_id: String,
    pub manifest_sha256: String,
    pub restored_bytes: u64,
    pub replaced_existing: bool,
    pub resumed: bool,
}
