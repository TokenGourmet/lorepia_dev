use std::{
    cell::Cell,
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, TryLockError},
    time::{Duration, Instant},
};

use lorepia_assets::{AssetError, AssetMime, AssetOwner, AssetStore, IngestRequest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive};

use crate::{
    AcceptedAsset, AcceptedMetadata, ExecutableLanguage, IMPORT_POLICY_VERSION, ImportCounts,
    ImportError, ImportErrorCode, ImportLimits, ImportReceipt, ImportSourceKind,
    QuarantinedExecutable, Result,
    path_policy::{PortablePath, insert_unique, validate_path},
    png_policy::validate_png,
};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const COPY_PREFIX_BYTES: usize = 8;
const MAX_STAGING_ATTEMPTS: usize = 32;
const IMPORT_SESSION_OWNER_TYPE: &str = "lorepia-import-session";
const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const LEASE_HEARTBEAT_CHECKS: u16 = 64;

#[derive(Clone, Debug)]
pub struct ImportService {
    root: PathBuf,
    assets: AssetStore,
    limits: ImportLimits,
    admission: Arc<Mutex<()>>,
}

#[derive(Clone, Debug)]
enum EntryKind {
    Directory,
    Asset(AssetMime),
    Metadata,
    Executable(ExecutableLanguage),
}

#[derive(Clone, Debug)]
struct EntryPlan {
    index: usize,
    path: PortablePath,
    kind: EntryKind,
    bytes: u64,
}

#[derive(Debug)]
struct StagedAsset {
    logical_path: String,
    path: PathBuf,
    mime: AssetMime,
    bytes: u64,
}

#[derive(Debug)]
struct CopiedSource {
    bytes: u64,
    sha256: String,
    prefix: [u8; COPY_PREFIX_BYTES],
}

#[derive(Debug)]
struct StagingSession {
    id: String,
    path: PathBuf,
    cleaned: bool,
    preserved_for_recovery: bool,
}

impl ImportService {
    pub fn open(
        staging_root: impl AsRef<Path>,
        assets: AssetStore,
        limits: ImportLimits,
    ) -> Result<Self> {
        limits.validate()?;
        let root = staging_root.as_ref();
        reject_symlink_if_present(root)?;
        fs::create_dir_all(root).map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
        let root = root
            .canonicalize()
            .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
        let metadata =
            fs::metadata(&root).map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
        if !metadata.is_dir() {
            return Err(ImportError::new(ImportErrorCode::StagingFailure));
        }
        assets
            .recover_temporary_owner_type(IMPORT_SESSION_OWNER_TYPE)
            .map_err(|_| ImportError::new(ImportErrorCode::CleanupFailure).cleanup_pending())?;
        cleanup_stale_staging_sessions(&root, &assets)?;
        Ok(Self {
            root,
            assets,
            limits,
            admission: Arc::new(Mutex::new(())),
        })
    }

    pub fn limits(&self) -> &ImportLimits {
        &self.limits
    }

    pub fn import_path<C>(
        &self,
        source: impl AsRef<Path>,
        owner: AssetOwner,
        is_cancelled: C,
    ) -> Result<ImportReceipt>
    where
        C: FnMut() -> bool,
    {
        validate_final_owner(&owner)?;
        let source = source.as_ref();
        let (mut file, metadata) = open_regular_source(source)?;
        if metadata.len() > self.limits.max_source_bytes {
            return Err(ImportError::new(ImportErrorCode::SourceTooLarge));
        }
        self.import_reader(&mut file, owner, is_cancelled)
    }

    /// Admits a non-seekable stream by copying it into a bounded, generated staging file. The
    /// caller supplies the cancellation policy; cancellation is checked between fixed-size reads.
    pub fn import_reader<R, C>(
        &self,
        reader: &mut R,
        owner: AssetOwner,
        mut is_cancelled: C,
    ) -> Result<ImportReceipt>
    where
        R: Read,
        C: FnMut() -> bool,
    {
        validate_final_owner(&owner)?;
        let _admission = self.try_admit()?;
        self.assets
            .recover_temporary_owner_type(IMPORT_SESSION_OWNER_TYPE)
            .map_err(|_| ImportError::new(ImportErrorCode::CleanupFailure).cleanup_pending())?;
        let mut staging = StagingSession::allocate(&self.root, &self.assets)?;
        let temporary_owner = staging.temporary_owner()?;
        let lease_lost = Cell::new(false);
        let result = {
            let mut heartbeat_checks = 0u16;
            let mut last_heartbeat = Instant::now();
            let mut guarded_cancel = || {
                if is_cancelled() {
                    return true;
                }
                heartbeat_checks = heartbeat_checks.saturating_add(1);
                if heartbeat_checks < LEASE_HEARTBEAT_CHECKS
                    && last_heartbeat.elapsed() < LEASE_HEARTBEAT_INTERVAL
                {
                    return false;
                }
                heartbeat_checks = 0;
                last_heartbeat = Instant::now();
                match self.assets.renew_temporary_owner_session(&temporary_owner) {
                    Ok(true) => false,
                    Ok(false) | Err(_) => {
                        lease_lost.set(true);
                        true
                    }
                }
            };
            (|| {
                let source_path = staging.path.join("source.bin");
                let source = copy_source(reader, &source_path, &self.limits, &mut guarded_cancel)?;
                self.ensure_session_live(&temporary_owner)?;
                if source.prefix == *PNG_SIGNATURE {
                    self.import_direct_png(
                        &source_path,
                        source,
                        owner,
                        &temporary_owner,
                        &mut guarded_cancel,
                    )
                } else if matches!(&source.prefix[..4], b"PK\x03\x04" | b"PK\x05\x06") {
                    self.import_zip(
                        &staging.path,
                        &source_path,
                        source,
                        owner,
                        &temporary_owner,
                        &mut guarded_cancel,
                    )
                } else {
                    Err(ImportError::new(ImportErrorCode::UnsupportedFormat))
                }
            })()
        };
        let result = if lease_lost.get() {
            Err(ImportError::new(ImportErrorCode::AssetRejected))
        } else {
            result
        };
        match result {
            Ok(receipt) => match staging.cleanup() {
                Ok(()) => Ok(receipt),
                Err(()) => Err(ImportError::new(ImportErrorCode::CleanupFailure).cleanup_pending()),
            },
            Err(error) => {
                if self
                    .assets
                    .rollback_temporary_owner(&temporary_owner)
                    .is_err()
                {
                    staging.preserve_for_recovery();
                    return Err(error.cleanup_pending());
                }
                match staging.cleanup() {
                    Ok(()) => Err(error),
                    Err(()) => Err(error.cleanup_pending()),
                }
            }
        }
    }

    fn try_admit(&self) -> Result<MutexGuard<'_, ()>> {
        match self.admission.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::WouldBlock) => Err(ImportError::new(ImportErrorCode::Busy)),
            Err(TryLockError::Poisoned(_)) => Err(ImportError::new(ImportErrorCode::Internal)),
        }
    }

    fn ensure_session_live(&self, temporary_owner: &AssetOwner) -> Result<()> {
        match self
            .assets
            .renew_temporary_owner_session(temporary_owner)
            .map_err(map_asset_error)?
        {
            true => Ok(()),
            false => Err(ImportError::new(ImportErrorCode::AssetRejected)),
        }
    }

    fn import_direct_png<C>(
        &self,
        source_path: &Path,
        source: CopiedSource,
        owner: AssetOwner,
        temporary_owner: &AssetOwner,
        is_cancelled: &mut C,
    ) -> Result<ImportReceipt>
    where
        C: FnMut() -> bool,
    {
        let (_, metadata) = validate_png(source_path, "card.png", &self.limits, is_cancelled)?;
        self.ensure_session_live(temporary_owner)?;
        let mut file = File::open(source_path)
            .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
        let request = IngestRequest::new(AssetMime::Png)
            .with_source_name("card.png")
            .map_err(map_asset_error)?
            .with_owner(temporary_owner.clone());
        let outcome = self
            .assets
            .ingest(&mut file, request, &mut *is_cancelled)
            .map_err(map_asset_error)?;
        let accepted = AcceptedAsset {
            logical_path: "card.png".to_owned(),
            hash: outcome.object.hash,
            bytes: outcome.object.size,
            mime: outcome.object.mime,
        };
        if is_cancelled() {
            return Err(ImportError::new(ImportErrorCode::Cancelled));
        }
        self.ensure_session_live(temporary_owner)?;
        self.assets
            .commit_temporary_owner_refs(temporary_owner, &owner, 1)
            .map_err(map_asset_error)?;
        Ok(receipt(
            ImportSourceKind::PngCard,
            source,
            vec![accepted],
            metadata,
            Vec::new(),
        ))
    }

    fn import_zip<C>(
        &self,
        staging_root: &Path,
        source_path: &Path,
        source: CopiedSource,
        owner: AssetOwner,
        temporary_owner: &AssetOwner,
        is_cancelled: &mut C,
    ) -> Result<ImportReceipt>
    where
        C: FnMut() -> bool,
    {
        let plans = preflight_zip(source_path, source.bytes, &self.limits, is_cancelled)?;
        self.ensure_session_live(temporary_owner)?;
        let file = File::open(source_path)
            .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        let mut archive = ZipArchive::new(BufReader::new(file))
            .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        let mut total_actual = 0u64;
        let mut staged_assets = Vec::new();
        let mut metadata = Vec::new();
        let mut quarantined = Vec::new();

        for plan in &plans {
            if is_cancelled() {
                return Err(ImportError::new(ImportErrorCode::Cancelled));
            }
            if matches!(plan.kind, EntryKind::Directory) {
                continue;
            }
            self.ensure_session_live(temporary_owner)?;
            let mut entry = archive
                .by_index(plan.index)
                .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
            let generated_path = staging_root.join(format!("entry-{:08}.bin", plan.index));
            match plan.kind {
                EntryKind::Asset(mime) => {
                    let (bytes, sha256) = stream_entry(
                        &mut entry,
                        Some(&generated_path),
                        plan.bytes,
                        &mut total_actual,
                        &self.limits,
                        is_cancelled,
                    )?;
                    if bytes != plan.bytes {
                        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
                    }
                    if mime == AssetMime::Png {
                        let (_, png_metadata) = validate_png(
                            &generated_path,
                            &plan.path.logical,
                            &self.limits,
                            is_cancelled,
                        )?;
                        metadata.extend(png_metadata);
                    }
                    let _ = sha256;
                    staged_assets.push(StagedAsset {
                        logical_path: plan.path.logical.clone(),
                        path: generated_path,
                        mime,
                        bytes,
                    });
                }
                EntryKind::Metadata => {
                    let (bytes, sha256) = stream_entry(
                        &mut entry,
                        Some(&generated_path),
                        plan.bytes,
                        &mut total_actual,
                        &self.limits,
                        is_cancelled,
                    )?;
                    validate_json(&generated_path, self.limits.max_metadata_bytes)?;
                    metadata.push(AcceptedMetadata {
                        logical_path: plan.path.logical.clone(),
                        sha256,
                        bytes,
                    });
                }
                EntryKind::Executable(language) => {
                    let (bytes, sha256) = stream_entry(
                        &mut entry,
                        None,
                        plan.bytes,
                        &mut total_actual,
                        &self.limits,
                        is_cancelled,
                    )?;
                    quarantined.push(QuarantinedExecutable::new(
                        plan.path.logical.clone(),
                        language,
                        sha256,
                        bytes,
                    ));
                }
                EntryKind::Directory => unreachable!("directories are skipped"),
            }
        }

        let mut accepted = Vec::with_capacity(staged_assets.len());
        for staged in staged_assets {
            if is_cancelled() {
                return Err(ImportError::new(ImportErrorCode::Cancelled));
            }
            self.ensure_session_live(temporary_owner)?;
            let mut file = File::open(&staged.path)
                .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
            let request = IngestRequest::new(staged.mime)
                .with_source_name(&staged.logical_path)
                .map_err(map_asset_error)?
                .with_owner(temporary_owner.clone());
            let outcome = self
                .assets
                .ingest(&mut file, request, &mut *is_cancelled)
                .map_err(map_asset_error)?;
            if outcome.object.size != staged.bytes {
                return Err(ImportError::new(ImportErrorCode::Internal));
            }
            accepted.push(AcceptedAsset {
                logical_path: staged.logical_path,
                hash: outcome.object.hash,
                bytes: outcome.object.size,
                mime: outcome.object.mime,
            });
        }
        if is_cancelled() {
            return Err(ImportError::new(ImportErrorCode::Cancelled));
        }
        self.ensure_session_live(temporary_owner)?;
        let unique_assets = accepted
            .iter()
            .map(|asset| asset.hash.as_str())
            .collect::<HashSet<_>>()
            .len() as u64;
        if unique_assets == 0 {
            self.assets
                .finish_empty_temporary_owner_session(temporary_owner)
                .map_err(map_asset_error)?;
        } else {
            self.assets
                .commit_temporary_owner_refs(temporary_owner, &owner, unique_assets)
                .map_err(map_asset_error)?;
        }

        Ok(receipt(
            ImportSourceKind::ZipArchive,
            source,
            accepted,
            metadata,
            quarantined,
        ))
    }
}

fn preflight_zip_central(
    source_path: &Path,
    source_bytes: u64,
    limits: &ImportLimits,
) -> Result<usize> {
    const EOCD_BYTES: usize = 22;
    const CENTRAL_BYTES: usize = 46;
    if source_bytes < EOCD_BYTES as u64 {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    let mut file =
        File::open(source_path).map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    let tail_bytes = usize::try_from(source_bytes.min((EOCD_BYTES + u16::MAX as usize) as u64))
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    file.seek(SeekFrom::End(-(tail_bytes as i64)))
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    let mut tail = vec![0u8; tail_bytes];
    file.read_exact(&mut tail)
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    let eocd_in_tail = (0..=tail.len() - EOCD_BYTES)
        .rev()
        .find(|offset| {
            tail[*offset..].starts_with(b"PK\x05\x06")
                && *offset + EOCD_BYTES + usize::from(le_u16(&tail[*offset + 20..*offset + 22]))
                    == tail.len()
        })
        .ok_or_else(|| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    let eocd_offset = source_bytes - tail_bytes as u64 + eocd_in_tail as u64;
    let eocd = &tail[eocd_in_tail..eocd_in_tail + EOCD_BYTES];
    let entries_on_disk = le_u16(&eocd[8..10]);
    let entry_count = le_u16(&eocd[10..12]);
    let central_bytes = le_u32(&eocd[12..16]);
    let central_offset = le_u32(&eocd[16..20]);
    if le_u16(&eocd[4..6]) != 0
        || le_u16(&eocd[6..8]) != 0
        || entries_on_disk != entry_count
        || entry_count == u16::MAX
        || central_bytes == u32::MAX
        || central_offset == u32::MAX
    {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    if entry_count == 0 {
        return Err(ImportError::new(ImportErrorCode::UnsupportedFileType));
    }
    if usize::from(entry_count) > limits.max_archive_entries {
        return Err(ImportError::new(ImportErrorCode::EntryCountLimit));
    }
    let central_offset = u64::from(central_offset);
    let central_end = central_offset
        .checked_add(u64::from(central_bytes))
        .ok_or_else(|| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    if central_end != eocd_offset {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }

    file.seek(SeekFrom::Start(central_offset))
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    let mut total_uncompressed = 0u64;
    let mut files = HashSet::with_capacity(usize::from(entry_count));
    let mut directories = HashSet::with_capacity(usize::from(entry_count));
    for _ in 0..entry_count {
        let mut fixed = [0u8; CENTRAL_BYTES];
        file.read_exact(&mut fixed)
            .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        if &fixed[..4] != b"PK\x01\x02" {
            return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
        }
        let flags = le_u16(&fixed[8..10]);
        let method = le_u16(&fixed[10..12]);
        validate_zip_flags(flags, method)?;
        let compressed_size = le_u32(&fixed[20..24]);
        let uncompressed_size = le_u32(&fixed[24..28]);
        let name_bytes = usize::from(le_u16(&fixed[28..30]));
        let extra_bytes = usize::from(le_u16(&fixed[30..32]));
        let comment_bytes = usize::from(le_u16(&fixed[32..34]));
        if compressed_size == u32::MAX
            || uncompressed_size == u32::MAX
            || le_u16(&fixed[34..36]) != 0
            || le_u32(&fixed[42..46]) == u32::MAX
            || name_bytes == 0
            || name_bytes > limits.max_path_bytes.saturating_add(1)
            || extra_bytes > limits.max_zip_extra_bytes
            || comment_bytes != 0
        {
            return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
        }
        let compressed_size = u64::from(compressed_size);
        let uncompressed_size = u64::from(uncompressed_size);
        if method == 0 && compressed_size != uncompressed_size {
            return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
        }
        if uncompressed_size > limits.max_entry_bytes {
            return Err(ImportError::new(ImportErrorCode::EntrySizeLimit));
        }
        total_uncompressed = total_uncompressed
            .checked_add(uncompressed_size)
            .ok_or_else(|| ImportError::new(ImportErrorCode::TotalSizeLimit))?;
        if total_uncompressed > limits.max_total_uncompressed_bytes {
            return Err(ImportError::new(ImportErrorCode::TotalSizeLimit));
        }
        if ratio_exceeded(
            uncompressed_size,
            compressed_size,
            limits.max_compression_ratio,
        ) {
            return Err(ImportError::new(ImportErrorCode::CompressionRatioLimit));
        }
        let mut name = vec![0u8; name_bytes];
        file.read_exact(&mut name)
            .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        let mut extra = vec![0u8; extra_bytes];
        file.read_exact(&mut extra)
            .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        reject_zip64_extra(Some(&extra))?;
        let is_directory = name.ends_with(b"/");
        let path = validate_path(&name, is_directory, limits)?;
        insert_unique(&path, &mut files, &mut directories)?;
        let _ = classify_entry(&path)?;
    }
    let position = file
        .stream_position()
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    if position != central_end {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    Ok(usize::from(entry_count))
}

fn validate_zip_flags(flags: u16, method: u16) -> Result<()> {
    const ENCRYPTED: u16 = 1 << 0;
    const DEFLATE_OPTIONS: u16 = (1 << 1) | (1 << 2);
    const DATA_DESCRIPTOR: u16 = 1 << 3;
    const STRONG_ENCRYPTION: u16 = 1 << 6;
    const UTF8: u16 = 1 << 11;
    const CENTRAL_DIRECTORY_ENCRYPTION: u16 = 1 << 13;
    if flags & (ENCRYPTED | STRONG_ENCRYPTION | CENTRAL_DIRECTORY_ENCRYPTION) != 0 {
        return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
    }
    if flags & DATA_DESCRIPTOR != 0
        || flags & !(DEFLATE_OPTIONS | UTF8) != 0
        || (method == 0 && flags & DEFLATE_OPTIONS != 0)
    {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    if !matches!(method, 0 | 8) {
        return Err(ImportError::new(ImportErrorCode::UnsupportedCompression));
    }
    Ok(())
}

fn preflight_zip<C>(
    source_path: &Path,
    source_bytes: u64,
    limits: &ImportLimits,
    is_cancelled: &mut C,
) -> Result<Vec<EntryPlan>>
where
    C: FnMut() -> bool,
{
    let central_entry_count = preflight_zip_central(source_path, source_bytes, limits)?;
    let file =
        File::open(source_path).map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    if archive.len() != central_entry_count {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }

    let mut plans = Vec::with_capacity(archive.len());
    let mut files = HashSet::with_capacity(archive.len());
    let mut directories = HashSet::with_capacity(archive.len());
    let mut total = 0u64;
    let mut ranges = Vec::with_capacity(archive.len());
    let mut minimum_header = u64::MAX;

    for index in 0..archive.len() {
        if is_cancelled() {
            return Err(ImportError::new(ImportErrorCode::Cancelled));
        }
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        if entry.encrypted() {
            return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
        }
        if !matches!(
            entry.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(ImportError::new(ImportErrorCode::UnsupportedCompression));
        }
        let is_directory = entry.is_dir();
        if entry.is_symlink() || !portable_entry_type(entry.unix_mode(), is_directory) {
            return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
        }
        reject_zip64_extra(entry.extra_data())?;
        if entry.size() == u64::from(u32::MAX) || entry.compressed_size() == u64::from(u32::MAX) {
            return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
        }
        if entry.size() > limits.max_entry_bytes {
            return Err(ImportError::new(ImportErrorCode::EntrySizeLimit));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| ImportError::new(ImportErrorCode::TotalSizeLimit))?;
        if total > limits.max_total_uncompressed_bytes {
            return Err(ImportError::new(ImportErrorCode::TotalSizeLimit));
        }
        if ratio_exceeded(
            entry.size(),
            entry.compressed_size(),
            limits.max_compression_ratio,
        ) {
            return Err(ImportError::new(ImportErrorCode::CompressionRatioLimit));
        }
        let path = validate_path(entry.name_raw(), is_directory, limits)?;
        insert_unique(&path, &mut files, &mut directories)?;
        let kind = classify_entry(&path)?;
        if is_directory && entry.size() != 0 {
            return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
        }

        let header_start = entry.header_start();
        let data_start = entry
            .data_start()
            .ok_or_else(|| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        let data_end = data_start
            .checked_add(entry.compressed_size())
            .ok_or_else(|| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        if data_start < header_start
            || data_end > entry.central_header_start()
            || data_end > source_bytes
        {
            return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
        }
        validate_local_header(
            source_path,
            header_start,
            data_start,
            entry.name_raw(),
            entry.compression(),
            entry.crc32(),
            entry.compressed_size(),
            entry.size(),
            limits.max_zip_extra_bytes,
        )?;
        minimum_header = minimum_header.min(header_start);
        ranges.push((header_start, data_end));
        plans.push(EntryPlan {
            index,
            path,
            kind,
            bytes: entry.size(),
        });
    }
    if !plans.is_empty() && minimum_header != 0 {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    ranges.sort_unstable();
    for pair in ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
        }
    }
    let compressed_total = archive
        .decompressed_size()
        .ok_or_else(|| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    if compressed_total != u128::from(total) {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    Ok(plans)
}

#[allow(clippy::too_many_arguments)]
fn validate_local_header(
    source_path: &Path,
    header_start: u64,
    expected_data_start: u64,
    expected_name: &[u8],
    compression: CompressionMethod,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    max_extra_bytes: usize,
) -> Result<()> {
    let mut file =
        File::open(source_path).map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    file.seek(SeekFrom::Start(header_start))
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    let mut fixed = [0u8; 30];
    file.read_exact(&mut fixed)
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    if &fixed[..4] != b"PK\x03\x04" {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    let flags = le_u16(&fixed[6..8]);
    const ENCRYPTED: u16 = 1 << 0;
    const DATA_DESCRIPTOR: u16 = 1 << 3;
    const STRONG_ENCRYPTION: u16 = 1 << 6;
    const CENTRAL_DIRECTORY_ENCRYPTION: u16 = 1 << 13;
    const UTF8: u16 = 1 << 11;
    const DEFLATE_OPTIONS: u16 = (1 << 1) | (1 << 2);
    let allowed = UTF8 | DEFLATE_OPTIONS;
    if flags & (ENCRYPTED | DATA_DESCRIPTOR | STRONG_ENCRYPTION | CENTRAL_DIRECTORY_ENCRYPTION) != 0
        || flags & !allowed != 0
        || (compression == CompressionMethod::Stored && flags & DEFLATE_OPTIONS != 0)
    {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    let method = le_u16(&fixed[8..10]);
    let expected_method = match compression {
        CompressionMethod::Stored => 0,
        CompressionMethod::Deflated => 8,
        _ => return Err(ImportError::new(ImportErrorCode::UnsupportedCompression)),
    };
    if method != expected_method
        || le_u32(&fixed[14..18]) != crc32
        || u64::from(le_u32(&fixed[18..22])) != compressed_size
        || u64::from(le_u32(&fixed[22..26])) != uncompressed_size
    {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    let name_bytes = usize::from(le_u16(&fixed[26..28]));
    let extra_bytes = usize::from(le_u16(&fixed[28..30]));
    if name_bytes != expected_name.len() || extra_bytes > max_extra_bytes {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    let mut name = vec![0u8; name_bytes];
    file.read_exact(&mut name)
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    if name != expected_name {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    let mut extra = vec![0u8; extra_bytes];
    file.read_exact(&mut extra)
        .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    reject_zip64_extra(Some(&extra))?;
    let actual_data_start = header_start
        .checked_add(30)
        .and_then(|value| value.checked_add(name_bytes as u64))
        .and_then(|value| value.checked_add(extra_bytes as u64))
        .ok_or_else(|| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
    if actual_data_start != expected_data_start {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    Ok(())
}

fn reject_zip64_extra(extra: Option<&[u8]>) -> Result<()> {
    let Some(extra) = extra else {
        return Ok(());
    };
    let mut offset = 0usize;
    while offset < extra.len() {
        if extra.len() - offset < 4 {
            return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
        }
        let kind = le_u16(&extra[offset..offset + 2]);
        let length = usize::from(le_u16(&extra[offset + 2..offset + 4]));
        offset = offset
            .checked_add(4)
            .and_then(|value| value.checked_add(length))
            .ok_or_else(|| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        if kind == 0x0001 || offset > extra.len() {
            return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
        }
    }
    Ok(())
}

fn portable_entry_type(mode: Option<u32>, is_directory: bool) -> bool {
    let Some(mode) = mode else {
        return true;
    };
    let file_type = mode & 0o170_000;
    if is_directory {
        matches!(file_type, 0 | 0o040_000)
    } else {
        matches!(file_type, 0 | 0o100_000)
    }
}

fn classify_entry(path: &PortablePath) -> Result<EntryKind> {
    if path.is_directory {
        return Ok(EntryKind::Directory);
    }
    let extension = path
        .logical
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .ok_or_else(|| ImportError::new(ImportErrorCode::UnsupportedFileType))?;
    match extension.as_str() {
        "png" => Ok(EntryKind::Asset(AssetMime::Png)),
        "jpg" | "jpeg" => Ok(EntryKind::Asset(AssetMime::Jpeg)),
        "webp" => Ok(EntryKind::Asset(AssetMime::WebP)),
        "gif" => Ok(EntryKind::Asset(AssetMime::Gif)),
        "wav" => Ok(EntryKind::Asset(AssetMime::Wav)),
        "mp3" => Ok(EntryKind::Asset(AssetMime::Mp3)),
        "ogg" => Ok(EntryKind::Asset(AssetMime::Ogg)),
        "flac" => Ok(EntryKind::Asset(AssetMime::Flac)),
        "json" => Ok(EntryKind::Metadata),
        "js" | "mjs" | "cjs" => Ok(EntryKind::Executable(ExecutableLanguage::JavaScript)),
        "lua" => Ok(EntryKind::Executable(ExecutableLanguage::Lua)),
        "wasm" => Ok(EntryKind::Executable(ExecutableLanguage::WebAssembly)),
        _ => Err(ImportError::new(ImportErrorCode::UnsupportedFileType)),
    }
}

fn stream_entry<R, C>(
    reader: &mut R,
    output_path: Option<&Path>,
    declared_bytes: u64,
    total_actual: &mut u64,
    limits: &ImportLimits,
    is_cancelled: &mut C,
) -> Result<(u64, String)>
where
    R: Read,
    C: FnMut() -> bool,
{
    let mut output = output_path
        .map(|path| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))
        })
        .transpose()?;
    let mut buffer = vec![0u8; limits.copy_buffer_bytes];
    let mut entry_actual = 0u64;
    let mut hash = Sha256::new();
    loop {
        if is_cancelled() {
            return Err(ImportError::new(ImportErrorCode::Cancelled));
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|_| ImportError::new(ImportErrorCode::ArchiveMalformed))?;
        if read == 0 {
            break;
        }
        entry_actual = entry_actual
            .checked_add(read as u64)
            .ok_or_else(|| ImportError::new(ImportErrorCode::EntrySizeLimit))?;
        if entry_actual > declared_bytes || entry_actual > limits.max_entry_bytes {
            return Err(ImportError::new(ImportErrorCode::EntrySizeLimit));
        }
        *total_actual = total_actual
            .checked_add(read as u64)
            .ok_or_else(|| ImportError::new(ImportErrorCode::TotalSizeLimit))?;
        if *total_actual > limits.max_total_uncompressed_bytes {
            return Err(ImportError::new(ImportErrorCode::TotalSizeLimit));
        }
        hash.update(&buffer[..read]);
        if let Some(file) = output.as_mut() {
            file.write_all(&buffer[..read])
                .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
        }
    }
    if entry_actual != declared_bytes {
        return Err(ImportError::new(ImportErrorCode::ArchiveMalformed));
    }
    if let Some(file) = output {
        file.sync_all()
            .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
    }
    Ok((entry_actual, hex_digest(hash.finalize().as_slice())))
}

fn copy_source<R, C>(
    reader: &mut R,
    destination: &Path,
    limits: &ImportLimits,
    is_cancelled: &mut C,
) -> Result<CopiedSource>
where
    R: Read,
    C: FnMut() -> bool,
{
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
    let mut buffer = vec![0u8; limits.copy_buffer_bytes];
    let mut prefix = [0u8; COPY_PREFIX_BYTES];
    let mut prefix_len = 0usize;
    let mut bytes = 0u64;
    let mut hash = Sha256::new();
    loop {
        if is_cancelled() {
            return Err(ImportError::new(ImportErrorCode::Cancelled));
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| crate::error::io_code(&error, ImportErrorCode::StagingFailure))?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or_else(|| ImportError::new(ImportErrorCode::SourceTooLarge))?;
        if bytes > limits.max_source_bytes {
            return Err(ImportError::new(ImportErrorCode::SourceTooLarge));
        }
        let prefix_take = (COPY_PREFIX_BYTES - prefix_len).min(read);
        prefix[prefix_len..prefix_len + prefix_take].copy_from_slice(&buffer[..prefix_take]);
        prefix_len += prefix_take;
        hash.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
    }
    file.sync_all()
        .map_err(|_| ImportError::new(ImportErrorCode::StagingFailure))?;
    Ok(CopiedSource {
        bytes,
        sha256: hex_digest(hash.finalize().as_slice()),
        prefix,
    })
}

fn validate_json(path: &Path, max_bytes: u64) -> Result<()> {
    let metadata =
        fs::metadata(path).map_err(|_| ImportError::new(ImportErrorCode::MetadataMalformed))?;
    if metadata.len() > max_bytes {
        return Err(ImportError::new(ImportErrorCode::EntrySizeLimit));
    }
    let file =
        File::open(path).map_err(|_| ImportError::new(ImportErrorCode::MetadataMalformed))?;
    let mut deserializer = serde_json::Deserializer::from_reader(BufReader::new(file));
    serde::de::IgnoredAny::deserialize(&mut deserializer)
        .map_err(|_| ImportError::new(ImportErrorCode::MetadataMalformed))?;
    deserializer
        .end()
        .map_err(|_| ImportError::new(ImportErrorCode::MetadataMalformed))
}

fn receipt(
    source_kind: ImportSourceKind,
    source: CopiedSource,
    assets: Vec<AcceptedAsset>,
    metadata: Vec<AcceptedMetadata>,
    quarantined: Vec<QuarantinedExecutable>,
) -> ImportReceipt {
    let accepted = assets.len().saturating_add(metadata.len()) as u32;
    ImportReceipt {
        protocol_version: 1,
        policy_version: IMPORT_POLICY_VERSION,
        source_kind,
        source_sha256: source.sha256,
        source_bytes: source.bytes,
        counts: ImportCounts {
            accepted,
            quarantined: quarantined.len() as u32,
            rejected: 0,
        },
        assets,
        metadata,
        quarantined,
        executable_entries_executed: 0,
    }
}

fn ratio_exceeded(uncompressed: u64, compressed: u64, ratio: u64) -> bool {
    if uncompressed == 0 {
        false
    } else if compressed == 0 {
        true
    } else {
        uncompressed > compressed.saturating_mul(ratio)
    }
}

fn map_asset_error(error: AssetError) -> ImportError {
    match error {
        AssetError::Cancelled => ImportError::new(ImportErrorCode::Cancelled),
        AssetError::MutationBusy => ImportError::new(ImportErrorCode::Busy),
        _ => ImportError::new(ImportErrorCode::AssetRejected),
    }
}

fn validate_final_owner(owner: &AssetOwner) -> Result<()> {
    AssetOwner::new(owner.owner_type.clone(), owner.owner_id.clone()).map_err(map_asset_error)?;
    if owner.owner_type == IMPORT_SESSION_OWNER_TYPE {
        return Err(ImportError::new(ImportErrorCode::AssetRejected));
    }
    Ok(())
}

fn open_regular_source(path: &Path) -> Result<(File, fs::Metadata)> {
    let before = fs::symlink_metadata(path)
        .map_err(|_| ImportError::new(ImportErrorCode::UnsupportedFormat))?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        if before.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

        // Open a final reparse point itself instead of following it.  The
        // post-open metadata check below then rejects a path swapped to a
        // symlink/junction between the initial inspection and CreateFile.
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|_| ImportError::new(ImportErrorCode::UnsafeEntryType))?;
    let after = file
        .metadata()
        .map_err(|_| ImportError::new(ImportErrorCode::UnsafeEntryType))?;
    if !after.is_file() {
        return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        if after.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || before.volume_serial_number().is_none()
            || before.file_index().is_none()
            || before.volume_serial_number() != after.volume_serial_number()
            || before.file_index() != after.file_index()
        {
            return Err(ImportError::new(ImportErrorCode::UnsafeEntryType));
        }
    }
    Ok((file, after))
}

fn reject_symlink_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ImportError::new(ImportErrorCode::StagingFailure))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ImportError::new(ImportErrorCode::StagingFailure)),
    }
}

impl StagingSession {
    fn allocate(root: &Path, assets: &AssetStore) -> Result<Self> {
        for _ in 0..MAX_STAGING_ATTEMPTS {
            let id = Uuid::new_v4().simple().to_string();
            let temporary_owner = AssetOwner::new(IMPORT_SESSION_OWNER_TYPE, id.clone())
                .map_err(|_| ImportError::new(ImportErrorCode::Internal))?;
            assets
                .begin_temporary_owner_session(&temporary_owner)
                .map_err(map_asset_error)?;
            let path = root.join(format!("{id}.partial"));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        id,
                        path,
                        cleaned: false,
                        preserved_for_recovery: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    assets
                        .finish_empty_temporary_owner_session(&temporary_owner)
                        .map_err(map_asset_error)?;
                    continue;
                }
                Err(_) => {
                    assets
                        .finish_empty_temporary_owner_session(&temporary_owner)
                        .map_err(map_asset_error)?;
                    return Err(ImportError::new(ImportErrorCode::StagingFailure));
                }
            }
        }
        Err(ImportError::new(ImportErrorCode::StagingFailure))
    }

    fn temporary_owner(&self) -> Result<AssetOwner> {
        AssetOwner::new(IMPORT_SESSION_OWNER_TYPE, self.id.clone())
            .map_err(|_| ImportError::new(ImportErrorCode::Internal))
    }

    fn preserve_for_recovery(&mut self) {
        self.preserved_for_recovery = true;
    }

    fn cleanup(&mut self) -> std::result::Result<(), ()> {
        let result = match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_symlink() => fs::remove_file(&self.path),
            Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(&self.path),
            Ok(_) => fs::remove_file(&self.path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
        if result.is_ok() {
            self.cleaned = true;
            Ok(())
        } else {
            Err(())
        }
    }
}

impl Drop for StagingSession {
    fn drop(&mut self) {
        if !self.cleaned && !self.preserved_for_recovery {
            let _ = self.cleanup();
        }
    }
}

fn cleanup_stale_staging_sessions(root: &Path, assets: &AssetStore) -> Result<()> {
    let entries = fs::read_dir(root)
        .map_err(|_| ImportError::new(ImportErrorCode::CleanupFailure).cleanup_pending())?;
    for entry in entries {
        let entry = entry
            .map_err(|_| ImportError::new(ImportErrorCode::CleanupFailure).cleanup_pending())?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(id) = name.strip_suffix(".partial") else {
            continue;
        };
        if id.len() != 32
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            continue;
        }
        let temporary_owner =
            AssetOwner::new(IMPORT_SESSION_OWNER_TYPE, id.to_owned()).map_err(map_asset_error)?;
        if assets
            .temporary_owner_session_is_live(&temporary_owner)
            .map_err(map_asset_error)?
        {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| ImportError::new(ImportErrorCode::CleanupFailure).cleanup_pending())?;
        let cleanup = if metadata.file_type().is_symlink() || metadata.is_file() {
            fs::remove_file(path)
        } else if metadata.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        cleanup.map_err(|_| ImportError::new(ImportErrorCode::CleanupFailure).cleanup_pending())?;
    }
    Ok(())
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}
