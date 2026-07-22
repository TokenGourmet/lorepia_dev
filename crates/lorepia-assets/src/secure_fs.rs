use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(not(unix))]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io;

use crate::{AssetError, Result};

#[derive(Clone, Debug)]
pub(crate) struct SecureDirectory {
    path: PathBuf,
    #[cfg(unix)]
    handle: Arc<File>,
    #[cfg(not(unix))]
    identity: Arc<DirectoryIdentity>,
}

#[derive(Debug)]
pub(crate) struct SecureDirEntry {
    name: OsString,
}

impl SecureDirEntry {
    pub(crate) fn file_name(&self) -> &OsStr {
        &self.name
    }
}

#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct SecureReadDir {
    inner: rustix::fs::Dir,
}

#[cfg(not(unix))]
#[derive(Debug)]
pub(crate) struct SecureReadDir {
    inner: fs::ReadDir,
}

impl Iterator for SecureReadDir {
    type Item = Result<SecureDirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        #[cfg(unix)]
        loop {
            use std::os::unix::ffi::OsStrExt;

            let entry = self.inner.next()?;
            match entry {
                Ok(entry) => {
                    let bytes = entry.file_name().to_bytes();
                    if bytes == b"." || bytes == b".." {
                        continue;
                    }
                    return Some(Ok(SecureDirEntry {
                        name: OsStr::from_bytes(bytes).to_os_string(),
                    }));
                }
                Err(error) => return Some(Err(AssetError::Io(error.into()))),
            }
        }

        #[cfg(not(unix))]
        {
            self.inner.next().map(|entry| {
                entry
                    .map(|entry| SecureDirEntry {
                        name: entry.file_name(),
                    })
                    .map_err(AssetError::Io)
            })
        }
    }
}

impl SecureDirectory {
    pub(crate) fn open_root(path: &Path) -> Result<Self> {
        if path.as_os_str().is_empty() {
            return Err(AssetError::InvalidInput {
                field: "asset root",
                reason: "must not be empty",
            });
        }
        match fs::symlink_metadata(path) {
            Ok(metadata) => reject_unsafe_directory(path, &metadata)?,
            Err(error) if error.kind() == ErrorKind::NotFound => fs::create_dir_all(path)?,
            Err(error) => return Err(error.into()),
        }
        Self::open_ambient(path)
    }

    #[cfg(unix)]
    fn open_ambient(path: &Path) -> Result<Self> {
        use rustix::fs::{CWD, Mode, OFlags, openat};

        let handle = openat(
            CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?;
        let file: File = handle.into();
        let canonical = fs::canonicalize(path)?;
        let directory = Self {
            path: canonical,
            handle: Arc::new(file),
        };
        directory.ensure_path_identity()?;
        Ok(directory)
    }

    #[cfg(not(unix))]
    fn open_ambient(path: &Path) -> Result<Self> {
        let canonical = fs::canonicalize(path)?;
        let metadata = fs::symlink_metadata(&canonical)?;
        reject_unsafe_directory(&canonical, &metadata)?;
        let identity = DirectoryIdentity::from_path(&canonical)?;
        Ok(Self {
            path: canonical,
            identity: Arc::new(identity),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn ensure_path_identity(&self) -> Result<()> {
        let metadata = fs::symlink_metadata(&self.path)?;
        reject_unsafe_directory(&self.path, &metadata)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            let opened = self.handle.metadata()?;
            if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
                return Err(unsafe_path(
                    &self.path,
                    "directory path no longer names the opened storage boundary",
                ));
            }
        }

        #[cfg(not(unix))]
        if *self.identity != DirectoryIdentity::from_path(&self.path)? {
            return Err(unsafe_path(
                &self.path,
                "directory identity changed after the storage boundary was opened",
            ));
        }

        Ok(())
    }

    pub(crate) fn open_or_create_child(&self, name: &OsStr) -> Result<Self> {
        match self.open_child(name) {
            Ok(directory) => Ok(directory),
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                self.create_child(name)?;
                self.open_child(name)
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(unix)]
    pub(crate) fn open_child(&self, name: &OsStr) -> Result<Self> {
        use rustix::fs::{Mode, OFlags, openat};

        validate_component(name, &self.path)?;
        let handle = openat(
            &*self.handle,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| map_unix_open_error(error, &self.path.join(name)))?;
        let file: File = handle.into();
        Ok(Self {
            path: self.path.join(name),
            handle: Arc::new(file),
        })
    }

    #[cfg(not(unix))]
    pub(crate) fn open_child(&self, name: &OsStr) -> Result<Self> {
        validate_component(name, &self.path)?;
        self.ensure_path_identity()?;
        Self::open_ambient(&self.path.join(name))
    }

    #[cfg(unix)]
    fn create_child(&self, name: &OsStr) -> Result<()> {
        use rustix::{
            fs::{Mode, mkdirat},
            io::Errno,
        };

        validate_component(name, &self.path)?;
        match mkdirat(&*self.handle, name, Mode::RWXU) {
            Ok(()) | Err(Errno::EXIST) => Ok(()),
            Err(error) => Err(AssetError::Io(error.into())),
        }
    }

    #[cfg(not(unix))]
    fn create_child(&self, name: &OsStr) -> Result<()> {
        validate_component(name, &self.path)?;
        self.ensure_path_identity()?;
        match fs::create_dir(self.path.join(name)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(unix)]
    pub(crate) fn create_new_file(&self, name: &OsStr) -> Result<File> {
        use rustix::fs::{Mode, OFlags, openat};

        validate_component(name, &self.path)?;
        let handle = openat(
            &*self.handle,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(io::Error::from)?;
        Ok(handle.into())
    }

    #[cfg(not(unix))]
    pub(crate) fn create_new_file(&self, name: &OsStr) -> Result<File> {
        validate_component(name, &self.path)?;
        self.ensure_path_identity()?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path.join(name))
            .map_err(Into::into)
    }

    #[cfg(unix)]
    pub(crate) fn open_file(&self, name: &OsStr) -> Result<File> {
        use rustix::fs::{Mode, OFlags, openat};

        validate_component(name, &self.path)?;
        let handle = openat(
            &*self.handle,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| map_unix_open_error(error, &self.path.join(name)))?;
        let file: File = handle.into();
        if !file.metadata()?.is_file() {
            return Err(unsafe_path(
                &self.path.join(name),
                "entry is not a regular no-follow file",
            ));
        }
        Ok(file)
    }

    #[cfg(windows)]
    pub(crate) fn open_file(&self, name: &OsStr) -> Result<File> {
        validate_component(name, &self.path)?;
        self.ensure_path_identity()?;
        let path = self.path.join(name);
        let inspected = open_windows_entry_no_follow(&path, false)?;
        reject_unsafe_file(&path, &inspected.metadata()?)?;
        let opened = open_windows_entry_no_follow(&path, false)?;
        reject_unsafe_file(&path, &opened.metadata()?)?;
        if !same_file_identity(&inspected, &opened)? {
            return Err(unsafe_path(
                &path,
                "entry changed while it was being opened",
            ));
        }
        Ok(opened)
    }

    #[cfg(not(any(unix, windows)))]
    pub(crate) fn open_file(&self, name: &OsStr) -> Result<File> {
        validate_component(name, &self.path)?;
        self.ensure_path_identity()?;
        let path = self.path.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        reject_unsafe_file(&path, &metadata)?;
        let file = File::open(&path)?;
        if !file.metadata()?.is_file() {
            return Err(unsafe_path(
                &path,
                "entry changed while it was being opened",
            ));
        }
        same_file_identity(&file, &file)?;
        Ok(file)
    }

    pub(crate) fn file_exists(&self, name: &OsStr) -> Result<bool> {
        match self.open_file(name) {
            Ok(_) => Ok(true),
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn same_file_as(
        &self,
        name: &OsStr,
        other: &Self,
        other_name: &OsStr,
    ) -> Result<bool> {
        let first = self.open_file(name)?;
        let second = other.open_file(other_name)?;
        same_file_identity(&first, &second)
    }

    #[cfg(unix)]
    pub(crate) fn hard_link_to(
        &self,
        source_name: &OsStr,
        target: &Self,
        target_name: &OsStr,
    ) -> Result<()> {
        use rustix::fs::{AtFlags, linkat};

        validate_component(source_name, &self.path)?;
        validate_component(target_name, &target.path)?;
        let source = self.open_file(source_name)?;
        linkat(
            &*self.handle,
            source_name,
            &*target.handle,
            target_name,
            AtFlags::empty(),
        )
        .map_err(|error| AssetError::Io(error.into()))?;
        let published = target.open_file(target_name)?;
        if !same_file_identity(&source, &published)? {
            return Err(unsafe_path(
                &target.path.join(target_name),
                "source entry changed while a no-follow hard link was being published",
            ));
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub(crate) fn hard_link_to(
        &self,
        source_name: &OsStr,
        target: &Self,
        target_name: &OsStr,
    ) -> Result<()> {
        validate_component(source_name, &self.path)?;
        validate_component(target_name, &target.path)?;
        self.ensure_path_identity()?;
        target.ensure_path_identity()?;
        let source = self.open_file(source_name)?;
        fs::hard_link(self.path.join(source_name), target.path.join(target_name))?;
        let published = target.open_file(target_name)?;
        if !same_file_identity(&source, &published)? {
            return Err(unsafe_path(
                &target.path.join(target_name),
                "source entry changed while a hard link was being published",
            ));
        }
        Ok(())
    }

    pub(crate) fn move_file_no_replace(
        &self,
        source_name: &OsStr,
        target: &Self,
        target_name: &OsStr,
    ) -> Result<()> {
        self.hard_link_to(source_name, target, target_name)?;
        target.sync()?;
        self.remove_file(source_name)?;
        self.sync()?;
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn remove_file(&self, name: &OsStr) -> Result<()> {
        use rustix::fs::{AtFlags, unlinkat};

        validate_component(name, &self.path)?;
        unlinkat(&*self.handle, name, AtFlags::empty())
            .map_err(|error| AssetError::Io(error.into()))
    }

    #[cfg(not(unix))]
    pub(crate) fn remove_file(&self, name: &OsStr) -> Result<()> {
        validate_component(name, &self.path)?;
        self.ensure_path_identity()?;
        let path = self.path.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        reject_unsafe_file(&path, &metadata)?;
        fs::remove_file(path).map_err(Into::into)
    }

    #[cfg(unix)]
    pub(crate) fn sync(&self) -> Result<()> {
        rustix::fs::fsync(&*self.handle).map_err(|error| AssetError::Io(error.into()))
    }

    #[cfg(not(unix))]
    pub(crate) fn sync(&self) -> Result<()> {
        self.ensure_path_identity()
    }

    #[cfg(unix)]
    pub(crate) fn read_dir(&self) -> Result<SecureReadDir> {
        Ok(SecureReadDir {
            inner: rustix::fs::Dir::read_from(&*self.handle)
                .map_err(|error| AssetError::Io(error.into()))?,
        })
    }

    #[cfg(not(unix))]
    pub(crate) fn read_dir(&self) -> Result<SecureReadDir> {
        self.ensure_path_identity()?;
        Ok(SecureReadDir {
            inner: fs::read_dir(&self.path)?,
        })
    }
}

#[cfg(not(unix))]
#[derive(Debug)]
struct DirectoryIdentity {
    canonical: PathBuf,
    #[cfg(windows)]
    identity: WindowsFileIdentity,
    #[cfg(windows)]
    _handle: File,
}

#[cfg(not(unix))]
impl PartialEq for DirectoryIdentity {
    fn eq(&self, other: &Self) -> bool {
        if self.canonical != other.canonical {
            return false;
        }

        #[cfg(windows)]
        {
            self.identity == other.identity
        }

        #[cfg(not(windows))]
        {
            true
        }
    }
}

#[cfg(not(unix))]
impl Eq for DirectoryIdentity {}

#[cfg(not(unix))]
impl DirectoryIdentity {
    fn from_path(path: &Path) -> Result<Self> {
        #[cfg(windows)]
        {
            let handle = open_windows_entry_no_follow(path, true)?;
            reject_unsafe_directory(path, &handle.metadata()?)?;
            let identity = query_windows_file_identity(&handle).map_err(|_| {
                unsafe_path(path, "Windows did not expose a stable directory identity")
            })?;
            Ok(Self {
                canonical: path.to_path_buf(),
                identity,
                _handle: handle,
            })
        }

        #[cfg(not(windows))]
        {
            Err(unsafe_path(
                path,
                "this non-Unix platform has no supported no-follow directory identity contract",
            ))
        }
    }
}

#[cfg(unix)]
fn map_unix_open_error(error: rustix::io::Errno, path: &Path) -> AssetError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        unsafe_path(
            path,
            "entry is a symlink, reparse point, or non-directory boundary",
        )
    } else {
        AssetError::Io(error.into())
    }
}

#[cfg(unix)]
fn same_file_identity(first: &File, second: &File) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let first = first.metadata()?;
    let second = second.metadata()?;
    Ok(first.dev() == second.dev() && first.ino() == second.ino())
}

#[cfg(windows)]
fn same_file_identity(first: &File, second: &File) -> Result<bool> {
    let first = query_windows_file_identity(first).map_err(|_| windows_identity_unavailable())?;
    let second = query_windows_file_identity(second).map_err(|_| windows_identity_unavailable())?;
    Ok(first == second)
}

#[cfg(windows)]
fn windows_identity_unavailable() -> AssetError {
    unsafe_path(
        Path::new("<Windows file identity>"),
        "Windows did not expose a stable file identity",
    )
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_first: &File, _second: &File) -> Result<bool> {
    Err(unsafe_path(
        Path::new("<opaque directory entry>"),
        "platform cannot prove that two recovery entries are the same file",
    ))
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WindowsFileIdentity {
    volume: u64,
    index: u64,
}

#[cfg(windows)]
fn query_windows_file_identity(file: &File) -> std::io::Result<WindowsFileIdentity> {
    let information = winapi_util::file::information(file)?;
    Ok(WindowsFileIdentity {
        volume: information.volume_serial_number(),
        index: information.file_index(),
    })
}

#[cfg(windows)]
fn open_windows_entry_no_follow(path: &Path, directory: bool) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let directory_flag = if directory {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        0
    };
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | directory_flag)
        .open(path)
}

fn reject_unsafe_directory(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() || is_reparse_point(metadata) {
        return Err(unsafe_path(
            path,
            "expected a non-reparse, non-symlink directory",
        ));
    }
    Ok(())
}

fn validate_component(name: &OsStr, parent: &Path) -> Result<()> {
    use std::path::Component;

    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(component)) if component == name)
        || components.next().is_some()
        || is_windows_reserved_component(name)
    {
        return Err(unsafe_path(
            &parent.join(name),
            "directory-relative operation requires one safe path component",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_reserved_component(name: &OsStr) -> bool {
    let value = name.to_string_lossy();
    if value.ends_with(' ') || value.ends_with('.') || value.contains(':') {
        return true;
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || stem.strip_prefix("COM").is_some_and(|number| {
            matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || stem.strip_prefix("LPT").is_some_and(|number| {
            matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
}

#[cfg(not(windows))]
fn is_windows_reserved_component(_name: &OsStr) -> bool {
    false
}

#[cfg(not(unix))]
fn reject_unsafe_file(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_file() || is_reparse_point(metadata) {
        return Err(unsafe_path(
            path,
            "expected a regular non-reparse, non-symlink file",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn unsafe_path(path: &Path, reason: &str) -> AssetError {
    AssetError::UnsafeFilesystem {
        path: path.display().to_string(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_directory_relative_primitive_rejects_parent_components() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root_path = temp.path().join("root");
        let root = SecureDirectory::open_root(&root_path).expect("root");
        let protected = temp.path().join("protected");
        fs::write(&protected, b"protected").expect("fixture");

        assert!(matches!(
            root.open_file(OsStr::new("../protected")),
            Err(AssetError::UnsafeFilesystem { .. })
        ));
        assert!(matches!(
            root.create_new_file(OsStr::new("../created")),
            Err(AssetError::UnsafeFilesystem { .. })
        ));
        assert!(matches!(
            root.remove_file(OsStr::new("../protected")),
            Err(AssetError::UnsafeFilesystem { .. })
        ));
        assert_eq!(
            fs::read(protected).expect("protected remains"),
            b"protected"
        );
        assert!(!temp.path().join("created").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_open_handle_identity_matches_hard_links_only() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let first_path = temp.path().join("first.bin");
        let linked_path = temp.path().join("linked.bin");
        let distinct_path = temp.path().join("distinct.bin");
        fs::write(&first_path, b"first").expect("first fixture");
        fs::hard_link(&first_path, &linked_path).expect("hard-link fixture");
        fs::write(&distinct_path, b"distinct").expect("distinct fixture");

        let first = open_windows_entry_no_follow(&first_path, false).expect("first handle");
        let linked = open_windows_entry_no_follow(&linked_path, false).expect("linked handle");
        let distinct =
            open_windows_entry_no_follow(&distinct_path, false).expect("distinct handle");

        assert!(same_file_identity(&first, &linked).expect("hard-link identity"));
        assert!(!same_file_identity(&first, &distinct).expect("distinct identity"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_directory_identity_rejects_path_replacement() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root_path = temp.path().join("root");
        let displaced_path = temp.path().join("displaced");
        let root = SecureDirectory::open_root(&root_path).expect("root");

        fs::rename(&root_path, &displaced_path).expect("displace opened root");
        fs::create_dir(&root_path).expect("replacement root");

        assert!(matches!(
            root.ensure_path_identity(),
            Err(AssetError::UnsafeFilesystem { .. })
        ));
    }
}
