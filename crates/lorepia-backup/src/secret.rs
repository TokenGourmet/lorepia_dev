use std::{
    fmt,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use crate::{BackupError, Result, model::COPY_BUFFER_BYTES};

/// A byte sequence that must never occur in an exported database, manifest, or object.
/// Debug output is deliberately redacted and scanner errors never include sentinel contents.
#[derive(Clone, Eq, PartialEq)]
pub struct SecretSentinel {
    bytes: Vec<u8>,
}

impl SecretSentinel {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let bytes = bytes.into();
        if bytes.len() < 4 {
            return Err(BackupError::InvalidInput {
                field: "secret sentinel",
                reason: "must contain at least four bytes",
            });
        }
        Ok(Self { bytes })
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for SecretSentinel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretSentinel")
            .field("bytes", &"[REDACTED]")
            .field("length", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SecretScanReport {
    pub files_scanned: u64,
    pub bytes_scanned: u64,
    pub matches: u64,
}

pub fn scan_paths_for_secrets(
    entries: &[(String, PathBuf)],
    sentinels: &[SecretSentinel],
) -> Result<SecretScanReport> {
    scan_owned_paths_for_secrets(entries.iter().cloned(), sentinels)
}

pub(crate) fn scan_owned_paths_for_secrets(
    entries: impl IntoIterator<Item = (String, PathBuf)>,
    sentinels: &[SecretSentinel],
) -> Result<SecretScanReport> {
    scan_owned_paths_for_secrets_cancellable(entries, sentinels, || false)
}

pub(crate) fn scan_owned_paths_for_secrets_cancellable<F>(
    entries: impl IntoIterator<Item = (String, PathBuf)>,
    sentinels: &[SecretSentinel],
    mut is_cancelled: F,
) -> Result<SecretScanReport>
where
    F: FnMut() -> bool,
{
    let mut report = SecretScanReport::default();
    if sentinels.is_empty() {
        for (_, path) in entries {
            if is_cancelled() {
                return Err(BackupError::Cancelled);
            }
            report.files_scanned = report.files_scanned.saturating_add(1);
            report.bytes_scanned = report
                .bytes_scanned
                .checked_add(path.metadata()?.len())
                .ok_or(BackupError::SizeOverflow)?;
        }
        return Ok(report);
    }
    for (label, path) in entries {
        let (bytes, found) = scan_file(&path, sentinels, &mut is_cancelled)?;
        report.files_scanned = report.files_scanned.saturating_add(1);
        report.bytes_scanned = report
            .bytes_scanned
            .checked_add(bytes)
            .ok_or(BackupError::SizeOverflow)?;
        if found {
            return Err(BackupError::SecretFound { entry: label });
        }
    }
    Ok(report)
}

fn scan_file<F>(
    path: &Path,
    sentinels: &[SecretSentinel],
    is_cancelled: &mut F,
) -> Result<(u64, bool)>
where
    F: FnMut() -> bool,
{
    let mut file = File::open(path)?;
    let max_pattern = sentinels
        .iter()
        .map(|sentinel| sentinel.as_bytes().len())
        .max()
        .unwrap_or(1);
    let mut carry = Vec::new();
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut scanned = 0u64;
    loop {
        if is_cancelled() {
            return Err(BackupError::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        scanned = scanned
            .checked_add(read as u64)
            .ok_or(BackupError::SizeOverflow)?;
        carry.extend_from_slice(&buffer[..read]);
        if sentinels
            .iter()
            .any(|sentinel| contains(&carry, sentinel.as_bytes()))
        {
            return Ok((scanned, true));
        }
        let retain = max_pattern.saturating_sub(1).min(carry.len());
        carry.drain(..carry.len().saturating_sub(retain));
    }
    Ok((scanned, false))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn sentinel_crossing_read_boundary_is_detected_without_disclosure() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("blob");
        let mut file = File::create(&path).unwrap();
        file.write_all(&vec![b'x'; COPY_BUFFER_BYTES - 3]).unwrap();
        file.write_all(b"secret-value").unwrap();
        drop(file);
        let sentinel = SecretSentinel::new(b"secret-value".to_vec()).unwrap();
        let error = scan_paths_for_secrets(
            &[("object".to_owned(), path)],
            std::slice::from_ref(&sentinel),
        )
        .unwrap_err();
        assert!(matches!(error, BackupError::SecretFound { .. }));
        assert!(!format!("{sentinel:?}").contains("secret-value"));
        assert!(!error.to_string().contains("secret-value"));
        fs::remove_dir_all(directory).unwrap();
    }
}
