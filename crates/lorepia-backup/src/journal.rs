use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

use crate::{
    BackupError, BackupProgress, Operation, Phase, Result,
    fsutil::{atomic_write, canonical_json, reject_symlink_file, validate_portable_relative_path},
    model::{MAX_BACKUP_ASSET_OBJECTS, MAX_BACKUP_JOURNAL_BYTES},
};

pub(crate) fn read_progress(path: &Path) -> Result<BackupProgress> {
    reject_symlink_file(path)?;
    if fs::metadata(path)?.len() > MAX_BACKUP_JOURNAL_BYTES {
        return Err(BackupError::JournalConflict);
    }
    let mut bytes = Vec::with_capacity(MAX_BACKUP_JOURNAL_BYTES as usize);
    File::open(path)?
        .take(MAX_BACKUP_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BACKUP_JOURNAL_BYTES {
        return Err(BackupError::JournalConflict);
    }
    let progress: BackupProgress =
        serde_json::from_slice(&bytes).map_err(|_| BackupError::JournalConflict)?;
    validate_progress(&progress)?;
    Ok(progress)
}

pub(crate) fn write_progress(path: &Path, progress: &BackupProgress) -> Result<()> {
    validate_progress(progress)?;
    let bytes = canonical_json(progress)?;
    if bytes.len() as u64 > MAX_BACKUP_JOURNAL_BYTES {
        return Err(BackupError::JournalConflict);
    }
    atomic_write(path, &bytes)
}

fn validate_progress(progress: &BackupProgress) -> Result<()> {
    if progress.journal_version != 1
        || !valid_lower_hex(&progress.session_id, 32)
        || progress.verified_objects > MAX_BACKUP_ASSET_OBJECTS as u64
        || progress.verified_bytes > progress.required_bytes
        || validate_portable_relative_path(&progress.destination_name).is_err()
        || progress
            .last_verified_object_hash
            .as_deref()
            .is_some_and(|value| !valid_lower_hex(value, 64))
    {
        return Err(BackupError::JournalConflict);
    }
    match progress.operation {
        Operation::Export => {
            if progress.source_manifest_sha256.is_some()
                || !matches!(
                    progress.phase,
                    Phase::Prepared
                        | Phase::ProductSnapshot
                        | Phase::AssetCatalogSnapshot
                        | Phase::Objects
                        | Phase::ReadyToPublish
                        | Phase::Verified
                )
            {
                return Err(BackupError::JournalConflict);
            }
        }
        Operation::Restore => {
            if !progress
                .source_manifest_sha256
                .as_deref()
                .is_some_and(|value| valid_lower_hex(value, 64))
                || !matches!(
                    progress.phase,
                    Phase::Prepared
                        | Phase::Copied
                        | Phase::Verified
                        | Phase::OldMoved
                        | Phase::NewPublished
                        | Phase::Complete
                )
            {
                return Err(BackupError::JournalConflict);
            }
        }
    }
    Ok(())
}

fn valid_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}
