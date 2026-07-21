use lorepia_assets::{AssetHash, AssetMime};
use serde::{Deserialize, Serialize};

use crate::{ImportError, ImportErrorCode, Result};

pub const IMPORT_POLICY_VERSION: &str = "lorepia-import-v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportLimits {
    pub max_source_bytes: u64,
    pub max_archive_entries: usize,
    pub max_entry_bytes: u64,
    pub max_total_uncompressed_bytes: u64,
    pub max_compression_ratio: u64,
    pub max_path_bytes: usize,
    pub max_component_bytes: usize,
    pub max_path_depth: usize,
    pub max_zip_extra_bytes: usize,
    pub copy_buffer_bytes: usize,
    pub max_png_bytes: u64,
    pub max_png_chunks: usize,
    pub max_png_chunk_bytes: u64,
    pub max_png_metadata_bytes: u64,
    pub max_png_width: u32,
    pub max_png_height: u32,
    pub max_png_pixels: u64,
    pub max_png_decode_bytes: usize,
    pub max_metadata_bytes: u64,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 256 * 1024 * 1024,
            max_archive_entries: 2_048,
            max_entry_bytes: 64 * 1024 * 1024,
            max_total_uncompressed_bytes: 512 * 1024 * 1024,
            max_compression_ratio: 200,
            max_path_bytes: 512,
            max_component_bytes: 128,
            max_path_depth: 16,
            max_zip_extra_bytes: 16 * 1024,
            copy_buffer_bytes: 64 * 1024,
            max_png_bytes: 64 * 1024 * 1024,
            max_png_chunks: 4_096,
            max_png_chunk_bytes: 32 * 1024 * 1024,
            max_png_metadata_bytes: 4 * 1024 * 1024,
            max_png_width: 16_384,
            max_png_height: 16_384,
            max_png_pixels: 67_108_864,
            max_png_decode_bytes: 256 * 1024 * 1024,
            max_metadata_bytes: 8 * 1024 * 1024,
        }
    }
}

impl ImportLimits {
    pub fn validate(&self) -> Result<()> {
        if self.max_source_bytes == 0
            || self.max_archive_entries == 0
            || self.max_entry_bytes == 0
            || self.max_total_uncompressed_bytes < self.max_entry_bytes
            || self.max_compression_ratio == 0
            || self.max_path_bytes == 0
            || self.max_component_bytes == 0
            || self.max_path_depth == 0
            || self.max_zip_extra_bytes == 0
            || !(4 * 1024..=1024 * 1024).contains(&self.copy_buffer_bytes)
            || self.max_png_bytes == 0
            || self.max_png_chunks == 0
            || self.max_png_chunk_bytes == 0
            || self.max_png_metadata_bytes == 0
            || self.max_png_width == 0
            || self.max_png_height == 0
            || self.max_png_pixels == 0
            || self.max_png_decode_bytes == 0
            || self.max_metadata_bytes == 0
        {
            return Err(ImportError::new(ImportErrorCode::Internal));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImportSourceKind {
    ZipArchive,
    PngCard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutableLanguage {
    JavaScript,
    Lua,
    WebAssembly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedAsset {
    pub logical_path: String,
    pub hash: AssetHash,
    pub bytes: u64,
    pub mime: AssetMime,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AcceptedMetadata {
    pub logical_path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuarantinedExecutable {
    pub metadata_contract_version: u8,
    pub logical_path: String,
    pub language: ExecutableLanguage,
    pub sha256: String,
    pub bytes: u64,
    pub disposition: &'static str,
    pub executable: bool,
    pub policy: &'static str,
}

impl QuarantinedExecutable {
    pub(crate) fn new(
        logical_path: String,
        language: ExecutableLanguage,
        sha256: String,
        bytes: u64,
    ) -> Self {
        Self {
            metadata_contract_version: 1,
            logical_path,
            language,
            sha256,
            bytes,
            disposition: "INERT_QUARANTINED",
            executable: false,
            policy: "DISABLED_BY_SECURITY_POLICY",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImportCounts {
    pub accepted: u32,
    pub quarantined: u32,
    pub rejected: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportReceipt {
    pub protocol_version: u8,
    pub policy_version: &'static str,
    pub source_kind: ImportSourceKind,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub counts: ImportCounts,
    pub assets: Vec<AcceptedAsset>,
    pub metadata: Vec<AcceptedMetadata>,
    pub quarantined: Vec<QuarantinedExecutable>,
    pub executable_entries_executed: u32,
}
