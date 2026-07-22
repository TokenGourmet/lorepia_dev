use std::{
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::Path,
};

use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use sha2::{Digest, Sha256};

use crate::{
    BackupEntry, BackupError, BackupManifest, Result,
    fsutil::{AtomicWriteFile, reject_symlink_file},
    model::{
        MAX_BACKUP_MANIFEST_BYTES, MAX_BACKUP_MANIFEST_ENTRIES, MAX_BACKUP_MANIFEST_PATH_BYTES,
    },
};

const BYTE_LIMIT_REASON: &str = "manifest exceeds the 32 MiB byte limit";
const ENTRY_LIMIT_REASON: &str = "manifest exceeds the 100004-entry limit";
const PATH_LIMIT_REASON: &str = "manifest paths exceed the 16 MiB aggregate UTF-8 limit";

#[derive(Default)]
struct PreflightState {
    entries_seen: bool,
    entry_count: usize,
    path_bytes: usize,
    violation: Option<&'static str>,
}

pub(crate) fn read_bounded_manifest(path: &Path) -> Result<Vec<u8>> {
    reject_symlink_file(path)?;
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_BACKUP_MANIFEST_BYTES {
        return Err(BackupError::InvalidManifest {
            reason: BYTE_LIMIT_REASON,
        });
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| BackupError::InvalidManifest {
        reason: BYTE_LIMIT_REASON,
    })?;
    let mut file = File::open(path)?;
    let mut bytes = Vec::with_capacity(capacity);
    Read::by_ref(&mut file)
        .take(MAX_BACKUP_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BACKUP_MANIFEST_BYTES {
        return Err(BackupError::InvalidManifest {
            reason: BYTE_LIMIT_REASON,
        });
    }
    Ok(bytes)
}

/// Performs a zero-copy pass over manifest entry paths before `BackupManifest` can allocate its
/// `Vec<BackupEntry>` and owned strings.
pub(crate) fn preflight_manifest(bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 > MAX_BACKUP_MANIFEST_BYTES {
        return Err(BackupError::InvalidManifest {
            reason: BYTE_LIMIT_REASON,
        });
    }
    let mut state = PreflightState::default();
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let parsed = ManifestSeed { state: &mut state }.deserialize(&mut deserializer);
    if let Some(reason) = state.violation {
        return Err(BackupError::InvalidManifest { reason });
    }
    parsed?;
    deserializer.end()?;
    if !state.entries_seen {
        return Err(BackupError::InvalidManifest {
            reason: "manifest entries field is missing",
        });
    }
    Ok(())
}

pub(crate) fn validate_manifest_budget(entries: &[BackupEntry]) -> Result<()> {
    if entries.len() > MAX_BACKUP_MANIFEST_ENTRIES {
        return Err(BackupError::InvalidManifest {
            reason: ENTRY_LIMIT_REASON,
        });
    }
    let path_bytes = entries.iter().try_fold(0usize, |total, entry| {
        total
            .checked_add(entry.path.len())
            .ok_or(BackupError::InvalidManifest {
                reason: PATH_LIMIT_REASON,
            })
    })?;
    if path_bytes > MAX_BACKUP_MANIFEST_PATH_BYTES {
        return Err(BackupError::InvalidManifest {
            reason: PATH_LIMIT_REASON,
        });
    }
    Ok(())
}

pub(crate) fn push_manifest_entry(
    entries: &mut Vec<BackupEntry>,
    path_bytes: &mut usize,
    entry: BackupEntry,
) -> Result<()> {
    if entries.len() >= MAX_BACKUP_MANIFEST_ENTRIES {
        return Err(BackupError::InvalidManifest {
            reason: ENTRY_LIMIT_REASON,
        });
    }
    let next_path_bytes =
        path_bytes
            .checked_add(entry.path.len())
            .ok_or(BackupError::InvalidManifest {
                reason: PATH_LIMIT_REASON,
            })?;
    if next_path_bytes > MAX_BACKUP_MANIFEST_PATH_BYTES {
        return Err(BackupError::InvalidManifest {
            reason: PATH_LIMIT_REASON,
        });
    }
    entries.push(entry);
    *path_bytes = next_path_bytes;
    Ok(())
}

pub(crate) fn canonical_manifest_matches(
    expected: &[u8],
    manifest: &BackupManifest,
) -> Result<bool> {
    let mut writer = ComparingWriter {
        expected,
        position: 0,
        equal: true,
    };
    serde_json::to_writer(&mut writer, manifest)?;
    writer.write_all(b"\n")?;
    Ok(writer.equal && writer.position == expected.len())
}

pub(crate) fn write_canonical_manifest(path: &Path, manifest: &BackupManifest) -> Result<String> {
    validate_manifest_budget(&manifest.entries)?;
    let mut temporary = AtomicWriteFile::create(path)?;
    let digest = {
        let mut writer = HashingLimitWriter {
            file: temporary.file_mut(),
            digest: Sha256::new(),
            written: 0,
            exceeded: false,
        };
        let serialized = serde_json::to_writer(&mut writer, manifest);
        let newline_error = if serialized.is_ok() {
            writer.write_all(b"\n").err()
        } else {
            None
        };
        if writer.exceeded {
            return Err(BackupError::InvalidManifest {
                reason: BYTE_LIMIT_REASON,
            });
        }
        if let Err(error) = serialized {
            return Err(error.into());
        }
        if let Some(error) = newline_error {
            return Err(error.into());
        }
        hex::encode(writer.digest.finalize())
    };
    temporary.commit()?;
    Ok(digest)
}

struct ManifestSeed<'a> {
    state: &'a mut PreflightState,
}

impl<'de> DeserializeSeed<'de> for ManifestSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ManifestVisitor { state: self.state })
    }
}

struct ManifestVisitor<'a> {
    state: &'a mut PreflightState,
}

impl<'de> Visitor<'de> for ManifestVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a backup manifest object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while let Some(key) = map.next_key::<&'de str>()? {
            if key == "entries" {
                if self.state.entries_seen {
                    return Err(serde::de::Error::duplicate_field("entries"));
                }
                self.state.entries_seen = true;
                map.next_value_seed(EntriesSeed { state: self.state })?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

struct EntriesSeed<'a> {
    state: &'a mut PreflightState,
}

impl<'de> DeserializeSeed<'de> for EntriesSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(EntriesVisitor { state: self.state })
    }
}

struct EntriesVisitor<'a> {
    state: &'a mut PreflightState,
}

impl<'de> Visitor<'de> for EntriesVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the manifest entry array")
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence
            .next_element_seed(EntrySeed { state: self.state })?
            .is_some()
        {}
        Ok(())
    }
}

struct EntrySeed<'a> {
    state: &'a mut PreflightState,
}

impl<'de> DeserializeSeed<'de> for EntrySeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(EntryVisitor { state: self.state })
    }
}

struct EntryVisitor<'a> {
    state: &'a mut PreflightState,
}

impl<'de> Visitor<'de> for EntryVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a manifest entry object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        self.state.entry_count = self.state.entry_count.saturating_add(1);
        if self.state.entry_count > MAX_BACKUP_MANIFEST_ENTRIES {
            self.state.violation = Some(ENTRY_LIMIT_REASON);
            return Err(serde::de::Error::custom(ENTRY_LIMIT_REASON));
        }
        let mut path_seen = false;
        while let Some(key) = map.next_key::<&'de str>()? {
            if key == "path" {
                if path_seen {
                    return Err(serde::de::Error::duplicate_field("path"));
                }
                path_seen = true;
                let path = map.next_value::<&'de str>()?;
                self.state.path_bytes =
                    self.state
                        .path_bytes
                        .checked_add(path.len())
                        .ok_or_else(|| {
                            self.state.violation = Some(PATH_LIMIT_REASON);
                            serde::de::Error::custom(PATH_LIMIT_REASON)
                        })?;
                if self.state.path_bytes > MAX_BACKUP_MANIFEST_PATH_BYTES {
                    self.state.violation = Some(PATH_LIMIT_REASON);
                    return Err(serde::de::Error::custom(PATH_LIMIT_REASON));
                }
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        if !path_seen {
            return Err(serde::de::Error::missing_field("path"));
        }
        Ok(())
    }
}

struct ComparingWriter<'a> {
    expected: &'a [u8],
    position: usize,
    equal: bool,
}

impl Write for ComparingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self.position.saturating_add(bytes.len());
        if end > self.expected.len() || self.expected[self.position..end] != *bytes {
            self.equal = false;
        }
        self.position = end;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct HashingLimitWriter<'a> {
    file: &'a mut File,
    digest: Sha256,
    written: u64,
    exceeded: bool,
}

impl Write for HashingLimitWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self.written.saturating_add(bytes.len() as u64);
        if next > MAX_BACKUP_MANIFEST_BYTES {
            self.exceeded = true;
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                BYTE_LIMIT_REASON,
            ));
        }
        self.file.write_all(bytes)?;
        self.digest.update(bytes);
        self.written = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CompatibilityReceipt, ManifestDatabase, SnapshotContract,
        fsutil::{canonical_json, sha256_bytes},
    };

    use super::*;

    #[test]
    fn streamed_manifest_preserves_v1_canonical_bytes_and_hash() {
        let manifest = BackupManifest {
            format: "lorepia-directory-backup".to_owned(),
            format_version: 1,
            session_id: "a".repeat(32),
            product_database: ManifestDatabase {
                path: "data/product.sqlite3".to_owned(),
                schema_version: 1,
                sha256: "b".repeat(64),
                size: 10,
            },
            asset_catalog: ManifestDatabase {
                path: "data/assets/assets.sqlite3".to_owned(),
                schema_version: 1,
                sha256: "c".repeat(64),
                size: 20,
            },
            entries: Vec::new(),
            total_entry_bytes: 0,
            snapshot_contract: SnapshotContract::default(),
            compatibility_receipts: vec![CompatibilityReceipt {
                check_id: "BACKUP-007".to_owned(),
                disposition: "not_applicable_by_design".to_owned(),
                reason: "directory format".to_owned(),
            }],
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("manifest.json");
        #[cfg(unix)]
        let victim = {
            use std::os::unix::fs::symlink;

            let victim = directory.path().join("manifest-victim");
            fs::write(&victim, b"protected").unwrap();
            symlink(&victim, directory.path().join("manifest.json.tmp")).unwrap();
            victim
        };
        let hash = write_canonical_manifest(&path, &manifest).unwrap();
        let expected = canonical_json(&manifest).unwrap();
        assert_eq!(fs::read(path).unwrap(), expected);
        assert_eq!(hash, sha256_bytes(&expected));
        #[cfg(unix)]
        assert_eq!(fs::read(victim).unwrap(), b"protected");
    }

    #[test]
    fn preflight_rejects_declared_entry_count_before_owned_manifest_decode() {
        let mut bytes = Vec::with_capacity(MAX_BACKUP_MANIFEST_ENTRIES * 13 + 32);
        bytes.extend_from_slice(br#"{"entries":["#);
        for index in 0..=MAX_BACKUP_MANIFEST_ENTRIES {
            if index != 0 {
                bytes.push(b',');
            }
            bytes.extend_from_slice(br#"{"path":"x"}"#);
        }
        bytes.extend_from_slice(b"]}");
        assert!(bytes.len() as u64 <= MAX_BACKUP_MANIFEST_BYTES);
        assert!(matches!(
            preflight_manifest(&bytes),
            Err(BackupError::InvalidManifest {
                reason: ENTRY_LIMIT_REASON
            })
        ));
    }

    #[test]
    fn preflight_rejects_aggregate_path_bytes_before_owned_manifest_decode() {
        let path = "x".repeat(200);
        let entry_count = MAX_BACKUP_MANIFEST_PATH_BYTES / path.len() + 1;
        assert!(entry_count <= MAX_BACKUP_MANIFEST_ENTRIES);
        let mut bytes = Vec::with_capacity(entry_count * (path.len() + 13) + 32);
        bytes.extend_from_slice(br#"{"entries":["#);
        for index in 0..entry_count {
            if index != 0 {
                bytes.push(b',');
            }
            bytes.extend_from_slice(br#"{"path":""#);
            bytes.extend_from_slice(path.as_bytes());
            bytes.extend_from_slice(br#""}"#);
        }
        bytes.extend_from_slice(b"]}");
        assert!(bytes.len() as u64 <= MAX_BACKUP_MANIFEST_BYTES);
        assert!(matches!(
            preflight_manifest(&bytes),
            Err(BackupError::InvalidManifest {
                reason: PATH_LIMIT_REASON
            })
        ));
    }
}
