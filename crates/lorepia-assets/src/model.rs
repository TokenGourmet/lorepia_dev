use std::{fmt, str::FromStr};

use crate::{AssetError, Result};

pub const MAX_OWNER_TYPE_BYTES: usize = 64;
pub const MAX_OWNER_ID_BYTES: usize = 512;
pub const MAX_SOURCE_NAME_BYTES: usize = 4_096;
pub const MAX_PAGE_SIZE: u16 = 1_000;
/// Backup snapshot leases older than this are abandoned during bounded maintenance.
pub const BACKUP_SNAPSHOT_LEASE_TIMEOUT_MS: i64 = 24 * 60 * 60 * 1_000;
/// Bounds live export sessions and therefore bounds one complete stale-session cleanup pass.
pub const MAX_BACKUP_SNAPSHOT_SESSIONS: u16 = 128;
/// Temporary-owner leases older than this can be reclaimed after an interrupted import.
pub const TEMPORARY_OWNER_LEASE_TIMEOUT_MS: i64 = 24 * 60 * 60 * 1_000;
/// Bounds live temporary-owner sessions and one complete recovery pass.
pub const MAX_TEMPORARY_OWNER_SESSIONS: u16 = 128;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct AssetHash(String);

impl AssetHash {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() != 64
            || !value
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(AssetError::InvalidInput {
                field: "asset hash",
                reason: "must be exactly 64 lowercase hexadecimal characters",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_digest(digest: &[u8]) -> Self {
        Self(hex::encode(digest))
    }
}

impl fmt::Display for AssetHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AssetHash {
    type Err = AssetError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetMime {
    Png,
    Jpeg,
    WebP,
    Gif,
    Wav,
    Mp3,
    Ogg,
    Flac,
}

impl AssetMime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::WebP => "image/webp",
            Self::Gif => "image/gif",
            Self::Wav => "audio/wav",
            Self::Mp3 => "audio/mpeg",
            Self::Ogg => "audio/ogg",
            Self::Flac => "audio/flac",
        }
    }
}

impl fmt::Display for AssetMime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AssetMime {
    type Err = AssetError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "image/png" => Ok(Self::Png),
            "image/jpeg" => Ok(Self::Jpeg),
            "image/webp" => Ok(Self::WebP),
            "image/gif" => Ok(Self::Gif),
            "audio/wav" => Ok(Self::Wav),
            "audio/mpeg" => Ok(Self::Mp3),
            "audio/ogg" => Ok(Self::Ogg),
            "audio/flac" => Ok(Self::Flac),
            _ => Err(AssetError::InvalidInput {
                field: "declared MIME",
                reason: "MIME is not in the product allowlist",
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetLimits {
    pub max_object_bytes: u64,
    pub max_total_bytes: u64,
    pub max_image_width: u32,
    pub max_image_height: u32,
    pub max_image_pixels: u64,
}

impl AssetLimits {
    pub fn new(max_object_bytes: u64, max_total_bytes: u64) -> Result<Self> {
        if max_object_bytes == 0 {
            return Err(AssetError::InvalidInput {
                field: "max object bytes",
                reason: "must be greater than zero",
            });
        }
        if max_total_bytes < max_object_bytes {
            return Err(AssetError::InvalidInput {
                field: "max total bytes",
                reason: "must be at least max object bytes",
            });
        }
        if max_total_bytes > i64::MAX as u64 {
            return Err(AssetError::InvalidInput {
                field: "max total bytes",
                reason: "must fit in SQLite's signed 64-bit INTEGER range",
            });
        }
        Ok(Self {
            max_object_bytes,
            max_total_bytes,
            max_image_width: 32_768,
            max_image_height: 32_768,
            max_image_pixels: 268_435_456,
        })
    }

    pub fn with_image_limits(
        mut self,
        max_width: u32,
        max_height: u32,
        max_pixels: u64,
    ) -> Result<Self> {
        if max_width == 0 || max_height == 0 || max_pixels == 0 {
            return Err(AssetError::InvalidInput {
                field: "image limits",
                reason: "dimensions and pixel limit must be greater than zero",
            });
        }
        self.max_image_width = max_width;
        self.max_image_height = max_height;
        self.max_image_pixels = max_pixels;
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct AssetOwner {
    pub owner_type: String,
    pub owner_id: String,
}

impl AssetOwner {
    pub fn new(owner_type: impl Into<String>, owner_id: impl Into<String>) -> Result<Self> {
        let owner_type = owner_type.into();
        let owner_id = owner_id.into();
        validate_owner_type(&owner_type)?;
        validate_owner_id(&owner_id)?;
        Ok(Self {
            owner_type,
            owner_id,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct AssetReference {
    pub owner: AssetOwner,
    pub hash: AssetHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestRequest {
    pub declared_mime: AssetMime,
    pub source_name: Option<String>,
    pub owner: Option<AssetOwner>,
}

impl IngestRequest {
    pub fn new(declared_mime: AssetMime) -> Self {
        Self {
            declared_mime,
            source_name: None,
            owner: None,
        }
    }

    pub fn with_source_name(mut self, source_name: impl Into<String>) -> Result<Self> {
        let source_name = source_name.into();
        if source_name.len() > MAX_SOURCE_NAME_BYTES {
            return Err(AssetError::InvalidInput {
                field: "source name",
                reason: "is too long",
            });
        }
        if source_name.contains('\0') {
            return Err(AssetError::InvalidInput {
                field: "source name",
                reason: "must not contain NUL",
            });
        }
        self.source_name = Some(source_name);
        Ok(self)
    }

    pub fn with_owner(mut self, owner: AssetOwner) -> Self {
        self.owner = Some(owner);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetState {
    Active,
    Missing,
    Quarantined,
}

impl AssetState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Missing => "missing",
            Self::Quarantined => "quarantined",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "missing" => Ok(Self::Missing),
            "quarantined" => Ok(Self::Quarantined),
            _ => Err(AssetError::IncompatibleCatalog {
                reason: "asset object has an unknown state",
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetObject {
    pub hash: AssetHash,
    pub size: u64,
    pub mime: AssetMime,
    pub relative_path: String,
    pub state: AssetState,
    pub verified_at_ms: i64,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestOutcome {
    pub object: AssetObject,
    pub deduplicated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetObjectPage {
    pub objects: Vec<AssetObject>,
    pub next_cursor: Option<AssetHash>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetStats {
    pub object_count: u64,
    pub active_bytes: u64,
    pub reference_count: u64,
    pub missing_count: u64,
    pub quarantined_count: u64,
    pub staging_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconcileFindingKind {
    Missing,
    Corrupt,
    Restored,
    MetadataPathRepaired,
    FilesystemOrphanQuarantined,
    InvalidEntryQuarantined,
    UnsafeEntry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcileFinding {
    pub hash: Option<AssetHash>,
    pub kind: ReconcileFindingKind,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcilePage {
    pub checked: u16,
    pub findings: Vec<ReconcileFinding>,
    pub next_cursor: Option<AssetHash>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkSweepPage {
    pub removed: Vec<AssetHash>,
    pub next_cursor: Option<AssetHash>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupPage {
    pub removed_names: Vec<String>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardReconcilePage {
    pub examined: u16,
    pub findings: Vec<ReconcileFinding>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportedObject {
    pub hash: AssetHash,
    pub bytes_written: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupSnapshotLease {
    pub session_id: String,
    pub pinned_objects: u64,
    pub lease_updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupSnapshotCleanup {
    pub removed_sessions: Vec<String>,
    pub released_pins: u64,
}

pub(crate) fn validate_owner(owner: &AssetOwner) -> Result<()> {
    validate_owner_type(&owner.owner_type)?;
    validate_owner_id(&owner.owner_id)
}

fn validate_owner_type(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_OWNER_TYPE_BYTES {
        return Err(AssetError::InvalidInput {
            field: "owner type",
            reason: "must be between 1 and 64 bytes",
        });
    }
    if !value.as_bytes().iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
    }) {
        return Err(AssetError::InvalidInput {
            field: "owner type",
            reason: "must use lowercase ASCII letters, digits, dot, dash, or underscore",
        });
    }
    Ok(())
}

fn validate_owner_id(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_OWNER_ID_BYTES {
        return Err(AssetError::InvalidInput {
            field: "owner id",
            reason: "must be between 1 and 512 bytes",
        });
    }
    if value.contains('\0') {
        return Err(AssetError::InvalidInput {
            field: "owner id",
            reason: "must not contain NUL",
        });
    }
    Ok(())
}
