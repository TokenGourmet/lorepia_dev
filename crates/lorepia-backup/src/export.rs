use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
};

use lorepia_assets::AssetHash;
use uuid::Uuid;

use crate::{
    BackupEntry, BackupError, BackupManifest, BackupProgress, CompatibilityReceipt, Control,
    ExportOptions, ExportReport, ExportRequest, FreeSpaceAssessment, FreeSpaceDisposition,
    ManifestDatabase, Operation, Phase, Result, SnapshotContract, UnknownSpacePolicy,
    fsutil::{
        atomic_write, canonical_json, directory_is_empty, hash_file_cancellable,
        reject_symlink_directory, reject_symlink_if_present, sync_directory, sync_tree,
        validate_portable_relative_path,
    },
    journal::{read_progress, write_progress},
    manifest::{push_manifest_entry, validate_manifest_budget, write_canonical_manifest},
    model::{
        BACKUP_FORMAT_VERSION, MAX_BACKUP_ASSET_OBJECTS, MAX_BACKUP_MANIFEST_BYTES,
        SPACE_RESERVE_BYTES,
    },
    secret::scan_owned_paths_for_secrets_cancellable,
    sqlite::{
        catalog_objects_after, catalog_summary, database_version, validate_asset_catalog,
        validate_product_database,
    },
};

const JOURNAL_NAME: &str = "progress.json";
const PRODUCT_DATABASE_PATH: &str = "data/product.sqlite3";
const ASSET_CATALOG_PATH: &str = "data/assets/assets.sqlite3";
const COMPATIBILITY_RECEIPT_PATH: &str = "receipts/compatibility.json";
const MANIFEST_PATH: &str = "manifest.json";
const MANIFEST_HASH_PATH: &str = "manifest.sha256";
const OBJECT_PAGE_SIZE: u16 = 512;

pub fn partial_path_for(destination: &Path) -> Result<PathBuf> {
    let parent = destination.parent().ok_or(BackupError::InvalidInput {
        field: "backup destination",
        reason: "must have a parent directory",
    })?;
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(BackupError::InvalidInput {
            field: "backup destination",
            reason: "must have a UTF-8 file name",
        })?;
    validate_portable_relative_path(name)?;
    Ok(parent.join(format!(".{name}.lorepia-partial")))
}

pub fn export_backup<F>(
    request: ExportRequest<'_>,
    options: ExportOptions,
    observe: F,
) -> Result<ExportReport>
where
    F: FnMut(&BackupProgress) -> Control,
{
    let assets = request.assets;
    let destination = request.destination.to_path_buf();
    let result = export_backup_resumable(request, options, observe);
    if let Err(error) = &result
        && !is_resumable_export_error(error)
    {
        abandon_export(assets, &destination)?;
    }
    result
}

fn export_backup_resumable<F>(
    request: ExportRequest<'_>,
    options: ExportOptions,
    mut observe: F,
) -> Result<ExportReport>
where
    F: FnMut(&BackupProgress) -> Control,
{
    request.assets.cleanup_expired_backup_snapshots()?;
    if request.destination.exists() {
        return Err(BackupError::DestinationExists(
            request.destination.to_path_buf(),
        ));
    }
    let parent = request
        .destination
        .parent()
        .ok_or(BackupError::InvalidInput {
            field: "backup destination",
            reason: "must have a parent directory",
        })?;
    fs::create_dir_all(parent)?;
    reject_symlink_directory(parent)?;
    let destination_name = request
        .destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(BackupError::InvalidInput {
            field: "backup destination",
            reason: "must have a portable UTF-8 name",
        })?
        .to_owned();
    validate_portable_relative_path(&destination_name)?;

    let partial = partial_path_for(request.destination)?;
    reject_symlink_if_present(&partial)?;
    let journal_path = partial.join(JOURNAL_NAME);
    if partial.exists() && !journal_path.exists() {
        if directory_is_empty(&partial)? {
            fs::remove_dir(&partial)?;
            sync_directory(parent)?;
        } else {
            return Err(BackupError::JournalConflict);
        }
    }
    let resumed = partial.exists();
    let mut progress = if resumed {
        reject_symlink_directory(&partial)?;
        let journal = read_progress(&journal_path)?;
        if journal.operation != Operation::Export || journal.destination_name != destination_name {
            return Err(BackupError::JournalConflict);
        }
        journal
    } else {
        let required = estimate_required_bytes(&request)?;
        let available = request.space_probe.available_bytes(parent)?;
        let assessment = assess_space(required, available);
        enforce_space_policy(assessment, options.unknown_space_policy)?;
        fs::create_dir(&partial)?;
        sync_directory(parent)?;
        let journal = BackupProgress {
            journal_version: 1,
            operation: Operation::Export,
            session_id: Uuid::new_v4().simple().to_string(),
            phase: Phase::Prepared,
            destination_name: destination_name.clone(),
            source_manifest_sha256: None,
            last_verified_object_hash: None,
            verified_objects: 0,
            verified_bytes: 0,
            required_bytes: required,
            available_bytes: available,
            replaced_existing: false,
        };
        write_progress(&journal_path, &journal)?;
        journal
    };

    if resumed && progress.phase >= Phase::AssetCatalogSnapshot {
        let catalog_path = partial.join(ASSET_CATALOG_PATH);
        let catalog = catalog_summary(&catalog_path)?;
        let lease = request.assets.renew_backup_snapshot(&progress.session_id)?;
        if lease
            .as_ref()
            .is_none_or(|lease| lease.pinned_objects != catalog.object_count)
        {
            abandon_export(request.assets, request.destination)?;
            return export_backup_resumable(request, options, observe);
        }
    }

    notify(&progress, &mut observe)?;
    let product_path = partial.join(PRODUCT_DATABASE_PATH);
    if progress.phase < Phase::ProductSnapshot || !product_path.is_file() {
        remove_regular_file_if_present(&product_path)?;
        ensure_parent(&product_path)?;
        request.product.online_snapshot_to(&product_path, |_, _| {
            matches!(observe(&progress), Control::Continue)
        })?;
        validate_product_database(&product_path)?;
        progress.phase = Phase::ProductSnapshot;
        write_progress(&journal_path, &progress)?;
        notify(&progress, &mut observe)?;
    }

    let catalog_path = partial.join(ASSET_CATALOG_PATH);
    if progress.phase < Phase::AssetCatalogSnapshot || !catalog_path.is_file() {
        remove_regular_file_if_present(&catalog_path)?;
        ensure_parent(&catalog_path)?;
        request
            .assets
            .begin_backup_snapshot(&progress.session_id, &catalog_path, |_, _| {
                matches!(observe(&progress), Control::Continue)
            })?;
        validate_asset_catalog(&catalog_path)?;
        progress.phase = Phase::AssetCatalogSnapshot;
        write_progress(&journal_path, &progress)?;
        notify(&progress, &mut observe)?;
    }

    let catalog = catalog_summary(&catalog_path)?;
    require_snapshot_lease(&request, &progress, catalog.object_count)?;
    if let Err(error) = validate_catalog_object_count(catalog.object_count) {
        request
            .assets
            .release_backup_snapshot(&progress.session_id)?;
        reject_symlink_directory(&partial)?;
        fs::remove_dir_all(&partial)?;
        sync_directory(parent)?;
        return Err(error);
    }
    if progress.verified_objects > catalog.object_count
        || progress.verified_bytes > catalog.total_bytes
    {
        return Err(BackupError::JournalConflict);
    }
    let product_size = fs::metadata(&product_path)?.len();
    let catalog_size = fs::metadata(&catalog_path)?.len();
    progress.required_bytes = total_export_bytes(product_size, catalog_size, catalog.total_bytes)?;
    progress.available_bytes = request.space_probe.available_bytes(parent)?;
    let remaining = catalog
        .total_bytes
        .checked_sub(progress.verified_bytes)
        .and_then(|value| value.checked_add(MAX_BACKUP_MANIFEST_BYTES))
        .and_then(|value| value.checked_add(SPACE_RESERVE_BYTES))
        .ok_or(BackupError::SizeOverflow)?;
    let free_space = assess_space(remaining, progress.available_bytes);
    enforce_space_policy(free_space, options.unknown_space_policy)?;
    write_progress(&journal_path, &progress)?;

    copy_snapshot_objects(
        &request,
        &partial,
        &catalog_path,
        &journal_path,
        &mut progress,
        catalog.object_count,
        &mut observe,
    )?;
    progress.phase = Phase::Objects;
    write_progress(&journal_path, &progress)?;
    notify(&progress, &mut observe)?;

    let compatibility = vec![CompatibilityReceipt {
        check_id: "BACKUP-007".to_owned(),
        disposition: "not_applicable_by_design".to_owned(),
        reason: "the versioned backup package is a directory and has no ZIP central directory"
            .to_owned(),
    }];
    atomic_write(
        &partial.join(COMPATIBILITY_RECEIPT_PATH),
        &canonical_json(&compatibility)?,
    )?;

    progress.phase = Phase::ReadyToPublish;
    write_progress(&journal_path, &progress)?;
    require_snapshot_lease(&request, &progress, catalog.object_count)?;

    let manifest = build_manifest(
        &partial,
        &catalog_path,
        &progress,
        compatibility,
        &mut observe,
    )?;
    let manifest_hash = write_canonical_manifest(&partial.join(MANIFEST_PATH), &manifest)?;
    atomic_write(
        &partial.join(MANIFEST_HASH_PATH),
        format!("{manifest_hash}\n").as_bytes(),
    )?;
    require_snapshot_lease(&request, &progress, catalog.object_count)?;
    verify_manifest_entries(&partial, &manifest, &progress, &mut observe)?;
    require_snapshot_lease(&request, &progress, catalog.object_count)?;
    let secret_report = scan_owned_paths_for_secrets_cancellable(
        manifest
            .entries
            .iter()
            .map(|entry| (entry.path.clone(), partial.join(&entry.path)))
            .chain([
                (MANIFEST_PATH.to_owned(), partial.join(MANIFEST_PATH)),
                (
                    MANIFEST_HASH_PATH.to_owned(),
                    partial.join(MANIFEST_HASH_PATH),
                ),
            ]),
        request.secret_sentinels,
        || matches!(observe(&progress), Control::Cancel),
    )?;
    require_snapshot_lease(&request, &progress, catalog.object_count)?;
    if secret_report.matches != 0 {
        return Err(BackupError::InvalidManifest {
            reason: "secret scanner returned a non-zero match count",
        });
    }
    progress.phase = Phase::Verified;
    notify(&progress, &mut observe)?;

    request
        .assets
        .release_backup_snapshot(&progress.session_id)?;
    sync_tree(&partial)?;
    if request.destination.exists() {
        return Err(BackupError::DestinationExists(
            request.destination.to_path_buf(),
        ));
    }
    fs::rename(&partial, request.destination)?;
    sync_directory(parent)?;

    Ok(ExportReport {
        destination: request.destination.to_path_buf(),
        session_id: progress.session_id,
        manifest_sha256: manifest_hash,
        object_count: progress.verified_objects,
        total_entry_bytes: manifest.total_entry_bytes,
        free_space,
        resumed,
    })
}

/// Explicitly abandons a partial export and releases its durable asset pins.
///
/// A missing partial is an idempotent success. A malformed or mismatched journal is not trusted
/// to name a session; its database lease remains subject to the 24-hour stale cleanup contract.
pub fn abandon_export(assets: &lorepia_assets::AssetStore, destination: &Path) -> Result<bool> {
    let partial = partial_path_for(destination)?;
    match fs::symlink_metadata(&partial) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(BackupError::UnsafePath {
                path: partial.display().to_string(),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    let journal = read_progress(&partial.join(JOURNAL_NAME))?;
    let destination_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(BackupError::InvalidInput {
            field: "backup destination",
            reason: "must have a portable UTF-8 name",
        })?;
    if journal.operation != Operation::Export || journal.destination_name != destination_name {
        return Err(BackupError::JournalConflict);
    }
    assets.abandon_backup_snapshot(&journal.session_id)?;
    fs::remove_dir_all(&partial)?;
    sync_directory(
        partial
            .parent()
            .expect("validated partial backup path has a parent"),
    )?;
    Ok(true)
}

fn is_resumable_export_error(error: &BackupError) -> bool {
    matches!(
        error,
        BackupError::Cancelled
            | BackupError::InsufficientSpace { .. }
            | BackupError::FreeSpaceUnknown
            | BackupError::Io(_)
    )
}

fn require_snapshot_lease(
    request: &ExportRequest<'_>,
    progress: &BackupProgress,
    expected_objects: u64,
) -> Result<()> {
    let lease = request
        .assets
        .renew_backup_snapshot(&progress.session_id)?
        .ok_or(BackupError::SnapshotLeaseExpired)?;
    if lease.pinned_objects != expected_objects {
        return Err(BackupError::SnapshotLeaseExpired);
    }
    Ok(())
}

fn estimate_required_bytes(request: &ExportRequest<'_>) -> Result<u64> {
    let product = request.product.snapshot_size_estimate()?;
    let catalog = request.assets.catalog_snapshot_size_estimate()?;
    let objects = request.assets.stats()?.active_bytes;
    total_export_bytes(product, catalog, objects)
}

fn total_export_bytes(product: u64, catalog: u64, objects: u64) -> Result<u64> {
    product
        .checked_add(catalog)
        .and_then(|value| value.checked_add(objects))
        .and_then(|value| value.checked_add(MAX_BACKUP_MANIFEST_BYTES))
        .and_then(|value| value.checked_add(SPACE_RESERVE_BYTES))
        .ok_or(BackupError::SizeOverflow)
}

fn validate_catalog_object_count(object_count: u64) -> Result<()> {
    if object_count > MAX_BACKUP_ASSET_OBJECTS as u64 {
        return Err(BackupError::InvalidManifest {
            reason: "asset snapshot exceeds the v1 object limit",
        });
    }
    Ok(())
}

fn assess_space(required_bytes: u64, available_bytes: Option<u64>) -> FreeSpaceAssessment {
    let disposition = match available_bytes {
        Some(available) if available >= required_bytes => FreeSpaceDisposition::Enough,
        Some(_) => FreeSpaceDisposition::Insufficient,
        None => FreeSpaceDisposition::Unknown,
    };
    FreeSpaceAssessment {
        required_bytes,
        available_bytes,
        disposition,
    }
}

fn enforce_space_policy(
    assessment: FreeSpaceAssessment,
    unknown_policy: UnknownSpacePolicy,
) -> Result<()> {
    match assessment.disposition {
        FreeSpaceDisposition::Enough => Ok(()),
        FreeSpaceDisposition::Insufficient => Err(BackupError::InsufficientSpace {
            required: assessment.required_bytes,
            available: assessment.available_bytes.unwrap_or(0),
        }),
        FreeSpaceDisposition::Unknown
            if unknown_policy == UnknownSpacePolicy::ProceedWithExplicitUnknown =>
        {
            Ok(())
        }
        FreeSpaceDisposition::Unknown => Err(BackupError::FreeSpaceUnknown),
    }
}

fn copy_snapshot_objects<F>(
    request: &ExportRequest<'_>,
    partial: &Path,
    catalog_path: &Path,
    journal_path: &Path,
    progress: &mut BackupProgress,
    expected_objects: u64,
    observe: &mut F,
) -> Result<()>
where
    F: FnMut(&BackupProgress) -> Control,
{
    let mut cursor = progress.last_verified_object_hash.clone();
    loop {
        require_snapshot_lease(request, progress, expected_objects)?;
        let objects = catalog_objects_after(catalog_path, cursor.as_deref(), OBJECT_PAGE_SIZE)?;
        if objects.is_empty() {
            break;
        }
        for object in objects {
            if progress.verified_objects >= MAX_BACKUP_ASSET_OBJECTS as u64 {
                return Err(BackupError::InvalidManifest {
                    reason: "asset snapshot exceeds the v1 object limit",
                });
            }
            let expected_relative = object_relative_path(&object.hash);
            if object.relative_path != expected_relative {
                return Err(BackupError::InvalidDatabase {
                    database: "asset catalog",
                    reason: "active object path is not hash-derived",
                });
            }
            let package_relative = format!("data/assets/{expected_relative}");
            validate_portable_relative_path(&package_relative)?;
            let target = partial.join(&package_relative);
            ensure_parent(&target)?;
            let mut verified_existing = false;
            if target.is_file() {
                let (hash, size) = hash_file_cancellable(&target, || {
                    matches!(observe(progress), Control::Cancel)
                })?;
                if hash == object.hash.as_str() && size == object.size {
                    verified_existing = true;
                } else {
                    fs::remove_file(&target)?;
                }
            }
            if !verified_existing {
                let temporary = target.with_extension("copying");
                remove_regular_file_if_present(&temporary)?;
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                let exported = request.assets.export_object(&object.hash, &mut output, || {
                    matches!(observe(progress), Control::Cancel)
                });
                match exported {
                    Ok(exported) => {
                        output.sync_all()?;
                        drop(output);
                        let (actual_hash, actual_size) = hash_file_cancellable(&temporary, || {
                            matches!(observe(progress), Control::Cancel)
                        })?;
                        if actual_hash != object.hash.as_str()
                            || actual_size != object.size
                            || exported.bytes_written != object.size
                        {
                            fs::remove_file(&temporary)?;
                            return Err(BackupError::EntryMismatch {
                                path: package_relative,
                                kind: "hash or size",
                            });
                        }
                        fs::rename(&temporary, &target)?;
                        sync_directory(target.parent().expect("object has parent"))?;
                    }
                    Err(error) => {
                        drop(output);
                        match fs::remove_file(&temporary) {
                            Ok(()) => {}
                            Err(remove_error) if remove_error.kind() == ErrorKind::NotFound => {}
                            Err(_) => {}
                        }
                        return Err(error.into());
                    }
                }
            }
            progress.last_verified_object_hash = Some(object.hash.to_string());
            progress.verified_objects = progress
                .verified_objects
                .checked_add(1)
                .ok_or(BackupError::SizeOverflow)?;
            progress.verified_bytes = progress
                .verified_bytes
                .checked_add(object.size)
                .ok_or(BackupError::SizeOverflow)?;
            write_progress(journal_path, progress)?;
            notify(progress, observe)?;
            cursor = progress.last_verified_object_hash.clone();
        }
    }
    Ok(())
}

fn build_manifest<F>(
    root: &Path,
    catalog_path: &Path,
    progress: &BackupProgress,
    compatibility_receipts: Vec<CompatibilityReceipt>,
    observe: &mut F,
) -> Result<BackupManifest>
where
    F: FnMut(&BackupProgress) -> Control,
{
    let product_path = root.join(PRODUCT_DATABASE_PATH);
    let (product_hash, product_size) = hash_file_cancellable(&product_path, || {
        matches!(observe(progress), Control::Cancel)
    })?;
    let (catalog_hash, catalog_size) = hash_file_cancellable(catalog_path, || {
        matches!(observe(progress), Control::Cancel)
    })?;
    let mut entries = Vec::new();
    let mut path_bytes = 0usize;
    push_manifest_entry(
        &mut entries,
        &mut path_bytes,
        BackupEntry {
            path: PRODUCT_DATABASE_PATH.to_owned(),
            size: product_size,
            sha256: product_hash.clone(),
            kind: "product_database".to_owned(),
        },
    )?;
    push_manifest_entry(
        &mut entries,
        &mut path_bytes,
        BackupEntry {
            path: ASSET_CATALOG_PATH.to_owned(),
            size: catalog_size,
            sha256: catalog_hash.clone(),
            kind: "asset_catalog".to_owned(),
        },
    )?;
    let mut cursor = None;
    loop {
        let objects = catalog_objects_after(catalog_path, cursor.as_deref(), OBJECT_PAGE_SIZE)?;
        if objects.is_empty() {
            break;
        }
        for object in objects {
            let relative = format!("data/assets/{}", object_relative_path(&object.hash));
            let (hash, size) = hash_file_cancellable(&root.join(&relative), || {
                matches!(observe(progress), Control::Cancel)
            })?;
            if hash != object.hash.as_str() || size != object.size {
                return Err(BackupError::EntryMismatch {
                    path: relative,
                    kind: "catalog hash or size",
                });
            }
            push_manifest_entry(
                &mut entries,
                &mut path_bytes,
                BackupEntry {
                    path: relative,
                    size,
                    sha256: hash,
                    kind: "asset_object".to_owned(),
                },
            )?;
            cursor = Some(object.hash.to_string());
        }
    }
    for (path, kind) in [
        (JOURNAL_NAME, "progress_receipt"),
        (COMPATIBILITY_RECEIPT_PATH, "compatibility_receipt"),
    ] {
        let (hash, size) = hash_file_cancellable(&root.join(path), || {
            matches!(observe(progress), Control::Cancel)
        })?;
        push_manifest_entry(
            &mut entries,
            &mut path_bytes,
            BackupEntry {
                path: path.to_owned(),
                size,
                sha256: hash,
                kind: kind.to_owned(),
            },
        )?;
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    validate_manifest_budget(&entries)?;
    let total_entry_bytes = entries.iter().try_fold(0u64, |sum, entry| {
        sum.checked_add(entry.size).ok_or(BackupError::SizeOverflow)
    })?;
    Ok(BackupManifest {
        format: "lorepia-directory-backup".to_owned(),
        format_version: BACKUP_FORMAT_VERSION,
        session_id: progress.session_id.clone(),
        product_database: ManifestDatabase {
            path: PRODUCT_DATABASE_PATH.to_owned(),
            schema_version: database_version(&product_path)?,
            sha256: product_hash,
            size: product_size,
        },
        asset_catalog: ManifestDatabase {
            path: ASSET_CATALOG_PATH.to_owned(),
            schema_version: database_version(catalog_path)?,
            sha256: catalog_hash,
            size: catalog_size,
        },
        entries,
        total_entry_bytes,
        snapshot_contract: SnapshotContract::default(),
        compatibility_receipts,
    })
}

fn verify_manifest_entries<F>(
    root: &Path,
    manifest: &BackupManifest,
    progress: &BackupProgress,
    observe: &mut F,
) -> Result<()>
where
    F: FnMut(&BackupProgress) -> Control,
{
    for entry in &manifest.entries {
        validate_portable_relative_path(&entry.path)?;
        let (hash, size) = hash_file_cancellable(&root.join(&entry.path), || {
            matches!(observe(progress), Control::Cancel)
        })?;
        if hash != entry.sha256 {
            return Err(BackupError::EntryMismatch {
                path: entry.path.clone(),
                kind: "hash",
            });
        }
        if size != entry.size {
            return Err(BackupError::EntryMismatch {
                path: entry.path.clone(),
                kind: "size",
            });
        }
    }
    Ok(())
}

fn object_relative_path(hash: &AssetHash) -> String {
    format!(
        "objects/{}/{}/{}",
        &hash.as_str()[..2],
        &hash.as_str()[2..4],
        hash.as_str()
    )
}

fn notify<F>(progress: &BackupProgress, observe: &mut F) -> Result<()>
where
    F: FnMut(&BackupProgress) -> Control,
{
    match observe(progress) {
        Control::Continue => Ok(()),
        Control::Cancel => Err(BackupError::Cancelled),
    }
}

fn ensure_parent(path: &Path) -> Result<()> {
    let parent = path.parent().ok_or(BackupError::InvalidInput {
        field: "backup entry path",
        reason: "must have a parent directory",
    })?;
    fs::create_dir_all(parent)?;
    reject_symlink_directory(parent)
}

fn remove_regular_file_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(BackupError::UnsafePath {
                path: path.display().to_string(),
            })
        }
        Ok(_) => {
            fs::remove_file(path)?;
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_accounting_handles_hundred_gibibytes_without_allocation() {
        let required = 10u64
            .checked_mul(1024 * 1024 * 1024)
            .and_then(|value| value.checked_add(100 * 1024 * 1024 * 1024))
            .unwrap();
        assert_eq!(
            assess_space(required, Some(required * 2)).disposition,
            FreeSpaceDisposition::Enough
        );
        assert_eq!(
            assess_space(required, Some(required + required / 5)).disposition,
            FreeSpaceDisposition::Enough
        );
        assert_eq!(
            assess_space(required, Some(required - 1)).disposition,
            FreeSpaceDisposition::Insufficient
        );
    }

    #[test]
    fn asset_object_limit_is_checked_before_copy_contract() {
        assert!(validate_catalog_object_count(MAX_BACKUP_ASSET_OBJECTS as u64).is_ok());
        assert!(matches!(
            validate_catalog_object_count(MAX_BACKUP_ASSET_OBJECTS as u64 + 1),
            Err(BackupError::InvalidManifest { .. })
        ));
    }
}
