use std::{fmt, io};

/// Stable, transport-safe product error codes. Attacker-controlled paths and parser messages are
/// deliberately not included in the public error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportErrorCode {
    Busy,
    Cancelled,
    SourceTooLarge,
    UnsupportedFormat,
    ArchiveMalformed,
    UnsupportedCompression,
    EntryCountLimit,
    EntrySizeLimit,
    TotalSizeLimit,
    CompressionRatioLimit,
    UnsafePath,
    DuplicatePath,
    UnsafeEntryType,
    UnsupportedFileType,
    PngMalformed,
    PngDimensionLimit,
    PngMetadataLimit,
    PngAnimationUnsupported,
    MetadataMalformed,
    AssetRejected,
    StagingFailure,
    CleanupFailure,
    Internal,
}

impl ImportErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "IMPORT_BUSY",
            Self::Cancelled => "IMPORT_CANCELLED",
            Self::SourceTooLarge => "SOURCE_TOO_LARGE",
            Self::UnsupportedFormat => "UNSUPPORTED_FORMAT",
            Self::ArchiveMalformed => "ARCHIVE_MALFORMED",
            Self::UnsupportedCompression => "UNSUPPORTED_COMPRESSION",
            Self::EntryCountLimit => "ENTRY_COUNT_LIMIT",
            Self::EntrySizeLimit => "ENTRY_SIZE_LIMIT",
            Self::TotalSizeLimit => "TOTAL_SIZE_LIMIT",
            Self::CompressionRatioLimit => "COMPRESSION_RATIO_LIMIT",
            Self::UnsafePath => "UNSAFE_PATH",
            Self::DuplicatePath => "DUPLICATE_PATH",
            Self::UnsafeEntryType => "UNSAFE_ENTRY_TYPE",
            Self::UnsupportedFileType => "UNSUPPORTED_FILE_TYPE",
            Self::PngMalformed => "PNG_MALFORMED",
            Self::PngDimensionLimit => "PNG_DIMENSION_LIMIT",
            Self::PngMetadataLimit => "PNG_METADATA_LIMIT",
            Self::PngAnimationUnsupported => "PNG_ANIMATION_UNSUPPORTED",
            Self::MetadataMalformed => "METADATA_MALFORMED",
            Self::AssetRejected => "ASSET_REJECTED",
            Self::StagingFailure => "STAGING_FAILURE",
            Self::CleanupFailure => "CLEANUP_FAILURE",
            Self::Internal => "IMPORT_INTERNAL",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportError {
    pub code: ImportErrorCode,
    pub cleanup_pending: bool,
    pub rejected_entries: u32,
}

impl ImportError {
    pub(crate) const fn new(code: ImportErrorCode) -> Self {
        Self {
            code,
            cleanup_pending: false,
            rejected_entries: 1,
        }
    }

    pub(crate) const fn cleanup_pending(mut self) -> Self {
        self.cleanup_pending = true;
        self
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.as_str())
    }
}

impl std::error::Error for ImportError {}

pub type Result<T> = std::result::Result<T, ImportError>;

pub(crate) fn io_code(error: &io::Error, fallback: ImportErrorCode) -> ImportError {
    if error.kind() == io::ErrorKind::Interrupted {
        ImportError::new(ImportErrorCode::Cancelled)
    } else {
        ImportError::new(fallback)
    }
}
