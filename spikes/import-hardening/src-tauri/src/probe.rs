use crc32fast::Hasher as Crc32;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::{Mutex, TryLockError};
use unicode_normalization::UnicodeNormalization;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, System, ZipArchive, ZipWriter};

const PROTOCOL_VERSION: u8 = 1;
const POLICY_VERSION: &str = "m1-import-hardening-v1";
const CATALOG_JSON: &str = include_str!("../../fixtures/import-cases-v1.json");
const CATALOG_SHA256: &str = "484a313423d4e91c792818fb64097d96f8efb7c4a31befe96a1d3f739bfe5eb2";
const ZIP_VERSION: &str = "8.6.0";
const PNG_VERSION: &str = "0.18.1";
const VALID_ARCHIVE_SHA256: &str =
    "485733d6f60763ef1e2e63b4595debc63500d2b67321ea0e4ffa3084b611dc0f";
const VALID_ARCHIVE_BYTES: usize = 665;
const VALID_ARCHIVE_TOTAL_BYTES: u64 = 217;
const VALID_DIRECT_PNG_SHA256: &str =
    "ff36b8831e688e8fb5a511d916e82621821f67ce1c1c8ee204c395702c5a1a04";
const VALID_DIRECT_PNG_BYTES: usize = 70;

const PROBE_ROOT_NAME: &str = "lorepia-m1-import-hardening-probe-v1";
const SENTINEL_NAME: &str = "lorepia-m1-import-hardening-outside-sentinel-v1";
const SENTINEL_CONTENT: &[u8] = b"lorepia-import-hardening-outside-sentinel-v1\n";

const MAX_SOURCE_BYTES: usize = 2_097_152;
const MAX_ARCHIVE_ENTRIES: usize = 32;
const MAX_ENTRY_BYTES: u64 = 524_288;
const MAX_TOTAL_BYTES: u64 = 1_048_576;
const MAX_COMPRESSION_RATIO: u64 = 100;
const MAX_PATH_BYTES: usize = 240;
const MAX_COMPONENT_BYTES: usize = 64;
const MAX_PATH_DEPTH: usize = 8;
const STREAM_BUFFER_BYTES: usize = 16_384;
const MAX_PNG_BYTES: usize = 524_288;
const MAX_PNG_CHUNKS: usize = 64;
const MAX_PNG_CHUNK_PAYLOAD: usize = 262_144;
const MAX_PNG_WIDTH: u32 = 2_048;
const MAX_PNG_HEIGHT: u32 = 2_048;
const MAX_PNG_PIXELS: u64 = 4_194_304;
const MAX_PNG_DECODE_BYTES: usize = 16_777_216;
const MAX_INDEX_BYTES: usize = 16_384;
const MAX_IPC_BYTES: usize = 4_096;

static PROBE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProbeErrorCode {
    PathUnavailable,
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
    PngMalformed,
    UnsupportedFileType,
    StagingFailure,
    PublishConflict,
    PublishFailure,
    CleanupFailure,
    ProbeBusy,
    InternalState,
}

const ALL_PROBE_ERROR_CODES: [ProbeErrorCode; 20] = [
    ProbeErrorCode::PathUnavailable,
    ProbeErrorCode::SourceTooLarge,
    ProbeErrorCode::UnsupportedFormat,
    ProbeErrorCode::ArchiveMalformed,
    ProbeErrorCode::UnsupportedCompression,
    ProbeErrorCode::EntryCountLimit,
    ProbeErrorCode::EntrySizeLimit,
    ProbeErrorCode::TotalSizeLimit,
    ProbeErrorCode::CompressionRatioLimit,
    ProbeErrorCode::UnsafePath,
    ProbeErrorCode::DuplicatePath,
    ProbeErrorCode::UnsafeEntryType,
    ProbeErrorCode::PngMalformed,
    ProbeErrorCode::UnsupportedFileType,
    ProbeErrorCode::StagingFailure,
    ProbeErrorCode::PublishConflict,
    ProbeErrorCode::PublishFailure,
    ProbeErrorCode::CleanupFailure,
    ProbeErrorCode::ProbeBusy,
    ProbeErrorCode::InternalState,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeError {
    protocol_version: u8,
    code: ProbeErrorCode,
    cleanup_pending: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CaseOutcome {
    Accepted,
    Rejected,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CaseCode {
    SourceTooLarge,
    UnsupportedFormat,
    UnsafePath,
    DuplicatePath,
    UnsafeEntryType,
    UnsupportedCompression,
    EntryCountLimit,
    EntrySizeLimit,
    TotalSizeLimit,
    CompressionRatioLimit,
    ArchiveMalformed,
    PngMalformed,
    UnsupportedFileType,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum FixtureKind {
    Zip,
    Png,
    Raw,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeLimits {
    source_bytes: usize,
    entry_count: usize,
    entry_bytes: u64,
    total_uncompressed_bytes: u64,
    compression_ratio: u64,
    path_bytes: usize,
    path_component_bytes: usize,
    path_depth: usize,
    stream_buffer_bytes: usize,
    png_bytes: usize,
    png_chunks: usize,
    png_chunk_bytes: usize,
    png_width: u32,
    png_height: u32,
    png_pixels: u64,
    png_decoded_bytes: usize,
    index_bytes: usize,
    ipc_response_bytes: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseReceipt {
    case_id: String,
    outcome: CaseOutcome,
    code: Option<CaseCode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidArchiveEvidence {
    source_sha256: String,
    source_bytes: usize,
    entry_count: usize,
    total_uncompressed_bytes: u64,
    script_entries: usize,
    executed_entries: usize,
    quarantine: &'static str,
    atomic_publish: bool,
    reopened_hash_verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidDirectPngEvidence {
    source_sha256: String,
    source_bytes: usize,
    width: u32,
    height: u32,
    atomic_publish: bool,
    reopened_hash_verified: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefenseEvidence {
    traversal_rejected: bool,
    collision_rejected: bool,
    unsafe_entry_types_rejected: bool,
    size_limits_enforced: bool,
    compression_ratio_enforced: bool,
    malformed_archive_rejected: bool,
    strict_png_validated: bool,
    unsupported_files_rejected: bool,
    outside_sentinel_preserved: bool,
    staging_cleaned: bool,
    script_execution_disabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReceipt {
    protocol_version: u8,
    policy_version: &'static str,
    fixture_catalog_sha256: &'static str,
    zip_version: &'static str,
    png_version: &'static str,
    limits: ProbeLimits,
    cases: Vec<CaseReceipt>,
    valid_archive: ValidArchiveEvidence,
    valid_direct_png: ValidDirectPngEvidence,
    defenses: DefenseEvidence,
    cleanup_pending: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureCatalog {
    version: u8,
    policy_version: String,
    license: String,
    provenance: String,
    cases: Vec<CatalogCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogCase {
    case_id: String,
    fixture_kind: FixtureKind,
    expected_outcome: CaseOutcome,
    expected_code: Option<CaseCode>,
}

#[derive(Clone, Copy)]
struct ExpectedCase {
    case_id: &'static str,
    fixture_kind: FixtureKind,
    expected_outcome: CaseOutcome,
    expected_code: Option<CaseCode>,
}

const EXPECTED_CASES: [ExpectedCase; 26] = [
    expected(
        "valid-archive",
        FixtureKind::Zip,
        CaseOutcome::Accepted,
        None,
    ),
    expected(
        "valid-direct-png",
        FixtureKind::Png,
        CaseOutcome::Accepted,
        None,
    ),
    expected(
        "source-too-large",
        FixtureKind::Raw,
        CaseOutcome::Rejected,
        Some(CaseCode::SourceTooLarge),
    ),
    expected(
        "unsupported-source",
        FixtureKind::Raw,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsupportedFormat),
    ),
    expected(
        "parent-traversal",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsafePath),
    ),
    expected(
        "nested-parent-traversal",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsafePath),
    ),
    expected(
        "absolute-posix-path",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsafePath),
    ),
    expected(
        "windows-drive-path",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsafePath),
    ),
    expected(
        "backslash-path",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsafePath),
    ),
    expected(
        "reserved-device-path",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsafePath),
    ),
    expected(
        "non-nfc-path",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsafePath),
    ),
    expected(
        "exact-duplicate-path",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::DuplicatePath),
    ),
    expected(
        "case-collision",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::DuplicatePath),
    ),
    expected(
        "prefix-conflict",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::DuplicatePath),
    ),
    expected(
        "symlink-entry",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsafeEntryType),
    ),
    expected(
        "unsupported-compression",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsupportedCompression),
    ),
    expected(
        "too-many-entries",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::EntryCountLimit),
    ),
    expected(
        "oversized-entry",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::EntrySizeLimit),
    ),
    expected(
        "oversized-total",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::TotalSizeLimit),
    ),
    expected(
        "high-compression-ratio",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::CompressionRatioLimit),
    ),
    expected(
        "malformed-archive",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::ArchiveMalformed),
    ),
    expected(
        "png-bad-crc",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::PngMalformed),
    ),
    expected(
        "png-truncated-chunk",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::PngMalformed),
    ),
    expected(
        "png-trailing-bytes",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::PngMalformed),
    ),
    expected(
        "png-oversized-dimensions",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::PngMalformed),
    ),
    expected(
        "unsupported-file-type",
        FixtureKind::Zip,
        CaseOutcome::Rejected,
        Some(CaseCode::UnsupportedFileType),
    ),
];

const fn expected(
    case_id: &'static str,
    fixture_kind: FixtureKind,
    expected_outcome: CaseOutcome,
    expected_code: Option<CaseCode>,
) -> ExpectedCase {
    ExpectedCase {
        case_id,
        fixture_kind,
        expected_outcome,
        expected_code,
    }
}

#[derive(Debug)]
enum ImportFailure {
    Rejected(CaseCode),
    Harness(ProbeErrorCode),
}

#[derive(Debug)]
enum AcceptedImport {
    Archive(ValidArchiveEvidence),
    DirectPng(ValidDirectPngEvidence),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    Manifest,
    Png,
    Script,
}

#[derive(Debug)]
struct EntryPlan {
    index: usize,
    logical_path: String,
    declared_size: u64,
    kind: EntryKind,
}

#[derive(Debug)]
struct ZipCentralPlan {
    central_header_offset: usize,
    raw_name: Vec<u8>,
    flags: u16,
    compression: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    disk_start: u16,
    local_header_offset: u32,
    local_data_start: usize,
    local_data_end: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredIndex {
    version: u8,
    entries: Vec<StoredIndexEntry>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredIndexEntry {
    logical_path: String,
    object_name: String,
    sha256: String,
    bytes: u64,
    disposition: String,
}

#[derive(Debug)]
struct FixtureEntry {
    name: String,
    data: Vec<u8>,
    compression: CompressionMethod,
    symlink_target: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PngInfo {
    width: u32,
    height: u32,
}

fn probe_error(code: ProbeErrorCode) -> ProbeError {
    ProbeError {
        protocol_version: PROTOCOL_VERSION,
        code,
        cleanup_pending: false,
    }
}

pub fn path_unavailable_error() -> ProbeError {
    probe_error(ProbeErrorCode::PathUnavailable)
}

pub fn internal_state_error() -> ProbeError {
    probe_error(ProbeErrorCode::InternalState)
}

pub fn with_process_lock<T>(
    operation: impl FnOnce() -> Result<T, ProbeError>,
) -> Result<T, ProbeError> {
    // This lock only serializes this app process. It does not prove resistance to a hostile
    // local process racing filesystem paths, no-follow semantics, or crash durability.
    let _guard = match PROBE_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Err(probe_error(ProbeErrorCode::ProbeBusy)),
        Err(TryLockError::Poisoned(_)) => {
            return Err(probe_error(ProbeErrorCode::InternalState));
        }
    };
    operation()
}

pub fn run_probe_in_directory(directory: &Path) -> Result<ProbeReceipt, ProbeError> {
    fs::create_dir_all(directory).map_err(|_| path_unavailable_error())?;
    let root = directory.join(PROBE_ROOT_NAME);
    let sentinel = directory.join(SENTINEL_NAME);

    if cleanup_owned_paths(&root, &sentinel).is_err() {
        return Err(ProbeError {
            cleanup_pending: true,
            ..probe_error(ProbeErrorCode::CleanupFailure)
        });
    }

    let run_result =
        prepare_owned_paths(&root, &sentinel).and_then(|()| run_fixture_catalog(&root, &sentinel));
    let cleanup_result = cleanup_owned_paths(&root, &sentinel);

    match (run_result, cleanup_result) {
        (Ok(receipt), Ok(())) => Ok(receipt),
        (Ok(_), Err(())) => Err(ProbeError {
            cleanup_pending: true,
            ..probe_error(ProbeErrorCode::CleanupFailure)
        }),
        (Err(mut error), Ok(())) => {
            error.cleanup_pending = false;
            Err(error)
        }
        (Err(mut error), Err(())) => {
            error.cleanup_pending = true;
            Err(error)
        }
    }
}

fn prepare_owned_paths(root: &Path, sentinel: &Path) -> Result<(), ProbeError> {
    fs::create_dir(root).map_err(|_| path_unavailable_error())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(sentinel)
        .map_err(|_| path_unavailable_error())?;
    file.write_all(SENTINEL_CONTENT)
        .map_err(|_| path_unavailable_error())
}

fn run_fixture_catalog(root: &Path, sentinel: &Path) -> Result<ProbeReceipt, ProbeError> {
    ensure_failure_receipts_are_bounded()?;
    let catalog = parse_and_validate_catalog()?;
    let mut cases = Vec::with_capacity(catalog.cases.len());
    let mut valid_archive = None;
    let mut valid_direct_png = None;

    for (ordinal, fixture_case) in catalog.cases.iter().enumerate() {
        let source = generate_fixture(&fixture_case.case_id)?;
        let staging = root.join(format!("case-{ordinal:02}.staging"));
        let published = root.join(format!("case-{ordinal:02}.published"));
        cleanup_case_paths(&staging, &published).map_err(|()| cleanup_error(true))?;

        let import_result = import_source(&source, &staging, &published);
        let (outcome, code) = match import_result {
            Ok(AcceptedImport::Archive(evidence)) => {
                if fixture_case.case_id != "valid-archive" || valid_archive.is_some() {
                    return Err(internal_state_error());
                }
                valid_archive = Some(evidence);
                (CaseOutcome::Accepted, None)
            }
            Ok(AcceptedImport::DirectPng(evidence)) => {
                if fixture_case.case_id != "valid-direct-png" || valid_direct_png.is_some() {
                    return Err(internal_state_error());
                }
                valid_direct_png = Some(evidence);
                (CaseOutcome::Accepted, None)
            }
            Err(ImportFailure::Rejected(code)) => (CaseOutcome::Rejected, Some(code)),
            Err(ImportFailure::Harness(code)) => return Err(probe_error(code)),
        };

        let cleanup_result = cleanup_case_paths(&staging, &published);
        if cleanup_result.is_err() {
            return Err(cleanup_error(true));
        }
        verify_case_is_clean(root, &staging, &published, sentinel)?;

        if outcome != fixture_case.expected_outcome || code != fixture_case.expected_code {
            return Err(internal_state_error());
        }
        cases.push(CaseReceipt {
            case_id: fixture_case.case_id.clone(),
            outcome,
            code,
        });
    }

    let valid_archive = valid_archive.ok_or_else(internal_state_error)?;
    let valid_direct_png = valid_direct_png.ok_or_else(internal_state_error)?;
    if valid_archive.source_sha256 != VALID_ARCHIVE_SHA256
        || valid_archive.source_bytes != VALID_ARCHIVE_BYTES
        || valid_archive.entry_count != 4
        || valid_archive.total_uncompressed_bytes != VALID_ARCHIVE_TOTAL_BYTES
        || valid_archive.script_entries != 2
        || valid_archive.executed_entries != 0
        || valid_archive.quarantine != "inert"
        || valid_direct_png.source_sha256 != VALID_DIRECT_PNG_SHA256
        || valid_direct_png.source_bytes != VALID_DIRECT_PNG_BYTES
        || valid_direct_png.width != 1
        || valid_direct_png.height != 1
    {
        return Err(internal_state_error());
    }

    let receipt = ProbeReceipt {
        protocol_version: PROTOCOL_VERSION,
        policy_version: POLICY_VERSION,
        fixture_catalog_sha256: CATALOG_SHA256,
        zip_version: ZIP_VERSION,
        png_version: PNG_VERSION,
        limits: probe_limits(),
        cases,
        valid_archive,
        valid_direct_png,
        defenses: DefenseEvidence {
            traversal_rejected: true,
            collision_rejected: true,
            unsafe_entry_types_rejected: true,
            size_limits_enforced: true,
            compression_ratio_enforced: true,
            malformed_archive_rejected: true,
            strict_png_validated: true,
            unsupported_files_rejected: true,
            outside_sentinel_preserved: true,
            staging_cleaned: true,
            script_execution_disabled: true,
        },
        cleanup_pending: false,
    };

    if serde_json::to_vec(&receipt)
        .map_err(|_| internal_state_error())?
        .len()
        > MAX_IPC_BYTES
    {
        return Err(internal_state_error());
    }
    Ok(receipt)
}

fn ensure_failure_receipts_are_bounded() -> Result<(), ProbeError> {
    for code in ALL_PROBE_ERROR_CODES {
        let bytes = serde_json::to_vec(&ProbeError {
            protocol_version: PROTOCOL_VERSION,
            code,
            cleanup_pending: true,
        })
        .map_err(|_| internal_state_error())?;
        if bytes.len() > MAX_IPC_BYTES {
            return Err(internal_state_error());
        }
    }
    Ok(())
}

fn probe_limits() -> ProbeLimits {
    ProbeLimits {
        source_bytes: MAX_SOURCE_BYTES,
        entry_count: MAX_ARCHIVE_ENTRIES,
        entry_bytes: MAX_ENTRY_BYTES,
        total_uncompressed_bytes: MAX_TOTAL_BYTES,
        compression_ratio: MAX_COMPRESSION_RATIO,
        path_bytes: MAX_PATH_BYTES,
        path_component_bytes: MAX_COMPONENT_BYTES,
        path_depth: MAX_PATH_DEPTH,
        stream_buffer_bytes: STREAM_BUFFER_BYTES,
        png_bytes: MAX_PNG_BYTES,
        png_chunks: MAX_PNG_CHUNKS,
        png_chunk_bytes: MAX_PNG_CHUNK_PAYLOAD,
        png_width: MAX_PNG_WIDTH,
        png_height: MAX_PNG_HEIGHT,
        png_pixels: MAX_PNG_PIXELS,
        png_decoded_bytes: MAX_PNG_DECODE_BYTES,
        index_bytes: MAX_INDEX_BYTES,
        ipc_response_bytes: MAX_IPC_BYTES,
    }
}

fn parse_and_validate_catalog() -> Result<FixtureCatalog, ProbeError> {
    if sha256_hex(CATALOG_JSON.as_bytes()) != CATALOG_SHA256 {
        return Err(internal_state_error());
    }
    let catalog: FixtureCatalog =
        serde_json::from_str(CATALOG_JSON).map_err(|_| internal_state_error())?;
    if catalog.version != 1
        || catalog.policy_version != POLICY_VERSION
        || catalog.license != "CC0-1.0"
        || catalog.provenance
            != "Self-authored deterministic fixtures generated by the Rust probe; no third-party card or application data."
        || catalog.cases.len() != EXPECTED_CASES.len()
    {
        return Err(internal_state_error());
    }

    for (actual, expected) in catalog.cases.iter().zip(EXPECTED_CASES) {
        if actual.case_id != expected.case_id
            || actual.fixture_kind != expected.fixture_kind
            || actual.expected_outcome != expected.expected_outcome
            || actual.expected_code != expected.expected_code
        {
            return Err(internal_state_error());
        }
    }
    Ok(catalog)
}

fn import_source(
    source: &[u8],
    staging: &Path,
    published: &Path,
) -> Result<AcceptedImport, ImportFailure> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ImportFailure::Rejected(CaseCode::SourceTooLarge));
    }
    if source.starts_with(b"\x89PNG\r\n\x1a\n") {
        return import_direct_png(source, staging, published).map(AcceptedImport::DirectPng);
    }
    if is_zip_signature(source) {
        return import_archive(source, staging, published).map(AcceptedImport::Archive);
    }
    Err(ImportFailure::Rejected(CaseCode::UnsupportedFormat))
}

fn is_zip_signature(source: &[u8]) -> bool {
    matches!(source.get(..4), Some(b"PK\x03\x04") | Some(b"PK\x05\x06"))
}

fn import_archive(
    source: &[u8],
    staging: &Path,
    published: &Path,
) -> Result<ValidArchiveEvidence, ImportFailure> {
    let central_plans = preflight_zip_container(source)?;
    let mut archive = ZipArchive::new(Cursor::new(source))
        .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    if archive.len() != central_plans.len() {
        return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
    }
    crosscheck_zip_archive(&mut archive, &central_plans)?;
    let plans = preflight_archive(&mut archive)?;

    fs::create_dir(staging).map_err(|_| ImportFailure::Harness(ProbeErrorCode::StagingFailure))?;
    let mut stored_entries = Vec::with_capacity(plans.len());
    let mut total_actual = 0_u64;
    let mut script_entries = 0_usize;
    let mut manifest_seen = false;
    let mut png_seen = false;

    for plan in &plans {
        let mut entry = archive
            .by_index(plan.index)
            .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let mut data = Vec::with_capacity(
            usize::try_from(plan.declared_size)
                .unwrap_or(MAX_ENTRY_BYTES as usize)
                .min(MAX_ENTRY_BYTES as usize),
        );
        let mut buffer = [0_u8; STREAM_BUFFER_BYTES];
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
            if read == 0 {
                break;
            }
            let next_len = data
                .len()
                .checked_add(read)
                .ok_or(ImportFailure::Rejected(CaseCode::EntrySizeLimit))?;
            if next_len as u64 > MAX_ENTRY_BYTES || next_len as u64 > plan.declared_size {
                return Err(ImportFailure::Rejected(CaseCode::EntrySizeLimit));
            }
            let next_total = total_actual
                .checked_add(read as u64)
                .ok_or(ImportFailure::Rejected(CaseCode::TotalSizeLimit))?;
            if next_total > MAX_TOTAL_BYTES {
                return Err(ImportFailure::Rejected(CaseCode::TotalSizeLimit));
            }
            data.extend_from_slice(&buffer[..read]);
            total_actual = next_total;
        }
        if data.len() as u64 != plan.declared_size {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }

        if plan.kind == EntryKind::Png {
            validate_png(&data)?;
        }

        let hash = sha256_hex(&data);
        match plan.kind {
            EntryKind::Manifest => manifest_seen = true,
            EntryKind::Png => png_seen = true,
            EntryKind::Script => script_entries += 1,
        }
        let object_name = format!("object-{:02}-{}.bin", plan.index, &hash[..16]);
        write_new_file(&staging.join(&object_name), &data)?;
        stored_entries.push(StoredIndexEntry {
            logical_path: plan.logical_path.clone(),
            object_name,
            sha256: hash,
            bytes: data.len() as u64,
            disposition: if plan.kind == EntryKind::Script {
                "quarantined-inert"
            } else {
                "validated-data"
            }
            .to_owned(),
        });
    }

    let index = StoredIndex {
        version: 1,
        entries: stored_entries,
    };
    let index_bytes = serde_json::to_vec(&index)
        .map_err(|_| ImportFailure::Harness(ProbeErrorCode::InternalState))?;
    if index_bytes.len() > MAX_INDEX_BYTES {
        return Err(ImportFailure::Harness(ProbeErrorCode::InternalState));
    }
    write_new_file(&staging.join("index.json"), &index_bytes)?;
    atomic_publish(staging, published)?;
    verify_reopened_published_index(published, &index)?;
    if !manifest_seen || !png_seen {
        return Err(ImportFailure::Harness(ProbeErrorCode::InternalState));
    }

    Ok(ValidArchiveEvidence {
        source_sha256: sha256_hex(source),
        source_bytes: source.len(),
        entry_count: plans.len(),
        total_uncompressed_bytes: total_actual,
        script_entries,
        executed_entries: 0,
        quarantine: "inert",
        atomic_publish: true,
        reopened_hash_verified: true,
    })
}

fn preflight_zip_container(source: &[u8]) -> Result<Vec<ZipCentralPlan>, ImportFailure> {
    const EOCD_MIN_BYTES: usize = 22;
    const CENTRAL_FIXED_BYTES: usize = 46;
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ImportFailure::Rejected(CaseCode::SourceTooLarge));
    }
    if source.len() < EOCD_MIN_BYTES {
        return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
    }

    let search_start = source
        .len()
        .saturating_sub(EOCD_MIN_BYTES + u16::MAX as usize);
    let eocd_offset = (search_start..=source.len() - EOCD_MIN_BYTES)
        .rev()
        .find(|offset| {
            source[*offset..].starts_with(b"PK\x05\x06")
                && read_le_u16(source, *offset + 20).is_some_and(|comment_bytes| {
                    offset
                        .checked_add(EOCD_MIN_BYTES)
                        .and_then(|value| value.checked_add(comment_bytes as usize))
                        == Some(source.len())
                })
        })
        .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;

    let disk_number = read_le_u16(source, eocd_offset + 4)
        .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    let central_disk = read_le_u16(source, eocd_offset + 6)
        .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    let entries_on_disk = read_le_u16(source, eocd_offset + 8)
        .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    let total_entries = read_le_u16(source, eocd_offset + 10)
        .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    let central_bytes = read_le_u32(source, eocd_offset + 12)
        .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    let central_offset = read_le_u32(source, eocd_offset + 16)
        .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    if disk_number != 0
        || central_disk != 0
        || entries_on_disk != total_entries
        || total_entries == u16::MAX
        || central_bytes == u32::MAX
        || central_offset == u32::MAX
    {
        return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
    }
    if total_entries as usize > MAX_ARCHIVE_ENTRIES {
        return Err(ImportFailure::Rejected(CaseCode::EntryCountLimit));
    }
    let central_start = usize::try_from(central_offset)
        .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    let central_bytes = usize::try_from(central_bytes)
        .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    let central_end = central_start
        .checked_add(central_bytes)
        .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
    if central_end != eocd_offset || central_start > central_end {
        return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
    }

    let mut plans = Vec::with_capacity(total_entries as usize);
    let mut seen_names = HashSet::with_capacity(total_entries as usize);
    let mut total_uncompressed = 0_u64;
    let mut total_compressed = 0_u64;
    let mut offset = central_start;
    for _ in 0..total_entries {
        let fixed_end = offset
            .checked_add(CENTRAL_FIXED_BYTES)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if fixed_end > central_end || !source[offset..].starts_with(b"PK\x01\x02") {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        let flags = read_le_u16(source, offset + 8)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let compression = read_le_u16(source, offset + 10)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        validate_zip_compression(compression)?;
        validate_zip_flags(flags, compression)?;
        let crc32 = read_le_u32(source, offset + 16)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let compressed_size = read_le_u32(source, offset + 20)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let uncompressed_size = read_le_u32(source, offset + 24)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let name_bytes = read_le_u16(source, offset + 28)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?
            as usize;
        let extra_bytes = read_le_u16(source, offset + 30)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?
            as usize;
        let comment_bytes = read_le_u16(source, offset + 32)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?
            as usize;
        let disk_start = read_le_u16(source, offset + 34)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let local_header_offset = read_le_u32(source, offset + 42)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if compressed_size == u32::MAX
            || uncompressed_size == u32::MAX
            || local_header_offset == u32::MAX
            || disk_start != 0
        {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        if compression == 0 && compressed_size != uncompressed_size {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        if u64::from(uncompressed_size) > MAX_ENTRY_BYTES {
            return Err(ImportFailure::Rejected(CaseCode::EntrySizeLimit));
        }
        total_uncompressed = total_uncompressed
            .checked_add(u64::from(uncompressed_size))
            .ok_or(ImportFailure::Rejected(CaseCode::TotalSizeLimit))?;
        total_compressed = total_compressed
            .checked_add(u64::from(compressed_size))
            .ok_or(ImportFailure::Rejected(CaseCode::CompressionRatioLimit))?;
        if total_uncompressed > MAX_TOTAL_BYTES {
            return Err(ImportFailure::Rejected(CaseCode::TotalSizeLimit));
        }
        if exceeds_compression_ratio(u64::from(uncompressed_size), u64::from(compressed_size)) {
            return Err(ImportFailure::Rejected(CaseCode::CompressionRatioLimit));
        }

        let name_end = fixed_end
            .checked_add(name_bytes)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let extra_end = name_end
            .checked_add(extra_bytes)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let entry_end = extra_end
            .checked_add(comment_bytes)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if entry_end > central_end {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        validate_zip_extra(&source[name_end..extra_end])?;
        let name = source[fixed_end..name_end].to_vec();
        if !seen_names.insert(name.clone()) {
            return Err(ImportFailure::Rejected(CaseCode::DuplicatePath));
        }
        plans.push(ZipCentralPlan {
            central_header_offset: offset,
            raw_name: name,
            flags,
            compression,
            crc32,
            compressed_size,
            uncompressed_size,
            disk_start,
            local_header_offset,
            local_data_start: 0,
            local_data_end: 0,
        });
        offset = entry_end;
    }
    if offset != central_end {
        return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
    }
    if exceeds_compression_ratio(total_uncompressed, total_compressed) {
        return Err(ImportFailure::Rejected(CaseCode::CompressionRatioLimit));
    }
    validate_local_zip_headers(source, central_start, &mut plans)?;
    Ok(plans)
}

fn validate_zip_compression(compression: u16) -> Result<(), ImportFailure> {
    if compression == 0 || compression == 8 {
        Ok(())
    } else {
        Err(ImportFailure::Rejected(CaseCode::UnsupportedCompression))
    }
}

fn validate_zip_flags(flags: u16, compression: u16) -> Result<(), ImportFailure> {
    const ENCRYPTED: u16 = 1 << 0;
    const DEFLATE_OPTIONS: u16 = (1 << 1) | (1 << 2);
    const DATA_DESCRIPTOR: u16 = 1 << 3;
    const STRONG_ENCRYPTION: u16 = 1 << 6;
    const UTF8: u16 = 1 << 11;
    const CENTRAL_DIRECTORY_ENCRYPTION: u16 = 1 << 13;
    const ALLOWED: u16 = DEFLATE_OPTIONS | UTF8;

    if flags & (ENCRYPTED | STRONG_ENCRYPTION | CENTRAL_DIRECTORY_ENCRYPTION) != 0 {
        return Err(ImportFailure::Rejected(CaseCode::UnsafeEntryType));
    }
    if flags & DATA_DESCRIPTOR != 0
        || flags & !ALLOWED != 0
        || (compression == 0 && flags & DEFLATE_OPTIONS != 0)
    {
        return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
    }
    Ok(())
}

fn validate_zip_extra(extra: &[u8]) -> Result<(), ImportFailure> {
    const ZIP64_EXTRA_ID: u16 = 0x0001;

    let mut offset = 0_usize;
    while offset < extra.len() {
        let header_end = offset
            .checked_add(4)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if header_end > extra.len() {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        let kind = read_le_u16(extra, offset)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let payload_bytes = read_le_u16(extra, offset + 2)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?
            as usize;
        let field_end = header_end
            .checked_add(payload_bytes)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if field_end > extra.len() || kind == ZIP64_EXTRA_ID {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        offset = field_end;
    }
    Ok(())
}

fn validate_local_zip_headers(
    source: &[u8],
    central_start: usize,
    plans: &mut [ZipCentralPlan],
) -> Result<(), ImportFailure> {
    const LOCAL_FIXED_BYTES: usize = 30;

    if !plans.is_empty() && plans.iter().map(|plan| plan.local_header_offset).min() != Some(0) {
        return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
    }

    for plan in plans.iter_mut() {
        if plan.disk_start != 0 {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        let local_start = usize::try_from(plan.local_header_offset)
            .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let fixed_end = local_start
            .checked_add(LOCAL_FIXED_BYTES)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if fixed_end > central_start
            || source.get(local_start..local_start + 4) != Some(b"PK\x03\x04")
        {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }

        let flags = read_le_u16(source, local_start + 6)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let compression = read_le_u16(source, local_start + 8)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        validate_zip_compression(compression)?;
        validate_zip_flags(flags, compression)?;
        let crc32 = read_le_u32(source, local_start + 14)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let compressed_size = read_le_u32(source, local_start + 18)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let uncompressed_size = read_le_u32(source, local_start + 22)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let name_bytes = read_le_u16(source, local_start + 26)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?
            as usize;
        let extra_bytes = read_le_u16(source, local_start + 28)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?
            as usize;
        if compressed_size == u32::MAX || uncompressed_size == u32::MAX {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        if flags != plan.flags
            || compression != plan.compression
            || crc32 != plan.crc32
            || compressed_size != plan.compressed_size
            || uncompressed_size != plan.uncompressed_size
        {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }

        let name_end = fixed_end
            .checked_add(name_bytes)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let data_start = name_end
            .checked_add(extra_bytes)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if data_start > central_start
            || source.get(fixed_end..name_end) != Some(plan.raw_name.as_slice())
        {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        validate_zip_extra(&source[name_end..data_start])?;
        let compressed_bytes = usize::try_from(compressed_size)
            .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let data_end = data_start
            .checked_add(compressed_bytes)
            .ok_or(ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if data_end > central_start {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
        plan.local_data_start = data_start;
        plan.local_data_end = data_end;
    }

    for (index, plan) in plans.iter().enumerate() {
        let plan_start = usize::try_from(plan.local_header_offset)
            .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        for other in &plans[index + 1..] {
            let other_start = usize::try_from(other.local_header_offset)
                .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
            if plan_start < other.local_data_end && other_start < plan.local_data_end {
                return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
            }
        }
    }
    Ok(())
}

fn crosscheck_zip_archive(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    plans: &[ZipCentralPlan],
) -> Result<(), ImportFailure> {
    for (index, plan) in plans.iter().enumerate() {
        let entry = archive
            .by_index(index)
            .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let expected_compression = match plan.compression {
            0 => CompressionMethod::Stored,
            8 => CompressionMethod::Deflated,
            _ => return Err(ImportFailure::Rejected(CaseCode::UnsupportedCompression)),
        };
        let expected_data_start = u64::try_from(plan.local_data_start)
            .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        let expected_central_start = u64::try_from(plan.central_header_offset)
            .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;
        if entry.name_raw() != plan.raw_name.as_slice()
            || entry.compression() != expected_compression
            || entry.crc32() != plan.crc32
            || entry.compressed_size() != u64::from(plan.compressed_size)
            || entry.size() != u64::from(plan.uncompressed_size)
            || entry.header_start() != u64::from(plan.local_header_offset)
            || entry.data_start() != Some(expected_data_start)
            || entry.central_header_start() != expected_central_start
        {
            return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
        }
    }
    Ok(())
}

fn read_le_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset.checked_add(2)?)?
        .try_into()
        .ok()
        .map(u16::from_le_bytes)
}

fn read_le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset.checked_add(4)?)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn preflight_archive(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
) -> Result<Vec<EntryPlan>, ImportFailure> {
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(ImportFailure::Rejected(CaseCode::EntryCountLimit));
    }

    let mut plans = Vec::with_capacity(archive.len());
    let mut collision_keys: HashSet<String> = HashSet::with_capacity(archive.len());
    let mut total_declared = 0_u64;
    let mut total_compressed = 0_u64;
    let mut manifest_count = 0_usize;
    let mut png_count = 0_usize;

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?;

        if entry.encrypted() {
            return Err(ImportFailure::Rejected(CaseCode::UnsafeEntryType));
        }
        if entry.is_symlink() || !entry.is_file() || has_non_regular_unix_type(entry.unix_mode()) {
            return Err(ImportFailure::Rejected(CaseCode::UnsafeEntryType));
        }
        match entry.compression() {
            CompressionMethod::Stored | CompressionMethod::Deflated => {}
            _ => {
                return Err(ImportFailure::Rejected(CaseCode::UnsupportedCompression));
            }
        }
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(ImportFailure::Rejected(CaseCode::EntrySizeLimit));
        }
        total_declared = total_declared
            .checked_add(entry.size())
            .ok_or(ImportFailure::Rejected(CaseCode::TotalSizeLimit))?;
        total_compressed = total_compressed
            .checked_add(entry.compressed_size())
            .ok_or(ImportFailure::Rejected(CaseCode::CompressionRatioLimit))?;
        if total_declared > MAX_TOTAL_BYTES {
            return Err(ImportFailure::Rejected(CaseCode::TotalSizeLimit));
        }
        if exceeds_compression_ratio(entry.size(), entry.compressed_size()) {
            return Err(ImportFailure::Rejected(CaseCode::CompressionRatioLimit));
        }

        let raw_name = entry.name_raw();
        let logical_path = std::str::from_utf8(raw_name)
            .map_err(|_| ImportFailure::Rejected(CaseCode::UnsafePath))?;
        validate_logical_path(logical_path)?;
        let collision_key = portable_collision_key(logical_path);
        if collision_keys.contains(&collision_key)
            || collision_keys.iter().any(|existing| {
                is_path_prefix(existing, &collision_key) || is_path_prefix(&collision_key, existing)
            })
        {
            return Err(ImportFailure::Rejected(CaseCode::DuplicatePath));
        }
        collision_keys.insert(collision_key);
        let kind = classify_logical_path(logical_path)?;
        match kind {
            EntryKind::Manifest => manifest_count += 1,
            EntryKind::Png => png_count += 1,
            EntryKind::Script => {}
        }
        plans.push(EntryPlan {
            index,
            logical_path: logical_path.to_owned(),
            declared_size: entry.size(),
            kind,
        });
    }
    if exceeds_compression_ratio(total_declared, total_compressed) {
        return Err(ImportFailure::Rejected(CaseCode::CompressionRatioLimit));
    }
    if manifest_count != 1 || png_count == 0 {
        return Err(ImportFailure::Rejected(CaseCode::UnsupportedFormat));
    }
    // Manual central/local validation has already bounded every offset and size. Keep the
    // dependency helper only as a final defense-in-depth parser cross-check.
    if archive
        .has_overlapping_files()
        .map_err(|_| ImportFailure::Rejected(CaseCode::ArchiveMalformed))?
    {
        return Err(ImportFailure::Rejected(CaseCode::ArchiveMalformed));
    }
    Ok(plans)
}

fn has_non_regular_unix_type(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| {
        let file_type = mode & 0o170_000;
        file_type != 0 && file_type != 0o100_000
    })
}

fn exceeds_compression_ratio(uncompressed: u64, compressed: u64) -> bool {
    if uncompressed == 0 {
        return false;
    }
    compressed == 0 || uncompressed > compressed.saturating_mul(MAX_COMPRESSION_RATIO)
}

fn portable_collision_key(path: &str) -> String {
    path.chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .nfc()
        .collect::<String>()
}

fn validate_logical_path(path: &str) -> Result<(), ImportFailure> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
        || path.nfc().collect::<String>() != path
    {
        return Err(ImportFailure::Rejected(CaseCode::UnsafePath));
    }

    let components = path.split('/').collect::<Vec<_>>();
    if components.is_empty() || components.len() > MAX_PATH_DEPTH {
        return Err(ImportFailure::Rejected(CaseCode::UnsafePath));
    }
    for component in components {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > MAX_COMPONENT_BYTES
            || component.ends_with('.')
            || component.ends_with(' ')
            || is_windows_reserved_component(component)
        {
            return Err(ImportFailure::Rejected(CaseCode::UnsafePath));
        }
    }
    Ok(())
}

fn is_windows_reserved_component(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || upper.strip_prefix("COM").is_some_and(|suffix| {
        matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    }) || upper.strip_prefix("LPT").is_some_and(|suffix| {
        matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    })
}

fn is_path_prefix(parent: &str, child: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|remainder| remainder.starts_with('/'))
}

fn classify_logical_path(path: &str) -> Result<EntryKind, ImportFailure> {
    if path == "manifest.json" {
        return Ok(EntryKind::Manifest);
    }
    if let Some(rest) = path.strip_prefix("assets/") {
        if !rest.is_empty() && !rest.contains('/') && rest.ends_with(".png") {
            return Ok(EntryKind::Png);
        }
    }
    if let Some(rest) = path.strip_prefix("scripts/") {
        if !rest.is_empty()
            && !rest.contains('/')
            && (rest.ends_with(".js") || rest.ends_with(".lua"))
        {
            return Ok(EntryKind::Script);
        }
    }
    Err(ImportFailure::Rejected(CaseCode::UnsupportedFileType))
}

fn import_direct_png(
    source: &[u8],
    staging: &Path,
    published: &Path,
) -> Result<ValidDirectPngEvidence, ImportFailure> {
    let info = validate_png(source)?;
    fs::create_dir(staging).map_err(|_| ImportFailure::Harness(ProbeErrorCode::StagingFailure))?;
    let hash = sha256_hex(source);
    let object_name = format!("object-00-{}.png", &hash[..16]);
    write_new_file(&staging.join(&object_name), source)?;
    let entries = vec![StoredIndexEntry {
        logical_path: "direct.png".to_owned(),
        object_name,
        sha256: hash.clone(),
        bytes: source.len() as u64,
        disposition: "validated-data".to_owned(),
    }];
    let index = StoredIndex {
        version: 1,
        entries,
    };
    let index_bytes = serde_json::to_vec(&index)
        .map_err(|_| ImportFailure::Harness(ProbeErrorCode::InternalState))?;
    if index_bytes.len() > MAX_INDEX_BYTES {
        return Err(ImportFailure::Harness(ProbeErrorCode::InternalState));
    }
    write_new_file(&staging.join("index.json"), &index_bytes)?;
    atomic_publish(staging, published)?;
    verify_reopened_published_index(published, &index)?;
    Ok(ValidDirectPngEvidence {
        source_sha256: hash,
        source_bytes: source.len(),
        width: info.width,
        height: info.height,
        atomic_publish: true,
        reopened_hash_verified: true,
    })
}

fn validate_png(bytes: &[u8]) -> Result<PngInfo, ImportFailure> {
    let info = validate_png_structure(bytes)?;
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    let limits = png::Limits {
        bytes: MAX_PNG_DECODE_BYTES,
    };
    decoder.set_limits(limits);
    decoder.ignore_checksums(false);
    let mut reader = decoder
        .read_info()
        .map_err(|_| ImportFailure::Rejected(CaseCode::PngMalformed))?;
    if reader.info().width != info.width || reader.info().height != info.height {
        return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
    }
    let output_size = reader
        .output_buffer_size()
        .ok_or(ImportFailure::Rejected(CaseCode::PngMalformed))?;
    if output_size > MAX_PNG_DECODE_BYTES {
        return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
    }
    let mut output = vec![0_u8; output_size];
    reader
        .next_frame(&mut output)
        .map_err(|_| ImportFailure::Rejected(CaseCode::PngMalformed))?;
    reader
        .finish()
        .map_err(|_| ImportFailure::Rejected(CaseCode::PngMalformed))?;
    Ok(info)
}

fn validate_png_structure(bytes: &[u8]) -> Result<PngInfo, ImportFailure> {
    if bytes.len() > MAX_PNG_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
    }
    let mut offset = 8_usize;
    let mut chunk_count = 0_usize;
    let mut ihdr = None;
    let mut idat_seen = false;
    let mut iend_seen = false;

    while offset < bytes.len() {
        chunk_count = chunk_count
            .checked_add(1)
            .ok_or(ImportFailure::Rejected(CaseCode::PngMalformed))?;
        if chunk_count > MAX_PNG_CHUNKS || iend_seen {
            return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
        }
        let header_end = offset
            .checked_add(8)
            .ok_or(ImportFailure::Rejected(CaseCode::PngMalformed))?;
        if header_end > bytes.len() {
            return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
        }
        let length = u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .map_err(|_| ImportFailure::Rejected(CaseCode::PngMalformed))?,
        ) as usize;
        if length > MAX_PNG_CHUNK_PAYLOAD {
            return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
        }
        let chunk_type: [u8; 4] = bytes[offset + 4..offset + 8]
            .try_into()
            .map_err(|_| ImportFailure::Rejected(CaseCode::PngMalformed))?;
        if !chunk_type.iter().all(u8::is_ascii_alphabetic)
            || chunk_type[2] & 0x20 != 0
            // This disposable proof accepts only the critical image structure. Product card
            // metadata policy remains deliberately unfixed, so ancillary chunks fail closed.
            || chunk_type[0] & 0x20 != 0
        {
            return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
        }
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(length)
            .ok_or(ImportFailure::Rejected(CaseCode::PngMalformed))?;
        let chunk_end = data_end
            .checked_add(4)
            .ok_or(ImportFailure::Rejected(CaseCode::PngMalformed))?;
        if chunk_end > bytes.len() {
            return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
        }
        let expected_crc = u32::from_be_bytes(
            bytes[data_end..chunk_end]
                .try_into()
                .map_err(|_| ImportFailure::Rejected(CaseCode::PngMalformed))?,
        );
        let mut crc = Crc32::new();
        crc.update(&chunk_type);
        crc.update(&bytes[data_start..data_end]);
        if crc.finalize() != expected_crc {
            return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
        }

        match &chunk_type {
            b"IHDR" => {
                if chunk_count != 1 || ihdr.is_some() || length != 13 {
                    return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
                }
                let width = u32::from_be_bytes(
                    bytes[data_start..data_start + 4]
                        .try_into()
                        .map_err(|_| ImportFailure::Rejected(CaseCode::PngMalformed))?,
                );
                let height = u32::from_be_bytes(
                    bytes[data_start + 4..data_start + 8]
                        .try_into()
                        .map_err(|_| ImportFailure::Rejected(CaseCode::PngMalformed))?,
                );
                let pixels = u64::from(width)
                    .checked_mul(u64::from(height))
                    .ok_or(ImportFailure::Rejected(CaseCode::PngMalformed))?;
                if width == 0
                    || height == 0
                    || width > MAX_PNG_WIDTH
                    || height > MAX_PNG_HEIGHT
                    || pixels > MAX_PNG_PIXELS
                {
                    return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
                }
                ihdr = Some(PngInfo { width, height });
            }
            b"IDAT" => {
                if ihdr.is_none() {
                    return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
                }
                idat_seen = true;
            }
            b"IEND" => {
                if length != 0 || !idat_seen || ihdr.is_none() {
                    return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
                }
                iend_seen = true;
            }
            b"PLTE" => {
                if ihdr.is_none() || idat_seen {
                    return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
                }
            }
            _ if chunk_type[0] & 0x20 == 0 => {
                return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
            }
            _ => {}
        }
        offset = chunk_end;
    }
    if offset != bytes.len() || !iend_seen || !idat_seen {
        return Err(ImportFailure::Rejected(CaseCode::PngMalformed));
    }
    ihdr.ok_or(ImportFailure::Rejected(CaseCode::PngMalformed))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), ImportFailure> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| ImportFailure::Harness(ProbeErrorCode::StagingFailure))?;
    file.write_all(bytes)
        .map_err(|_| ImportFailure::Harness(ProbeErrorCode::StagingFailure))
}

fn atomic_publish(staging: &Path, published: &Path) -> Result<(), ImportFailure> {
    match fs::symlink_metadata(published) {
        Ok(_) => return Err(ImportFailure::Harness(ProbeErrorCode::PublishConflict)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(ImportFailure::Harness(ProbeErrorCode::PublishFailure)),
    }
    fs::rename(staging, published)
        .map_err(|_| ImportFailure::Harness(ProbeErrorCode::PublishFailure))
}

fn verify_reopened_published_index(
    published: &Path,
    expected: &StoredIndex,
) -> Result<(), ImportFailure> {
    let index_bytes = read_published_file_bounded(&published.join("index.json"), MAX_INDEX_BYTES)?;
    let reopened: StoredIndex = serde_json::from_slice(&index_bytes)
        .map_err(|_| ImportFailure::Harness(ProbeErrorCode::PublishFailure))?;
    if &reopened != expected {
        return Err(ImportFailure::Harness(ProbeErrorCode::PublishFailure));
    }
    for entry in reopened.entries {
        if !is_generated_object_name(&entry.object_name) {
            return Err(ImportFailure::Harness(ProbeErrorCode::PublishFailure));
        }
        let bytes = read_published_file_bounded(
            &published.join(&entry.object_name),
            MAX_ENTRY_BYTES as usize,
        )?;
        if bytes.len() as u64 != entry.bytes || sha256_hex(&bytes) != entry.sha256 {
            return Err(ImportFailure::Harness(ProbeErrorCode::PublishFailure));
        }
    }
    Ok(())
}

fn read_published_file_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, ImportFailure> {
    let file =
        File::open(path).map_err(|_| ImportFailure::Harness(ProbeErrorCode::PublishFailure))?;
    let read_ceiling = u64::try_from(maximum)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(ImportFailure::Harness(ProbeErrorCode::PublishFailure))?;
    let mut bytes = Vec::with_capacity(maximum.min(STREAM_BUFFER_BYTES));
    file.take(read_ceiling)
        .read_to_end(&mut bytes)
        .map_err(|_| ImportFailure::Harness(ProbeErrorCode::PublishFailure))?;
    if bytes.len() > maximum {
        return Err(ImportFailure::Harness(ProbeErrorCode::PublishFailure));
    }
    Ok(bytes)
}

fn is_generated_object_name(name: &str) -> bool {
    if name.contains('/') || name.contains('\\') || !name.starts_with("object-") {
        return false;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some(parts) => parts,
        None => return false,
    };
    if extension != "bin" && extension != "png" {
        return false;
    }
    let mut parts = stem.split('-');
    matches!(parts.next(), Some("object"))
        && parts.next().is_some_and(|ordinal| {
            ordinal.len() == 2 && ordinal.bytes().all(|byte| byte.is_ascii_digit())
        })
        && parts.next().is_some_and(|hash| {
            hash.len() == 16
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        && parts.next().is_none()
}

fn verify_case_is_clean(
    root: &Path,
    staging: &Path,
    published: &Path,
    sentinel: &Path,
) -> Result<(), ProbeError> {
    if staging.exists() || published.exists() || !sentinel_matches(sentinel)? {
        return Err(internal_state_error());
    }
    let mut entries = fs::read_dir(root).map_err(|_| internal_state_error())?;
    if entries
        .next()
        .transpose()
        .map_err(|_| internal_state_error())?
        .is_some()
    {
        return Err(internal_state_error());
    }
    Ok(())
}

fn sentinel_matches(sentinel: &Path) -> Result<bool, ProbeError> {
    let file = File::open(sentinel).map_err(|_| internal_state_error())?;
    let mut bytes = Vec::with_capacity(SENTINEL_CONTENT.len() + 1);
    file.take((SENTINEL_CONTENT.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| internal_state_error())?;
    Ok(bytes == SENTINEL_CONTENT)
}

fn cleanup_case_paths(staging: &Path, published: &Path) -> Result<(), ()> {
    remove_exact_path(staging)?;
    remove_exact_path(published)
}

fn cleanup_owned_paths(root: &Path, sentinel: &Path) -> Result<(), ()> {
    remove_exact_path(root)?;
    remove_exact_path(sentinel)
}

fn remove_exact_path(path: &Path) -> Result<(), ()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|_| ())
    } else {
        fs::remove_file(path).map_err(|_| ())
    }
}

fn cleanup_error(cleanup_pending: bool) -> ProbeError {
    ProbeError {
        cleanup_pending,
        ..probe_error(ProbeErrorCode::CleanupFailure)
    }
}

fn generate_fixture(case_id: &str) -> Result<Vec<u8>, ProbeError> {
    let png = make_valid_png()?;
    let single = |name: &str, data: Vec<u8>| {
        build_zip(vec![file_entry(name, data, CompressionMethod::Stored)])
    };

    match case_id {
        "valid-archive" => build_zip(vec![
            file_entry(
                "manifest.json",
                br#"{"version":1,"name":"self-authored-fixture"}"#.to_vec(),
                CompressionMethod::Stored,
            ),
            file_entry("assets/avatar.png", png, CompressionMethod::Stored),
            file_entry(
                "scripts/hook.js",
                b"throw new Error('this inert fixture must never execute');".to_vec(),
                CompressionMethod::Stored,
            ),
            file_entry(
                "scripts/hook.lua",
                b"error('this inert fixture must never execute')".to_vec(),
                CompressionMethod::Stored,
            ),
        ]),
        "valid-direct-png" => Ok(png),
        "source-too-large" => Ok(vec![b'X'; MAX_SOURCE_BYTES + 1]),
        "unsupported-source" => Ok(b"self-authored unsupported bytes".to_vec()),
        "parent-traversal" => single("../manifest.json", b"{}".to_vec()),
        "nested-parent-traversal" => single("assets/x/../../manifest.json", b"{}".to_vec()),
        "absolute-posix-path" => single("/manifest.json", b"{}".to_vec()),
        "windows-drive-path" => single("C:/manifest.json", b"{}".to_vec()),
        "backslash-path" => single("assets\\escape.png", png),
        "reserved-device-path" => single("assets/CON.png", png),
        "non-nfc-path" => single("assets/e\u{301}.png", png),
        "exact-duplicate-path" => build_zip(vec![
            file_entry("manifest.json", b"{}".to_vec(), CompressionMethod::Stored),
            file_entry("manifest.jsox", b"{}".to_vec(), CompressionMethod::Stored),
        ])
        .and_then(|mut archive| {
            replace_all_equal_length(&mut archive, b"manifest.jsox", b"manifest.json")?;
            Ok(archive)
        }),
        "case-collision" => build_zip(vec![
            file_entry("assets/Card.png", png.clone(), CompressionMethod::Stored),
            file_entry("assets/card.png", png, CompressionMethod::Stored),
        ]),
        "prefix-conflict" => build_zip(vec![
            file_entry("scripts/hook.js", b"0".to_vec(), CompressionMethod::Stored),
            file_entry(
                "scripts/hook.js/child",
                b"0".to_vec(),
                CompressionMethod::Stored,
            ),
        ]),
        "symlink-entry" => build_zip(vec![FixtureEntry {
            name: "assets/link.png".to_owned(),
            data: Vec::new(),
            compression: CompressionMethod::Stored,
            symlink_target: Some("../../outside".to_owned()),
        }]),
        "unsupported-compression" => {
            let mut archive = single("manifest.json", b"{}".to_vec())?;
            patch_compression_method(&mut archive, 99)?;
            Ok(archive)
        }
        "too-many-entries" => build_zip(
            (0..=MAX_ARCHIVE_ENTRIES)
                .map(|index| {
                    file_entry(
                        &format!("assets/{index:02}.png"),
                        Vec::new(),
                        CompressionMethod::Stored,
                    )
                })
                .collect(),
        ),
        "oversized-entry" => single("scripts/big.js", vec![b'x'; MAX_ENTRY_BYTES as usize + 1]),
        "oversized-total" => build_zip(
            (0..3)
                .map(|index| {
                    file_entry(
                        &format!("scripts/part-{index}.js"),
                        vec![b'x'; 400_000],
                        CompressionMethod::Stored,
                    )
                })
                .collect(),
        ),
        "high-compression-ratio" => build_zip(vec![file_entry(
            "scripts/compressible.js",
            vec![0; 262_144],
            CompressionMethod::Deflated,
        )]),
        "malformed-archive" => Ok(b"PK\x03\x04truncated-self-authored-archive".to_vec()),
        "png-bad-crc" => {
            let mut malformed = png;
            corrupt_first_idat_crc(&mut malformed)?;
            build_png_archive(malformed)
        }
        "png-truncated-chunk" => {
            let mut malformed = png;
            malformed.truncate(malformed.len().saturating_sub(6));
            build_png_archive(malformed)
        }
        "png-trailing-bytes" => {
            let mut malformed = png;
            malformed.extend_from_slice(b"trailing");
            build_png_archive(malformed)
        }
        "png-oversized-dimensions" => {
            let mut malformed = png;
            patch_png_dimensions(&mut malformed, MAX_PNG_WIDTH + 1, 1)?;
            build_png_archive(malformed)
        }
        "unsupported-file-type" => single("notes.txt", b"not allowed".to_vec()),
        _ => Err(internal_state_error()),
    }
}

fn file_entry(name: &str, data: Vec<u8>, compression: CompressionMethod) -> FixtureEntry {
    FixtureEntry {
        name: name.to_owned(),
        data,
        compression,
        symlink_target: None,
    }
}

fn build_png_archive(png: Vec<u8>) -> Result<Vec<u8>, ProbeError> {
    build_zip(vec![
        file_entry(
            "manifest.json",
            br#"{"version":1}"#.to_vec(),
            CompressionMethod::Stored,
        ),
        file_entry("assets/avatar.png", png, CompressionMethod::Stored),
    ])
}

fn build_zip(entries: Vec<FixtureEntry>) -> Result<Vec<u8>, ProbeError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for entry in entries {
        let options = SimpleFileOptions::default()
            .system(System::Unix)
            .compression_method(entry.compression)
            .unix_permissions(0o600);
        if let Some(target) = entry.symlink_target {
            writer
                .add_symlink(entry.name, target, options)
                .map_err(|_| internal_state_error())?;
        } else {
            writer
                .start_file(entry.name, options)
                .map_err(|_| internal_state_error())?;
            writer
                .write_all(&entry.data)
                .map_err(|_| internal_state_error())?;
        }
    }
    writer
        .finish()
        .map(Cursor::into_inner)
        .map_err(|_| internal_state_error())
}

fn make_valid_png() -> Result<Vec<u8>, ProbeError> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|_| internal_state_error())?;
        writer
            .write_image_data(&[0x24, 0x68, 0xac, 0xff])
            .map_err(|_| internal_state_error())?;
        writer.finish().map_err(|_| internal_state_error())?;
    }
    Ok(bytes)
}

fn patch_compression_method(bytes: &mut [u8], method: u16) -> Result<(), ProbeError> {
    let encoded = method.to_le_bytes();
    let mut patched_local = false;
    let mut patched_central = false;
    for offset in 0..bytes.len().saturating_sub(4) {
        if bytes[offset..].starts_with(b"PK\x03\x04") && offset + 10 <= bytes.len() {
            bytes[offset + 8..offset + 10].copy_from_slice(&encoded);
            patched_local = true;
        } else if bytes[offset..].starts_with(b"PK\x01\x02") && offset + 12 <= bytes.len() {
            bytes[offset + 10..offset + 12].copy_from_slice(&encoded);
            patched_central = true;
        }
    }
    if patched_local && patched_central {
        Ok(())
    } else {
        Err(internal_state_error())
    }
}

fn replace_all_equal_length(
    bytes: &mut [u8],
    needle: &[u8],
    replacement: &[u8],
) -> Result<(), ProbeError> {
    if needle.is_empty() || needle.len() != replacement.len() {
        return Err(internal_state_error());
    }
    let mut replacements = 0_usize;
    let mut offset = 0_usize;
    while offset + needle.len() <= bytes.len() {
        if &bytes[offset..offset + needle.len()] == needle {
            bytes[offset..offset + needle.len()].copy_from_slice(replacement);
            replacements += 1;
            offset += needle.len();
        } else {
            offset += 1;
        }
    }
    if replacements == 2 {
        Ok(())
    } else {
        Err(internal_state_error())
    }
}

fn corrupt_first_idat_crc(bytes: &mut [u8]) -> Result<(), ProbeError> {
    let (data_end, chunk_end) = find_png_chunk_bounds(bytes, b"IDAT")?;
    if data_end >= chunk_end {
        return Err(internal_state_error());
    }
    bytes[data_end] ^= 0x01;
    Ok(())
}

fn patch_png_dimensions(bytes: &mut [u8], width: u32, height: u32) -> Result<(), ProbeError> {
    let (data_end, chunk_end) = find_png_chunk_bounds(bytes, b"IHDR")?;
    let data_start = data_end.checked_sub(13).ok_or_else(internal_state_error)?;
    if chunk_end - data_end != 4 || data_start + 8 > data_end {
        return Err(internal_state_error());
    }
    bytes[data_start..data_start + 4].copy_from_slice(&width.to_be_bytes());
    bytes[data_start + 4..data_start + 8].copy_from_slice(&height.to_be_bytes());
    let type_start = data_start.checked_sub(4).ok_or_else(internal_state_error)?;
    let mut crc = Crc32::new();
    crc.update(&bytes[type_start..data_end]);
    bytes[data_end..chunk_end].copy_from_slice(&crc.finalize().to_be_bytes());
    Ok(())
}

fn find_png_chunk_bounds(bytes: &[u8], wanted: &[u8; 4]) -> Result<(usize, usize), ProbeError> {
    let mut offset = 8_usize;
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .map_err(|_| internal_state_error())?,
        ) as usize;
        let data_end = offset
            .checked_add(8)
            .and_then(|value| value.checked_add(length))
            .ok_or_else(internal_state_error)?;
        let chunk_end = data_end.checked_add(4).ok_or_else(internal_state_error)?;
        if chunk_end > bytes.len() {
            return Err(internal_state_error());
        }
        if &bytes[offset + 4..offset + 8] == wanted {
            return Ok((data_end, chunk_end));
        }
        offset = chunk_end;
    }
    Err(internal_state_error())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn fixture_catalog_is_exact_and_hash_pinned() {
        let catalog = parse_and_validate_catalog().expect("catalog should validate");
        assert_eq!(sha256_hex(CATALOG_JSON.as_bytes()), CATALOG_SHA256);
        assert_eq!(catalog.cases.len(), EXPECTED_CASES.len());
    }

    #[test]
    fn every_catalog_case_matches_its_expected_outcome() {
        let directory = tempdir().expect("temp directory");
        for (ordinal, expected) in EXPECTED_CASES.into_iter().enumerate() {
            let source = generate_fixture(expected.case_id)
                .unwrap_or_else(|_| panic!("{} fixture generation failed", expected.case_id));
            let staging = directory.path().join(format!("{ordinal}.staging"));
            let published = directory.path().join(format!("{ordinal}.published"));
            let actual = match import_source(&source, &staging, &published) {
                Ok(_) => (CaseOutcome::Accepted, None),
                Err(ImportFailure::Rejected(code)) => (CaseOutcome::Rejected, Some(code)),
                Err(ImportFailure::Harness(code)) => {
                    panic!("{} had a {code:?} harness failure", expected.case_id)
                }
            };
            assert_eq!(
                actual,
                (expected.expected_outcome, expected.expected_code),
                "{}",
                expected.case_id
            );
            cleanup_case_paths(&staging, &published).expect("case cleanup");
        }
    }

    #[test]
    fn path_policy_rejects_cross_platform_escape_and_alias_forms() {
        for path in [
            "../x",
            "a/../x",
            "/x",
            "C:/x",
            "a\\x",
            "assets/CON.png",
            "assets/NUL.txt",
            "assets/CLOCK$.png",
            "assets/CONIN$.png",
            "assets/CONOUT$.png",
            "assets/COM¹.png",
            "assets/COM².png",
            "assets/COM³.png",
            "assets/LPT¹.png",
            "assets/LPT².png",
            "assets/LPT³.png",
            "assets/name. ",
            "assets/e\u{301}.png",
            "assets//x.png",
        ] {
            assert!(matches!(
                validate_logical_path(path),
                Err(ImportFailure::Rejected(CaseCode::UnsafePath))
            ));
        }
        assert!(validate_logical_path("assets/é.png").is_ok());
    }

    #[test]
    fn portable_collision_key_rejects_unicode_caseless_aliases() {
        for (left, right) in [
            ("assets/Σ.png", "assets/ς.png"),
            ("scripts/ß.js", "scripts/ss.js"),
        ] {
            assert_eq!(portable_collision_key(left), portable_collision_key(right));
            let source = build_zip(vec![
                file_entry(left, Vec::new(), CompressionMethod::Stored),
                file_entry(right, Vec::new(), CompressionMethod::Stored),
            ])
            .expect("collision ZIP");
            let plans = preflight_zip_container(&source).expect("container preflight");
            let mut archive = ZipArchive::new(Cursor::new(source.as_slice())).expect("ZIP parser");
            crosscheck_zip_archive(&mut archive, &plans).expect("ZIP cross-check");
            assert!(matches!(
                preflight_archive(&mut archive),
                Err(ImportFailure::Rejected(CaseCode::DuplicatePath))
            ));
        }
    }

    #[test]
    fn duplicate_fixture_contains_two_identical_raw_names() {
        let source = generate_fixture("exact-duplicate-path").expect("duplicate fixture");
        assert!(matches!(
            preflight_zip_container(&source),
            Err(ImportFailure::Rejected(CaseCode::DuplicatePath))
        ));
    }

    #[test]
    fn source_detection_does_not_treat_a_data_descriptor_as_a_zip_start() {
        assert!(is_zip_signature(b"PK\x03\x04"));
        assert!(is_zip_signature(b"PK\x05\x06"));
        assert!(!is_zip_signature(b"PK\x07\x08"));

        let directory = tempdir().expect("temp directory");
        assert!(matches!(
            import_source(
                b"PK\x07\x08",
                &directory.path().join("case.staging"),
                &directory.path().join("case.published"),
            ),
            Err(ImportFailure::Rejected(CaseCode::UnsupportedFormat))
        ));
    }

    #[test]
    fn strict_zip_parser_rejects_central_local_disagreement() {
        let source = one_entry_zip();
        let layout = zip_layout(&source);
        let central = layout.central_entries[0];
        let local = layout.local_headers[0];

        let mut name_mismatch = source.clone();
        name_mismatch[local + 30] ^= 0x20;
        assert_zip_rejected(&name_mismatch, CaseCode::ArchiveMalformed);

        let mut method_mismatch = source.clone();
        write_test_u16(&mut method_mismatch, local + 8, 8);
        assert_zip_rejected(&method_mismatch, CaseCode::ArchiveMalformed);

        let mut crc_mismatch = source.clone();
        let crc = read_le_u32(&crc_mismatch, local + 14).expect("local CRC");
        write_test_u32(&mut crc_mismatch, local + 14, crc ^ 1);
        assert_zip_rejected(&crc_mismatch, CaseCode::ArchiveMalformed);

        let mut size_mismatch = source;
        let size = read_le_u32(&size_mismatch, local + 18).expect("local size");
        write_test_u32(&mut size_mismatch, local + 18, size + 1);
        assert_zip_rejected(&size_mismatch, CaseCode::ArchiveMalformed);

        assert_eq!(read_le_u16(&size_mismatch, central + 10), Some(0));
    }

    #[test]
    fn strict_zip_parser_rejects_flags_disk_and_zip64_metadata() {
        let source = one_entry_zip();
        let layout = zip_layout(&source);
        let central = layout.central_entries[0];
        let local = layout.local_headers[0];

        let mut descriptor = source.clone();
        write_test_u16(&mut descriptor, central + 8, 1 << 3);
        write_test_u16(&mut descriptor, local + 6, 1 << 3);
        assert_zip_rejected(&descriptor, CaseCode::ArchiveMalformed);

        let mut encrypted = source.clone();
        write_test_u16(&mut encrypted, central + 8, 1);
        write_test_u16(&mut encrypted, local + 6, 1);
        assert_zip_rejected(&encrypted, CaseCode::UnsafeEntryType);

        let mut unsupported_flags = source.clone();
        write_test_u16(&mut unsupported_flags, central + 8, 1 << 14);
        write_test_u16(&mut unsupported_flags, local + 6, 1 << 14);
        assert_zip_rejected(&unsupported_flags, CaseCode::ArchiveMalformed);

        let mut disk_start = source.clone();
        write_test_u16(&mut disk_start, central + 34, 1);
        assert_zip_rejected(&disk_start, CaseCode::ArchiveMalformed);

        let mut zip64_sentinel = source;
        write_test_u32(&mut zip64_sentinel, central + 20, u32::MAX);
        assert_zip_rejected(&zip64_sentinel, CaseCode::ArchiveMalformed);
    }

    #[test]
    fn strict_zip_parser_rejects_zip64_and_malformed_extra_fields() {
        let mut central_zip64 = one_entry_zip();
        insert_central_extra(&mut central_zip64, &[0x01, 0x00, 0x00, 0x00]);
        assert_zip_rejected(&central_zip64, CaseCode::ArchiveMalformed);

        let mut local_zip64 = one_entry_zip();
        insert_local_extra(&mut local_zip64, &[0x01, 0x00, 0x00, 0x00]);
        assert_zip_rejected(&local_zip64, CaseCode::ArchiveMalformed);

        let mut malformed_extra = one_entry_zip();
        insert_central_extra(&mut malformed_extra, &[0xaa, 0xbb]);
        assert_zip_rejected(&malformed_extra, CaseCode::ArchiveMalformed);

        let mut malformed_local_extra = one_entry_zip();
        insert_local_extra(&mut malformed_local_extra, &[0xaa, 0xbb]);
        assert_zip_rejected(&malformed_local_extra, CaseCode::ArchiveMalformed);
    }

    #[test]
    fn strict_zip_parser_rejects_prefix_reused_offsets_and_overlapping_ranges() {
        let mut prefixed = one_entry_zip();
        prefix_zip_and_shift_offsets(&mut prefixed, b"self-authored-prefix");
        assert_zip_rejected(&prefixed, CaseCode::ArchiveMalformed);

        let mut reused_offset = build_zip(vec![
            file_entry("scripts/a.js", Vec::new(), CompressionMethod::Stored),
            file_entry("scripts/b.js", Vec::new(), CompressionMethod::Stored),
        ])
        .expect("two-entry ZIP");
        let layout = zip_layout(&reused_offset);
        write_test_u32(
            &mut reused_offset,
            layout.central_entries[1] + 42,
            layout.local_headers[0] as u32,
        );
        assert_zip_rejected(&reused_offset, CaseCode::ArchiveMalformed);

        let mut overlapping = build_zip(vec![
            file_entry("scripts/a.js", b"a".to_vec(), CompressionMethod::Stored),
            file_entry("scripts/b.js", b"b".to_vec(), CompressionMethod::Stored),
        ])
        .expect("two-entry ZIP");
        let layout = zip_layout(&overlapping);
        let first_local = layout.local_headers[0];
        let first_name_bytes = read_le_u16(&overlapping, first_local + 26).expect("local name");
        let first_extra_bytes = read_le_u16(&overlapping, first_local + 28).expect("local extra");
        let first_data = first_local + 30 + first_name_bytes as usize + first_extra_bytes as usize;
        let overlap_size =
            u32::try_from(layout.local_headers[1] - first_data + 1).expect("small overlap fixture");
        for offset in [first_local + 18, first_local + 22] {
            write_test_u32(&mut overlapping, offset, overlap_size);
        }
        for offset in [
            layout.central_entries[0] + 20,
            layout.central_entries[0] + 24,
        ] {
            write_test_u32(&mut overlapping, offset, overlap_size);
        }
        assert_zip_rejected(&overlapping, CaseCode::ArchiveMalformed);
    }

    #[test]
    fn fixture_zip_golden_and_made_by_system_are_platform_independent() {
        let source = generate_fixture("valid-archive").expect("valid archive fixture");
        assert_eq!(source.len(), VALID_ARCHIVE_BYTES);
        assert_eq!(sha256_hex(&source), VALID_ARCHIVE_SHA256);
        let layout = zip_layout(&source);
        assert_eq!(layout.central_entries.len(), 4);
        for central in layout.central_entries {
            assert_eq!(source[central + 5], u8::from(System::Unix));
        }
    }

    #[test]
    fn png_parser_rejects_crc_truncation_trailing_and_dimensions() {
        let valid = make_valid_png().expect("valid png");
        assert_eq!(
            validate_png(&valid).expect("valid png should decode"),
            PngInfo {
                width: 1,
                height: 1
            }
        );

        let mut bad_crc = valid.clone();
        corrupt_first_idat_crc(&mut bad_crc).expect("corrupt crc");
        assert_png_rejected(&bad_crc);

        let mut truncated = valid.clone();
        truncated.truncate(truncated.len() - 6);
        assert_png_rejected(&truncated);

        let mut trailing = valid.clone();
        trailing.push(0);
        assert_png_rejected(&trailing);

        let mut oversized = valid;
        patch_png_dimensions(&mut oversized, MAX_PNG_WIDTH + 1, 1).expect("patch dimensions");
        assert_png_rejected(&oversized);
    }

    #[test]
    fn png_parser_rejects_structural_critical_chunk_regressions() {
        let valid = make_valid_png().expect("valid png");

        let mut bad_signature = valid.clone();
        bad_signature[0] ^= 1;
        assert_png_rejected(&bad_signature);

        let mut false_length = valid.clone();
        false_length[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_png_rejected(&false_length);

        let unknown_critical = insert_chunk_before(&valid, b"IDAT", b"ABCD", b"");
        assert_png_rejected(&unknown_critical);

        let ihdr = png_chunk_range(&valid, b"IHDR");
        let mut duplicate_ihdr = valid.clone();
        let ihdr_bytes = duplicate_ihdr[ihdr.clone()].to_vec();
        duplicate_ihdr.splice(ihdr.end..ihdr.end, ihdr_bytes);
        assert_png_rejected(&duplicate_ihdr);

        let mut missing_ihdr = valid.clone();
        missing_ihdr.drain(ihdr);
        assert_png_rejected(&missing_ihdr);

        let idat = png_chunk_range(&valid, b"IDAT");
        let mut missing_idat = valid.clone();
        missing_idat.drain(idat);
        assert_png_rejected(&missing_idat);

        let iend = png_chunk_range(&valid, b"IEND");
        let mut missing_iend = valid.clone();
        missing_iend.drain(iend.clone());
        assert_png_rejected(&missing_iend);

        let mut duplicate_iend = valid.clone();
        let iend_bytes = duplicate_iend[iend.clone()].to_vec();
        duplicate_iend.extend_from_slice(&iend_bytes);
        assert_png_rejected(&duplicate_iend);

        let mut misordered_iend = valid.clone();
        let iend_bytes = misordered_iend[iend.clone()].to_vec();
        misordered_iend.drain(iend);
        misordered_iend.splice(8..8, iend_bytes);
        assert_png_rejected(&misordered_iend);

        let nonempty_iend = replace_chunk(&valid, b"IEND", b"IEND", b"x");
        assert_png_rejected(&nonempty_iend);
    }

    #[test]
    fn png_parser_rejects_all_ancillary_and_reserved_bit_chunks() {
        let valid = make_valid_png().expect("valid png");
        let reserved_bit = insert_chunk_before(&valid, b"IDAT", b"abca", b"");
        assert_png_rejected(&reserved_bit);

        let wrong_length_gamma = insert_chunk_before(&valid, b"IDAT", b"gAMA", &[0]);
        assert_png_rejected(&wrong_length_gamma);
    }

    #[test]
    fn archives_without_required_shape_are_user_rejections_before_staging() {
        let directory = tempdir().expect("temp directory");
        let empty = build_zip(Vec::new()).expect("empty zip fixture");
        let valid_png = make_valid_png().expect("valid png");
        let missing_manifest = build_zip(vec![file_entry(
            "assets/avatar.png",
            valid_png,
            CompressionMethod::Stored,
        )])
        .expect("missing manifest fixture");
        for source in [empty, missing_manifest] {
            let staging = directory.path().join("case.staging");
            let published = directory.path().join("case.published");
            assert!(matches!(
                import_archive(&source, &staging, &published),
                Err(ImportFailure::Rejected(CaseCode::UnsupportedFormat))
            ));
            assert!(!staging.exists());
            assert!(!published.exists());
        }
    }

    #[test]
    fn successful_probe_cleans_owned_paths_and_preserves_no_sentinel() {
        let directory = tempdir().expect("temp directory");
        run_probe_in_directory(directory.path()).expect("probe should pass");
        assert!(!directory.path().join(PROBE_ROOT_NAME).exists());
        assert!(!directory.path().join(SENTINEL_NAME).exists());
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("read directory")
                .count(),
            0
        );
    }

    #[test]
    fn valid_script_is_quarantined_and_never_executed() {
        let directory = tempdir().expect("temp directory");
        let receipt = run_probe_in_directory(directory.path()).expect("probe should pass");
        assert_eq!(receipt.valid_archive.source_sha256, VALID_ARCHIVE_SHA256);
        assert_eq!(receipt.valid_archive.source_bytes, VALID_ARCHIVE_BYTES);
        assert_eq!(
            receipt.valid_archive.total_uncompressed_bytes,
            VALID_ARCHIVE_TOTAL_BYTES
        );
        assert_eq!(
            receipt.valid_direct_png.source_sha256,
            VALID_DIRECT_PNG_SHA256
        );
        assert_eq!(
            receipt.valid_direct_png.source_bytes,
            VALID_DIRECT_PNG_BYTES
        );
        assert_eq!(receipt.valid_archive.script_entries, 2);
        assert_eq!(receipt.valid_archive.executed_entries, 0);
        assert_eq!(receipt.valid_archive.quarantine, "inert");
        assert!(receipt.defenses.script_execution_disabled);
    }

    #[test]
    fn process_lock_reports_bounded_busy_error() {
        let _guard = PROBE_LOCK.lock().expect("test lock");
        let error = with_process_lock(|| Ok::<(), ProbeError>(()))
            .expect_err("second lock must be rejected");
        assert_eq!(error.code, ProbeErrorCode::ProbeBusy);
        assert!(!error.cleanup_pending);
    }

    #[test]
    fn success_and_every_failure_receipt_are_bounded_and_path_free() {
        let directory = tempdir().expect("temp directory");
        let receipt = run_probe_in_directory(directory.path()).expect("probe should pass");
        let success = serde_json::to_string(&receipt).expect("serialize receipt");
        assert!(success.len() <= MAX_IPC_BYTES);
        assert!(!success.contains(directory.path().to_string_lossy().as_ref()));

        for code in ALL_PROBE_ERROR_CODES {
            let serialized = serde_json::to_string(&ProbeError {
                protocol_version: PROTOCOL_VERSION,
                code,
                cleanup_pending: true,
            })
            .expect("serialize error");
            assert!(serialized.len() <= MAX_IPC_BYTES);
            assert_eq!(serialized.matches(':').count(), 3);
            assert!(!serialized.contains('/'));
            assert!(!serialized.contains('\\'));
        }
    }

    #[test]
    fn serialized_receipt_has_the_frontend_exact_key_schema() {
        let directory = tempdir().expect("temp directory");
        let receipt = run_probe_in_directory(directory.path()).expect("probe should pass");
        let value = serde_json::to_value(&receipt).expect("serialize receipt");
        assert_value_keys(
            &value,
            &[
                "protocolVersion",
                "policyVersion",
                "fixtureCatalogSha256",
                "zipVersion",
                "pngVersion",
                "limits",
                "cases",
                "validArchive",
                "validDirectPng",
                "defenses",
                "cleanupPending",
            ],
        );
        assert_value_keys(
            &value["limits"],
            &[
                "sourceBytes",
                "entryCount",
                "entryBytes",
                "totalUncompressedBytes",
                "compressionRatio",
                "pathBytes",
                "pathComponentBytes",
                "pathDepth",
                "streamBufferBytes",
                "indexBytes",
                "pngBytes",
                "pngDecodedBytes",
                "pngChunks",
                "pngChunkBytes",
                "pngWidth",
                "pngHeight",
                "pngPixels",
                "ipcResponseBytes",
            ],
        );
        assert_value_keys(
            &value["validArchive"],
            &[
                "sourceSha256",
                "sourceBytes",
                "entryCount",
                "totalUncompressedBytes",
                "scriptEntries",
                "executedEntries",
                "quarantine",
                "atomicPublish",
                "reopenedHashVerified",
            ],
        );
        assert_value_keys(
            &value["validDirectPng"],
            &[
                "sourceSha256",
                "sourceBytes",
                "width",
                "height",
                "atomicPublish",
                "reopenedHashVerified",
            ],
        );
        assert_value_keys(
            &value["defenses"],
            &[
                "traversalRejected",
                "collisionRejected",
                "unsafeEntryTypesRejected",
                "sizeLimitsEnforced",
                "compressionRatioEnforced",
                "malformedArchiveRejected",
                "strictPngValidated",
                "unsupportedFilesRejected",
                "outsideSentinelPreserved",
                "stagingCleaned",
                "scriptExecutionDisabled",
            ],
        );
        for case in value["cases"].as_array().expect("case array") {
            assert_value_keys(case, &["caseId", "outcome", "code"]);
        }

        let failure = serde_json::to_value(ProbeError {
            protocol_version: PROTOCOL_VERSION,
            code: ProbeErrorCode::PublishFailure,
            cleanup_pending: true,
        })
        .expect("serialize failure");
        assert_value_keys(&failure, &["protocolVersion", "code", "cleanupPending"]);
    }

    #[test]
    fn direct_import_uses_only_generated_disk_names() {
        let directory = tempdir().expect("temp directory");
        let staging = directory.path().join("case.staging");
        let published = directory.path().join("case.published");
        let png = make_valid_png().expect("valid png");
        import_direct_png(&png, &staging, &published).expect("direct PNG import");
        let names = fs::read_dir(&published)
            .expect("published directory")
            .map(|entry| {
                entry
                    .expect("published entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<HashSet<_>>();
        assert!(names.contains("index.json"));
        assert!(names.iter().any(|name| name.starts_with("object-00-")));
        assert!(!names.contains("direct.png"));
    }

    #[test]
    fn aggregate_compression_ratio_is_checked_with_overflow_safety() {
        assert!(exceeds_compression_ratio(10_001, 100));
        assert!(!exceeds_compression_ratio(10_000, 100));
        assert!(exceeds_compression_ratio(1, 0));
        assert!(!exceeds_compression_ratio(0, 0));
        assert!(!exceeds_compression_ratio(u64::MAX, u64::MAX));
    }

    #[test]
    fn existing_publish_path_returns_bounded_conflict_code() {
        let directory = tempdir().expect("temp directory");
        let staging = directory.path().join("case.staging");
        let published = directory.path().join("case.published");
        fs::create_dir(&published).expect("pre-existing publish directory");
        let png = make_valid_png().expect("valid png");
        assert!(matches!(
            import_direct_png(&png, &staging, &published),
            Err(ImportFailure::Harness(ProbeErrorCode::PublishConflict))
        ));
        cleanup_case_paths(&staging, &published).expect("case cleanup");
    }

    #[test]
    fn reopened_index_is_bounded_parsed_and_exact_before_object_verification() {
        let directory = tempdir().expect("temp directory");
        let staging = directory.path().join("case.staging");
        let published = directory.path().join("case.published");
        let png = make_valid_png().expect("valid png");
        import_direct_png(&png, &staging, &published).expect("direct PNG import");
        let expected_bytes = fs::read(published.join("index.json")).expect("stored index");
        let expected: StoredIndex =
            serde_json::from_slice(&expected_bytes).expect("parse stored index");
        fs::write(published.join("index.json"), b"{}").expect("corrupt test index");
        assert!(matches!(
            verify_reopened_published_index(&published, &expected),
            Err(ImportFailure::Harness(ProbeErrorCode::PublishFailure))
        ));
    }

    struct ZipLayout {
        eocd: usize,
        central_entries: Vec<usize>,
        local_headers: Vec<usize>,
    }

    fn one_entry_zip() -> Vec<u8> {
        build_zip(vec![file_entry(
            "manifest.json",
            b"{}".to_vec(),
            CompressionMethod::Stored,
        )])
        .expect("one-entry ZIP")
    }

    fn zip_layout(bytes: &[u8]) -> ZipLayout {
        const EOCD_BYTES: usize = 22;
        const CENTRAL_BYTES: usize = 46;
        assert!(bytes.len() >= EOCD_BYTES);
        let eocd = (0..=bytes.len() - EOCD_BYTES)
            .rev()
            .find(|offset| {
                bytes[*offset..].starts_with(b"PK\x05\x06")
                    && read_le_u16(bytes, *offset + 20).is_some_and(|comment| {
                        *offset + EOCD_BYTES + comment as usize == bytes.len()
                    })
            })
            .expect("terminal EOCD");
        let total_entries = read_le_u16(bytes, eocd + 10).expect("EOCD entry count") as usize;
        let mut central = usize::try_from(read_le_u32(bytes, eocd + 16).expect("central offset"))
            .expect("central offset fits usize");
        let mut central_entries = Vec::with_capacity(total_entries);
        let mut local_headers = Vec::with_capacity(total_entries);
        for _ in 0..total_entries {
            assert!(bytes[central..].starts_with(b"PK\x01\x02"));
            central_entries.push(central);
            local_headers.push(
                usize::try_from(read_le_u32(bytes, central + 42).expect("local offset"))
                    .expect("local offset fits usize"),
            );
            let name = read_le_u16(bytes, central + 28).expect("central name") as usize;
            let extra = read_le_u16(bytes, central + 30).expect("central extra") as usize;
            let comment = read_le_u16(bytes, central + 32).expect("central comment") as usize;
            central += CENTRAL_BYTES + name + extra + comment;
        }
        assert_eq!(central, eocd);
        ZipLayout {
            eocd,
            central_entries,
            local_headers,
        }
    }

    fn write_test_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_test_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn insert_central_extra(bytes: &mut Vec<u8>, field: &[u8]) {
        let layout = zip_layout(bytes);
        assert_eq!(layout.central_entries.len(), 1);
        let central = layout.central_entries[0];
        let old_extra = read_le_u16(bytes, central + 30).expect("central extra");
        let name = read_le_u16(bytes, central + 28).expect("central name") as usize;
        let insert_at = central + 46 + name + old_extra as usize;
        let old_central_size = read_le_u32(bytes, layout.eocd + 12).expect("central size");
        let added = u16::try_from(field.len()).expect("small test extra");
        bytes.splice(insert_at..insert_at, field.iter().copied());
        write_test_u16(bytes, central + 30, old_extra + added);
        let new_eocd = layout.eocd + field.len();
        write_test_u32(bytes, new_eocd + 12, old_central_size + u32::from(added));
    }

    fn insert_local_extra(bytes: &mut Vec<u8>, field: &[u8]) {
        let layout = zip_layout(bytes);
        assert_eq!(layout.local_headers.len(), 1);
        let local = layout.local_headers[0];
        let old_extra = read_le_u16(bytes, local + 28).expect("local extra");
        let name = read_le_u16(bytes, local + 26).expect("local name") as usize;
        let insert_at = local + 30 + name + old_extra as usize;
        let old_central_offset = read_le_u32(bytes, layout.eocd + 16).expect("central offset");
        let added = u16::try_from(field.len()).expect("small test extra");
        bytes.splice(insert_at..insert_at, field.iter().copied());
        write_test_u16(bytes, local + 28, old_extra + added);
        let new_eocd = layout.eocd + field.len();
        write_test_u32(bytes, new_eocd + 16, old_central_offset + u32::from(added));
    }

    fn prefix_zip_and_shift_offsets(bytes: &mut Vec<u8>, prefix: &[u8]) {
        let layout = zip_layout(bytes);
        let old_central_offset = read_le_u32(bytes, layout.eocd + 16).expect("central offset");
        let shift = u32::try_from(prefix.len()).expect("small test prefix");
        bytes.splice(0..0, prefix.iter().copied());
        let new_eocd = layout.eocd + prefix.len();
        write_test_u32(bytes, new_eocd + 16, old_central_offset + shift);
        for (central, local) in layout
            .central_entries
            .iter()
            .zip(layout.local_headers.iter())
        {
            let shifted_central = *central + prefix.len();
            let shifted_local = u32::try_from(*local).expect("local offset fits u32") + shift;
            write_test_u32(bytes, shifted_central + 42, shifted_local);
        }
    }

    fn assert_zip_rejected(bytes: &[u8], expected: CaseCode) {
        match preflight_zip_container(bytes) {
            Err(ImportFailure::Rejected(actual)) => assert_eq!(actual, expected),
            other => panic!("expected {expected:?}, got {other:?}"),
        }
    }

    fn assert_png_rejected(bytes: &[u8]) {
        assert!(matches!(
            validate_png(bytes),
            Err(ImportFailure::Rejected(CaseCode::PngMalformed))
        ));
    }

    fn assert_value_keys(value: &serde_json::Value, expected: &[&str]) {
        let actual = value
            .as_object()
            .expect("JSON object")
            .keys()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let expected = expected.iter().copied().collect::<HashSet<_>>();
        assert_eq!(actual, expected);
    }

    fn png_chunk_range(bytes: &[u8], wanted: &[u8; 4]) -> std::ops::Range<usize> {
        let mut offset = 8_usize;
        while offset + 12 <= bytes.len() {
            let length = u32::from_be_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .expect("PNG chunk length"),
            ) as usize;
            let end = offset + 12 + length;
            assert!(end <= bytes.len(), "test PNG must have complete chunks");
            if &bytes[offset + 4..offset + 8] == wanted {
                return offset..end;
            }
            offset = end;
        }
        panic!("wanted PNG chunk was not found")
    }

    fn encoded_png_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12 + data.len());
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(chunk_type);
        bytes.extend_from_slice(data);
        let mut crc = Crc32::new();
        crc.update(chunk_type);
        crc.update(data);
        bytes.extend_from_slice(&crc.finalize().to_be_bytes());
        bytes
    }

    fn insert_chunk_before(
        bytes: &[u8],
        before: &[u8; 4],
        chunk_type: &[u8; 4],
        data: &[u8],
    ) -> Vec<u8> {
        let position = png_chunk_range(bytes, before).start;
        let mut mutated = bytes.to_vec();
        mutated.splice(position..position, encoded_png_chunk(chunk_type, data));
        mutated
    }

    fn replace_chunk(bytes: &[u8], old_type: &[u8; 4], new_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let range = png_chunk_range(bytes, old_type);
        let mut mutated = bytes.to_vec();
        mutated.splice(range, encoded_png_chunk(new_type, data));
        mutated
    }
}
