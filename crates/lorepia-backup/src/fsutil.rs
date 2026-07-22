use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Component, Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::{Builder, NamedTempFile};

use crate::{
    BackupError, Result,
    model::{COPY_BUFFER_BYTES, MAX_PORTABLE_PATH_BYTES},
};

pub(crate) fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn hash_file_cancellable<F>(path: &Path, mut is_cancelled: F) -> Result<(String, u64)>
where
    F: FnMut() -> bool,
{
    reject_symlink_file(path)?;
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut size = 0u64;
    loop {
        if is_cancelled() {
            return Err(BackupError::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        size = size
            .checked_add(read as u64)
            .ok_or(BackupError::SizeOverflow)?;
    }
    Ok((hex::encode(digest.finalize()), size))
}

pub(crate) fn directory_is_empty(path: &Path) -> Result<bool> {
    reject_symlink_directory(path)?;
    Ok(fs::read_dir(path)?.next().transpose()?.is_none())
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temporary = AtomicWriteFile::create(path)?;
    temporary.file_mut().write_all(bytes)?;
    temporary.commit()
}

pub(crate) struct AtomicWriteFile {
    destination: PathBuf,
    temporary: Option<NamedTempFile>,
}

impl AtomicWriteFile {
    pub(crate) fn create(path: &Path) -> Result<Self> {
        let parent = path.parent().ok_or(BackupError::InvalidInput {
            field: "atomic write path",
            reason: "must have a parent directory",
        })?;
        fs::create_dir_all(parent)?;
        let name = path.file_name().ok_or(BackupError::InvalidInput {
            field: "atomic write path",
            reason: "must name a file",
        })?;
        let mut prefix = name.to_os_string();
        prefix.push(".");
        // tempfile uses exclusive create semantics for a randomized name. An attacker-created
        // symlink can therefore only cause a collision/retry; it is never followed or truncated.
        let temporary = Builder::new()
            .prefix(&prefix)
            .suffix(".tmp")
            .rand_bytes(12)
            .tempfile_in(parent)?;
        Ok(Self {
            destination: path.to_path_buf(),
            temporary: Some(temporary),
        })
    }

    pub(crate) fn file_mut(&mut self) -> &mut File {
        self.temporary
            .as_mut()
            .expect("uncommitted atomic writer owns its temporary file")
            .as_file_mut()
    }

    pub(crate) fn commit(mut self) -> Result<()> {
        let temporary = self
            .temporary
            .take()
            .expect("uncommitted atomic writer owns its temporary file");
        temporary.as_file().sync_all()?;
        let parent = self
            .destination
            .parent()
            .expect("validated atomic destination has a parent");
        temporary
            .persist(&self.destination)
            .map_err(|error| BackupError::Io(error.error))?;
        sync_directory(parent)
    }
}

pub(crate) fn sync_tree(root: &Path) -> Result<()> {
    reject_symlink_directory(root)?;
    let mut directories = vec![root.to_path_buf()];
    let mut index = 0usize;
    while index < directories.len() {
        let directory = directories[index].clone();
        index += 1;
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = entry.file_type()?;
            if metadata.is_symlink() {
                return Err(BackupError::UnsafePath {
                    path: entry.path().display().to_string(),
                });
            }
            if metadata.is_dir() {
                directories.push(entry.path());
            } else if metadata.is_file() {
                File::open(entry.path())?.sync_all()?;
            } else {
                return Err(BackupError::UnsafePath {
                    path: entry.path().display().to_string(),
                });
            }
        }
    }
    for directory in directories.iter().rev() {
        sync_directory(directory)?;
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(BackupError::UnsafePath {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

pub(crate) fn reject_symlink_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(BackupError::UnsafePath {
            path: path.display().to_string(),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn reject_symlink_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BackupError::UnsafePath {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

pub(crate) fn reject_symlink_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BackupError::UnsafePath {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_portable_relative_path(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_PORTABLE_PATH_BYTES
        || value.contains('\\')
        || value.contains('\0')
        || value.starts_with('/')
        || value.bytes().any(|byte| !(0x20..0x7f).contains(&byte))
    {
        return Err(BackupError::UnsafePath {
            path: value.to_owned(),
        });
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BackupError::UnsafePath {
            path: value.to_owned(),
        });
    }
    for component in value.split('/') {
        let upper = component.to_ascii_uppercase();
        let stem = upper.split('.').next().unwrap_or(upper.as_str());
        if component.ends_with([' ', '.'])
            || matches!(
                stem,
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            )
        {
            return Err(BackupError::UnsafePath {
                path: value.to_owned(),
            });
        }
    }
    Ok(())
}

pub(crate) fn validate_no_case_collisions<'a>(paths: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut folded = BTreeSet::new();
    for path in paths {
        validate_portable_relative_path(path)?;
        if !folded.insert(path.to_ascii_lowercase()) {
            return Err(BackupError::InvalidManifest {
                reason: "entry paths collide on a case-insensitive filesystem",
            });
        }
    }
    Ok(())
}

pub(crate) fn list_tree_files_bounded(
    root: &Path,
    max_files: usize,
    max_path_bytes: usize,
) -> Result<Vec<String>> {
    reject_symlink_directory(root)?;
    let mut pending = vec![PathBuf::new()];
    let mut files = Vec::new();
    let mut path_bytes = 0usize;
    let max_nodes = max_files.saturating_mul(4).saturating_add(16);
    let mut nodes_seen = 0usize;
    while let Some(relative_directory) = pending.pop() {
        let directory = root.join(&relative_directory);
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            nodes_seen = nodes_seen.saturating_add(1);
            if nodes_seen > max_nodes {
                return Err(BackupError::InvalidManifest {
                    reason: "package tree exceeds the bounded node limit",
                });
            }
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(BackupError::UnsafePath {
                    path: entry.path().display().to_string(),
                });
            }
            let relative = relative_directory.join(entry.file_name());
            if file_type.is_dir() {
                pending.push(relative);
            } else if file_type.is_file() {
                let portable = relative
                    .to_str()
                    .ok_or_else(|| BackupError::UnsafePath {
                        path: relative.display().to_string(),
                    })?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                validate_portable_relative_path(&portable)?;
                if files.len() >= max_files {
                    return Err(BackupError::InvalidManifest {
                        reason: "package tree exceeds the bounded file limit",
                    });
                }
                path_bytes =
                    path_bytes
                        .checked_add(portable.len())
                        .ok_or(BackupError::InvalidManifest {
                            reason: "package paths exceed the aggregate UTF-8 limit",
                        })?;
                if path_bytes > max_path_bytes {
                    return Err(BackupError::InvalidManifest {
                        reason: "package paths exceed the aggregate UTF-8 limit",
                    });
                }
                files.push(portable);
            } else {
                return Err(BackupError::UnsafePath {
                    path: entry.path().display().to_string(),
                });
            }
        }
    }
    files.sort();
    validate_no_case_collisions(files.iter().map(String::as_str))?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_paths_cover_case_length_reserved_names_and_traversal() {
        let hash = "a".repeat(64);
        let object = format!("data/assets/objects/aa/aa/{hash}");
        assert!(validate_portable_relative_path(&object).is_ok());
        assert!(validate_portable_relative_path("../escape").is_err());
        assert!(validate_portable_relative_path("data\\escape").is_err());
        assert!(validate_portable_relative_path("data/CON/file").is_err());
        assert!(validate_portable_relative_path(&"a".repeat(241)).is_err());
        assert!(validate_no_case_collisions(["Data/a", "data/A"].into_iter()).is_err());
    }

    #[test]
    fn atomic_write_replaces_content_without_leaving_random_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("progress.json");
        fs::write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn predictable_legacy_tmp_symlink_is_never_followed_or_truncated() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let victim = directory.path().join("victim");
        let destination = directory.path().join("progress.json");
        let predictable = directory.path().join("progress.json.tmp");
        fs::write(&victim, b"protected").unwrap();
        symlink(&victim, &predictable).unwrap();

        atomic_write(&destination, b"journal").unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"journal");
        assert_eq!(fs::read(&victim).unwrap(), b"protected");
        assert!(
            fs::symlink_metadata(&predictable)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
