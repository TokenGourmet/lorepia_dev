use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

use lorepia_assets::AssetHash;

use crate::{
    BackupError, BackupManifest, BackupProgress, Control, FreeSpaceDisposition, Operation, Phase,
    RestoreOptions, RestorePolicy, RestoreReport, RestoreRequest, Result, UnknownSpacePolicy,
    fsutil::{
        directory_is_empty, hash_file_cancellable, list_tree_files_bounded,
        reject_symlink_directory, reject_symlink_file, reject_symlink_if_present, sha256_bytes,
        sync_directory, sync_tree, validate_no_case_collisions, validate_portable_relative_path,
    },
    journal::{read_progress, write_progress},
    manifest::{
        canonical_manifest_matches, preflight_manifest, read_bounded_manifest,
        validate_manifest_budget,
    },
    model::{
        BACKUP_FORMAT_VERSION, COPY_BUFFER_BYTES, MAX_BACKUP_ASSET_OBJECTS,
        MAX_BACKUP_MANIFEST_ENTRIES, MAX_BACKUP_MANIFEST_PATH_BYTES, SPACE_RESERVE_BYTES,
    },
    secret::scan_owned_paths_for_secrets_cancellable,
    sqlite::{catalog_objects_after, validate_asset_catalog, validate_product_database},
};

const MANIFEST_PATH: &str = "manifest.json";
const MANIFEST_HASH_PATH: &str = "manifest.sha256";
const PRODUCT_DATABASE_PATH: &str = "data/product.sqlite3";
const ASSET_CATALOG_PATH: &str = "data/assets/assets.sqlite3";
const PROGRESS_RECEIPT_PATH: &str = "progress.json";
const COMPATIBILITY_RECEIPT_PATH: &str = "receipts/compatibility.json";
const RESTORED_PRODUCT_PATH: &str = "product.sqlite3";
const RESTORED_ASSET_CATALOG_PATH: &str = "assets/assets.sqlite3";
const OBJECT_PAGE_SIZE: u16 = 512;

pub fn restore_journal_path_for(destination: &Path) -> Result<PathBuf> {
    sibling_path(destination, "restore-journal.json")
}

pub fn restore_backup<F>(
    request: RestoreRequest<'_>,
    options: RestoreOptions,
    mut observe: F,
) -> Result<RestoreReport>
where
    F: FnMut(&BackupProgress) -> Control,
{
    let parent = request
        .destination
        .parent()
        .ok_or(BackupError::InvalidInput {
            field: "restore destination",
            reason: "must have a parent directory",
        })?;
    fs::create_dir_all(parent)?;
    reject_symlink_directory(parent)?;
    let destination_name = request
        .destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(BackupError::InvalidInput {
            field: "restore destination",
            reason: "must have a portable UTF-8 name",
        })?
        .to_owned();
    validate_portable_relative_path(&destination_name)?;

    let journal_path = restore_journal_path_for(request.destination)?;
    let staging = sibling_path(request.destination, "restore-partial")?;
    let old = sibling_path(request.destination, "restore-old")?;
    reject_symlink_if_present(&journal_path)?;
    reject_symlink_if_present(&staging)?;
    reject_symlink_if_present(&old)?;
    let resumed = journal_path.exists();
    if !resumed {
        if staging.exists() {
            if old.exists() || !directory_is_empty(&staging)? {
                return Err(BackupError::JournalConflict);
            }
            fs::remove_dir(&staging)?;
            sync_directory(parent)?;
        }
        if old.exists() {
            return Err(BackupError::JournalConflict);
        }
    }

    let mut saved_progress = if resumed {
        let mut journal = read_progress(&journal_path)?;
        if journal.operation != Operation::Restore || journal.destination_name != destination_name {
            return Err(BackupError::JournalConflict);
        }
        if old.exists() && !request.destination.exists() {
            if !staging.exists() || !matches!(journal.phase, Phase::Verified | Phase::OldMoved) {
                return Err(BackupError::JournalConflict);
            }
            reject_symlink_directory(&old)?;
            reject_symlink_directory(&staging)?;
            fs::rename(&old, request.destination)?;
            sync_directory(parent)?;
            journal.phase = Phase::Verified;
            journal.replaced_existing = true;
            write_progress(&journal_path, &journal)?;
        } else if journal.phase == Phase::OldMoved
            && journal.replaced_existing
            && request.destination.exists()
            && staging.exists()
            && !old.exists()
        {
            reject_symlink_directory(request.destination)?;
            reject_symlink_directory(&staging)?;
            journal.phase = Phase::Verified;
            write_progress(&journal_path, &journal)?;
        }
        Some(journal)
    } else {
        None
    };

    if paths_are_same(request.package, request.destination)? {
        return Err(BackupError::InvalidInput {
            field: "restore destination",
            reason: "must differ from the backup package",
        });
    }
    let verified = load_package_manifest(request.package)?;

    let mut progress = if let Some(journal) = saved_progress.take() {
        if journal.session_id != verified.manifest.session_id
            || journal.source_manifest_sha256.as_deref() != Some(verified.manifest_hash.as_str())
            || journal.verified_bytes > verified.manifest.total_entry_bytes
        {
            return Err(BackupError::JournalConflict);
        }
        journal
    } else {
        if request.destination.exists() && options.policy == RestorePolicy::FailIfPresent {
            return Err(BackupError::ExistingData(request.destination.to_path_buf()));
        }
        let replaced_existing = request.destination.exists();
        if replaced_existing {
            reject_symlink_directory(request.destination)?;
        }
        BackupProgress {
            journal_version: 1,
            operation: Operation::Restore,
            session_id: verified.manifest.session_id.clone(),
            phase: Phase::Prepared,
            destination_name: destination_name.clone(),
            source_manifest_sha256: Some(verified.manifest_hash.clone()),
            last_verified_object_hash: None,
            verified_objects: 0,
            verified_bytes: 0,
            required_bytes: verified.manifest.total_entry_bytes,
            available_bytes: None,
            replaced_existing,
        }
    };
    progress.required_bytes = verified.manifest.total_entry_bytes;
    progress.available_bytes = request.space_probe.available_bytes(parent)?;
    let remaining = if progress.phase < Phase::Copied {
        verified
            .manifest
            .total_entry_bytes
            .checked_sub(progress.verified_bytes)
            .ok_or(BackupError::JournalConflict)?
    } else {
        0
    };
    let remaining_with_reserve = remaining
        .checked_add(SPACE_RESERVE_BYTES)
        .ok_or(BackupError::SizeOverflow)?;
    enforce_restore_space(
        remaining_with_reserve,
        progress.available_bytes,
        options.unknown_space_policy,
    )?;
    if !resumed {
        fs::create_dir(&staging)?;
        sync_directory(parent)?;
    }
    write_progress(&journal_path, &progress)?;
    notify(&progress, &mut observe)?;
    verify_package_contents(request.package, &verified, request.secret_sentinels, || {
        matches!(observe(&progress), Control::Cancel)
    })?;

    // Reconcile a publish rename that became visible before its following journal update.
    if progress.phase == Phase::OldMoved && request.destination.exists() && !staging.exists() {
        validate_restored_data(request.destination, &verified.manifest, || {
            matches!(observe(&progress), Control::Cancel)
        })?;
        progress.phase = Phase::NewPublished;
        write_progress(&journal_path, &progress)?;
        notify(&progress, &mut observe)?;
    }
    if progress.phase < Phase::Copied {
        copy_data_entries(
            request.package,
            &staging,
            &verified.manifest,
            &journal_path,
            &mut progress,
            &mut observe,
        )?;
        progress.phase = Phase::Copied;
        write_progress(&journal_path, &progress)?;
        notify(&progress, &mut observe)?;
    }
    if progress.phase < Phase::Verified {
        validate_restored_data(&staging, &verified.manifest, || {
            matches!(observe(&progress), Control::Cancel)
        })?;
        progress.phase = Phase::Verified;
        write_progress(&journal_path, &progress)?;
        notify(&progress, &mut observe)?;
    }

    if progress.phase < Phase::OldMoved {
        if request.destination.exists() {
            if options.policy != RestorePolicy::Replace {
                return Err(BackupError::ExistingData(request.destination.to_path_buf()));
            }
            if old.exists() {
                return Err(BackupError::JournalConflict);
            }
            fs::rename(request.destination, &old)?;
            sync_directory(parent)?;
        }
        progress.phase = Phase::OldMoved;
        write_progress(&journal_path, &progress)?;
        notify(&progress, &mut observe)?;
    }

    if progress.phase < Phase::NewPublished {
        if request.destination.exists() || !staging.exists() {
            return Err(BackupError::JournalConflict);
        }
        sync_tree(&staging)?;
        fs::rename(&staging, request.destination)?;
        sync_directory(parent)?;
        progress.phase = Phase::NewPublished;
        write_progress(&journal_path, &progress)?;
        notify(&progress, &mut observe)?;
    }

    if let Err(validation_error) =
        validate_restored_data(request.destination, &verified.manifest, || {
            matches!(observe(&progress), Control::Cancel)
        })
    {
        if !rollback_publish(request.destination, &staging, &old, parent)? {
            return Err(BackupError::RollbackFailed);
        }
        progress.phase = Phase::Prepared;
        progress.last_verified_object_hash = None;
        progress.verified_objects = 0;
        progress.verified_bytes = 0;
        write_progress(&journal_path, &progress)?;
        return Err(validation_error);
    }
    if old.exists() {
        reject_symlink_directory(&old)?;
        fs::remove_dir_all(&old)?;
        sync_directory(parent)?;
    }
    progress.phase = Phase::Complete;
    write_progress(&journal_path, &progress)?;
    notify(&progress, &mut observe)?;
    fs::remove_file(&journal_path)?;
    sync_directory(parent)?;

    Ok(RestoreReport {
        destination: request.destination.to_path_buf(),
        session_id: verified.manifest.session_id,
        manifest_sha256: verified.manifest_hash,
        restored_bytes: verified.manifest.total_entry_bytes,
        replaced_existing: progress.replaced_existing,
        resumed,
    })
}

struct VerifiedPackage {
    manifest: BackupManifest,
    manifest_hash: String,
}

fn read_manifest_checksum(path: &Path) -> Result<String> {
    const MAX_CHECKSUM_BYTES: u64 = 66;
    reject_symlink_file(path)?;
    if fs::metadata(path)?.len() > MAX_CHECKSUM_BYTES {
        return Err(BackupError::InvalidManifest {
            reason: "manifest checksum exceeds 66 bytes",
        });
    }
    let mut checksum = String::new();
    File::open(path)?
        .take(MAX_CHECKSUM_BYTES + 1)
        .read_to_string(&mut checksum)?;
    if checksum.len() as u64 > MAX_CHECKSUM_BYTES {
        return Err(BackupError::InvalidManifest {
            reason: "manifest checksum exceeds 66 bytes",
        });
    }
    Ok(checksum)
}

fn load_package_manifest(package: &Path) -> Result<VerifiedPackage> {
    reject_symlink_directory(package)?;
    let manifest_path = package.join(MANIFEST_PATH);
    let checksum_path = package.join(MANIFEST_HASH_PATH);
    let manifest_bytes = read_bounded_manifest(&manifest_path)?;
    let actual_hash = sha256_bytes(&manifest_bytes);
    let checksum = read_manifest_checksum(&checksum_path)?;
    if checksum.trim_end_matches(['\r', '\n']) != actual_hash
        || checksum.trim_end_matches(['\r', '\n']).len() != 64
    {
        return Err(BackupError::EntryMismatch {
            path: MANIFEST_PATH.to_owned(),
            kind: "manifest hash",
        });
    }
    preflight_manifest(&manifest_bytes)?;
    let mut manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    match manifest.format_version {
        BACKUP_FORMAT_VERSION => {
            if !canonical_manifest_matches(&manifest_bytes, &manifest)? {
                return Err(BackupError::InvalidManifest {
                    reason: "current-version manifest is not canonical JSON",
                });
            }
        }
        0 => {
            // v0 used the same fields but did not require canonical JSON. Normalize in memory;
            // data entries and the original manifest checksum remain fully verified.
            manifest.format_version = BACKUP_FORMAT_VERSION;
        }
        found if found > BACKUP_FORMAT_VERSION => {
            return Err(BackupError::FutureVersion {
                found,
                supported: BACKUP_FORMAT_VERSION,
            });
        }
        found => return Err(BackupError::UnsupportedVersion(found)),
    }
    validate_manifest(&manifest)?;

    let mut expected = manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .chain([MANIFEST_PATH, MANIFEST_HASH_PATH])
        .collect::<Vec<_>>();
    expected.sort_unstable();
    let actual = list_tree_files_bounded(
        package,
        MAX_BACKUP_MANIFEST_ENTRIES + 2,
        MAX_BACKUP_MANIFEST_PATH_BYTES + MANIFEST_PATH.len() + MANIFEST_HASH_PATH.len(),
    )?;
    if actual.len() != expected.len()
        || !actual
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
    {
        return Err(BackupError::InvalidManifest {
            reason: "package contains missing or unmanifested files",
        });
    }
    Ok(VerifiedPackage {
        manifest,
        manifest_hash: actual_hash,
    })
}

fn verify_package_contents<F>(
    package: &Path,
    verified: &VerifiedPackage,
    sentinels: &[crate::SecretSentinel],
    mut is_cancelled: F,
) -> Result<()>
where
    F: FnMut() -> bool,
{
    let manifest = &verified.manifest;
    let manifest_path = package.join(MANIFEST_PATH);
    let checksum_path = package.join(MANIFEST_HASH_PATH);
    for entry in &manifest.entries {
        let (hash, size) = hash_file_cancellable(&package.join(&entry.path), &mut is_cancelled)?;
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
    validate_product_database(&package.join(PRODUCT_DATABASE_PATH))?;
    validate_asset_catalog(&package.join(ASSET_CATALOG_PATH))?;
    validate_catalog_manifest(package, manifest, &mut is_cancelled)?;
    scan_owned_paths_for_secrets_cancellable(
        manifest
            .entries
            .iter()
            .map(|entry| (entry.path.clone(), package.join(&entry.path)))
            .chain([
                (MANIFEST_PATH.to_owned(), manifest_path),
                (MANIFEST_HASH_PATH.to_owned(), checksum_path),
            ]),
        sentinels,
        is_cancelled,
    )?;
    Ok(())
}

fn validate_manifest(manifest: &BackupManifest) -> Result<()> {
    validate_manifest_budget(&manifest.entries)?;
    if manifest.format != "lorepia-directory-backup" || !is_session_id(&manifest.session_id) {
        return Err(BackupError::InvalidManifest {
            reason: "format marker or session id is invalid",
        });
    }
    if manifest.product_database.path != PRODUCT_DATABASE_PATH
        || manifest.asset_catalog.path != ASSET_CATALOG_PATH
    {
        return Err(BackupError::InvalidManifest {
            reason: "database paths are not the fixed portable paths",
        });
    }
    if !manifest.compatibility_receipts.iter().any(|receipt| {
        receipt.check_id == "BACKUP-007" && receipt.disposition == "not_applicable_by_design"
    }) {
        return Err(BackupError::InvalidManifest {
            reason: "directory-format compatibility receipt is missing",
        });
    }
    validate_no_case_collisions(manifest.entries.iter().map(|entry| entry.path.as_str()))?;
    let mut previous = None;
    let mut total = 0u64;
    let mut product_entries = 0usize;
    let mut catalog_entries = 0usize;
    let mut progress_receipts = 0usize;
    let mut compatibility_receipts = 0usize;
    let mut object_entries = 0usize;
    for entry in &manifest.entries {
        validate_portable_relative_path(&entry.path)?;
        if previous.is_some_and(|value: &str| value >= entry.path.as_str()) {
            return Err(BackupError::InvalidManifest {
                reason: "entries are not in strict path order",
            });
        }
        if !valid_sha256(&entry.sha256) {
            return Err(BackupError::InvalidManifest {
                reason: "entry hash is not lowercase SHA-256",
            });
        }
        total = total
            .checked_add(entry.size)
            .ok_or(BackupError::SizeOverflow)?;
        match (entry.path.as_str(), entry.kind.as_str()) {
            (PRODUCT_DATABASE_PATH, "product_database") => product_entries += 1,
            (ASSET_CATALOG_PATH, "asset_catalog") => catalog_entries += 1,
            (PROGRESS_RECEIPT_PATH, "progress_receipt") => progress_receipts += 1,
            (COMPATIBILITY_RECEIPT_PATH, "compatibility_receipt") => {
                compatibility_receipts += 1;
            }
            (path, "asset_object") if path.starts_with("data/assets/objects/") => {
                object_entries += 1;
                if object_entries > MAX_BACKUP_ASSET_OBJECTS {
                    return Err(BackupError::InvalidManifest {
                        reason: "asset object count exceeds the v1 limit",
                    });
                }
            }
            _ => {
                return Err(BackupError::InvalidManifest {
                    reason: "manifest contains an unsupported entry path or kind",
                });
            }
        }
        previous = Some(entry.path.as_str());
    }
    if product_entries != 1
        || catalog_entries != 1
        || progress_receipts != 1
        || compatibility_receipts != 1
    {
        return Err(BackupError::InvalidManifest {
            reason: "manifest mandatory entries are missing or duplicated",
        });
    }
    if total != manifest.total_entry_bytes {
        return Err(BackupError::InvalidManifest {
            reason: "entry byte total does not match",
        });
    }
    for database in [&manifest.product_database, &manifest.asset_catalog] {
        let entry = manifest
            .entries
            .iter()
            .find(|entry| entry.path == database.path)
            .ok_or(BackupError::InvalidManifest {
                reason: "database entry is missing",
            })?;
        if entry.sha256 != database.sha256 || entry.size != database.size {
            return Err(BackupError::InvalidManifest {
                reason: "database descriptor disagrees with its entry",
            });
        }
    }
    Ok(())
}

fn validate_catalog_manifest<F>(
    package: &Path,
    manifest: &BackupManifest,
    mut is_cancelled: F,
) -> Result<()>
where
    F: FnMut() -> bool,
{
    let catalog = package.join(ASSET_CATALOG_PATH);
    let mut object_entries = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == "asset_object");
    let mut cursor = None;
    loop {
        if is_cancelled() {
            return Err(BackupError::Cancelled);
        }
        let objects = catalog_objects_after(&catalog, cursor.as_deref(), OBJECT_PAGE_SIZE)?;
        if objects.is_empty() {
            break;
        }
        for object in objects {
            if is_cancelled() {
                return Err(BackupError::Cancelled);
            }
            let expected = format!("data/assets/{}", object_relative_path(&object.hash));
            let entry = object_entries.next().ok_or(BackupError::InvalidManifest {
                reason: "catalogued active object is absent from manifest",
            })?;
            if entry.path != expected {
                return Err(BackupError::InvalidManifest {
                    reason: "catalogued active object is absent from manifest",
                });
            }
            if entry.size != object.size || entry.sha256 != object.hash.as_str() {
                return Err(BackupError::InvalidManifest {
                    reason: "catalog object metadata disagrees with manifest",
                });
            }
            cursor = Some(object.hash.to_string());
        }
    }
    if object_entries.next().is_some() {
        return Err(BackupError::InvalidManifest {
            reason: "manifest contains an object absent from the catalog snapshot",
        });
    }
    Ok(())
}

fn copy_data_entries<F>(
    package: &Path,
    staging: &Path,
    manifest: &BackupManifest,
    journal_path: &Path,
    progress: &mut BackupProgress,
    observe: &mut F,
) -> Result<()>
where
    F: FnMut(&BackupProgress) -> Control,
{
    reject_symlink_directory(staging)?;
    progress.last_verified_object_hash = None;
    progress.verified_objects = 0;
    progress.verified_bytes = 0;
    for entry in manifest
        .entries
        .iter()
        .filter(|entry| entry.path.starts_with("data/"))
    {
        let restored_relative =
            entry
                .path
                .strip_prefix("data/")
                .ok_or(BackupError::InvalidManifest {
                    reason: "data entry prefix is invalid",
                })?;
        validate_portable_relative_path(restored_relative)?;
        let source = package.join(&entry.path);
        let target = staging.join(restored_relative);
        let parent = target.parent().ok_or(BackupError::UnsafePath {
            path: target.display().to_string(),
        })?;
        fs::create_dir_all(parent)?;
        reject_symlink_directory(parent)?;
        let already_valid = if target.is_file() {
            let (hash, size) =
                hash_file_cancellable(&target, || matches!(observe(progress), Control::Cancel))?;
            hash == entry.sha256 && size == entry.size
        } else {
            false
        };
        if !already_valid {
            remove_regular_file_if_present(&target)?;
            copy_verified(&source, &target, entry.size, &entry.sha256, || {
                matches!(observe(progress), Control::Cancel)
            })?;
        }
        progress.verified_bytes = progress
            .verified_bytes
            .checked_add(entry.size)
            .ok_or(BackupError::SizeOverflow)?;
        if entry.kind == "asset_object" {
            progress.verified_objects = progress
                .verified_objects
                .checked_add(1)
                .ok_or(BackupError::SizeOverflow)?;
            progress.last_verified_object_hash = Some(entry.sha256.clone());
        }
        write_progress(journal_path, progress)?;
        notify(progress, observe)?;
    }
    Ok(())
}

fn copy_verified<F>(
    source: &Path,
    target: &Path,
    size: u64,
    hash: &str,
    mut is_cancelled: F,
) -> Result<()>
where
    F: FnMut() -> bool,
{
    let temporary = target.with_extension("copying");
    remove_regular_file_if_present(&temporary)?;
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    loop {
        if is_cancelled() {
            return Err(BackupError::Cancelled);
        }
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
    }
    output.sync_all()?;
    drop(output);
    let (actual_hash, actual_size) = hash_file_cancellable(&temporary, is_cancelled)?;
    if actual_hash != hash || actual_size != size {
        fs::remove_file(&temporary)?;
        return Err(BackupError::EntryMismatch {
            path: source.display().to_string(),
            kind: "copy hash or size",
        });
    }
    fs::rename(&temporary, target)?;
    sync_directory(target.parent().expect("restore entry has parent"))?;
    Ok(())
}

fn validate_restored_data<F>(
    root: &Path,
    manifest: &BackupManifest,
    mut is_cancelled: F,
) -> Result<()>
where
    F: FnMut() -> bool,
{
    if is_cancelled() {
        return Err(BackupError::Cancelled);
    }
    reject_symlink_directory(root)?;
    let expected_count = manifest
        .entries
        .iter()
        .filter(|entry| entry.path.starts_with("data/"))
        .count();
    let expected_path_bytes = manifest
        .entries
        .iter()
        .filter_map(|entry| entry.path.strip_prefix("data/"))
        .try_fold(0usize, |total, path| {
            total
                .checked_add(path.len())
                .ok_or(BackupError::SizeOverflow)
        })?;
    let actual = list_tree_files_bounded(root, expected_count, expected_path_bytes)?;
    if actual.len() != expected_count
        || !actual.iter().map(String::as_str).eq(manifest
            .entries
            .iter()
            .filter_map(|entry| entry.path.strip_prefix("data/")))
    {
        return Err(BackupError::InvalidManifest {
            reason: "restored data has missing or unexpected files",
        });
    }
    for entry in manifest
        .entries
        .iter()
        .filter(|entry| entry.path.starts_with("data/"))
    {
        if is_cancelled() {
            return Err(BackupError::Cancelled);
        }
        let relative = entry.path.strip_prefix("data/").expect("prefix checked");
        let (hash, size) = hash_file_cancellable(&root.join(relative), &mut is_cancelled)?;
        if hash != entry.sha256 || size != entry.size {
            return Err(BackupError::EntryMismatch {
                path: relative.to_owned(),
                kind: "restored hash or size",
            });
        }
    }
    if is_cancelled() {
        return Err(BackupError::Cancelled);
    }
    validate_product_database(&root.join(RESTORED_PRODUCT_PATH))?;
    if is_cancelled() {
        return Err(BackupError::Cancelled);
    }
    validate_asset_catalog(&root.join(RESTORED_ASSET_CATALOG_PATH))?;
    Ok(())
}

fn rollback_publish(destination: &Path, staging: &Path, old: &Path, parent: &Path) -> Result<bool> {
    if destination.exists() {
        if staging.exists() {
            return Ok(false);
        }
        fs::rename(destination, staging)?;
        sync_directory(parent)?;
    }
    if old.exists() {
        fs::rename(old, destination)?;
        sync_directory(parent)?;
    }
    Ok(true)
}

fn enforce_restore_space(
    required_bytes: u64,
    available_bytes: Option<u64>,
    unknown_policy: UnknownSpacePolicy,
) -> Result<()> {
    let disposition = match available_bytes {
        Some(available) if available >= required_bytes => FreeSpaceDisposition::Enough,
        Some(_) => FreeSpaceDisposition::Insufficient,
        None => FreeSpaceDisposition::Unknown,
    };
    match disposition {
        FreeSpaceDisposition::Enough => Ok(()),
        FreeSpaceDisposition::Insufficient => Err(BackupError::InsufficientSpace {
            required: required_bytes,
            available: available_bytes.unwrap_or(0),
        }),
        FreeSpaceDisposition::Unknown
            if unknown_policy == UnknownSpacePolicy::ProceedWithExplicitUnknown =>
        {
            Ok(())
        }
        FreeSpaceDisposition::Unknown => Err(BackupError::FreeSpaceUnknown),
    }
}

fn sibling_path(destination: &Path, suffix: &str) -> Result<PathBuf> {
    let parent = destination.parent().ok_or(BackupError::InvalidInput {
        field: "restore destination",
        reason: "must have a parent directory",
    })?;
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(BackupError::InvalidInput {
            field: "restore destination",
            reason: "must have a UTF-8 file name",
        })?;
    validate_portable_relative_path(name)?;
    Ok(parent.join(format!(".{name}.{suffix}")))
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

fn object_relative_path(hash: &AssetHash) -> String {
    format!(
        "objects/{}/{}/{}",
        &hash.as_str()[..2],
        &hash.as_str()[2..4],
        hash.as_str()
    )
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

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn is_session_id(value: &str) -> bool {
    value.len() == 32
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn paths_are_same(left: &Path, right: &Path) -> Result<bool> {
    let left = fs::canonicalize(left)?;
    let right = if right.exists() {
        fs::canonicalize(right)?
    } else {
        let parent = right.parent().ok_or(BackupError::InvalidInput {
            field: "restore destination",
            reason: "must have a parent directory",
        })?;
        fs::canonicalize(parent)?.join(right.file_name().ok_or(BackupError::InvalidInput {
            field: "restore destination",
            reason: "must name a directory",
        })?)
    };
    Ok(left == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_entry_copy_observes_cancellation_between_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let target = directory.path().join("target");
        fs::write(&source, vec![b'x'; COPY_BUFFER_BYTES * 3]).unwrap();
        let mut calls = 0usize;
        let error = copy_verified(&source, &target, 0, &"0".repeat(64), || {
            calls += 1;
            calls >= 2
        })
        .unwrap_err();
        assert!(matches!(error, BackupError::Cancelled));
        assert!(!target.exists());
    }
}
