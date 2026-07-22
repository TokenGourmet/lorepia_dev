use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard, TryLockError},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, backup, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    AssetError, AssetHash, AssetLimits, AssetObject, AssetObjectPage, AssetOwner, AssetReference,
    AssetState, AssetStats, BACKUP_SNAPSHOT_LEASE_TIMEOUT_MS, BackupSnapshotCleanup,
    BackupSnapshotLease, CleanupPage, ExportedObject, IngestOutcome, IngestRequest,
    MAX_BACKUP_SNAPSHOT_SESSIONS, MAX_PAGE_SIZE, MAX_SOURCE_NAME_BYTES,
    MAX_TEMPORARY_OWNER_SESSIONS, MarkSweepPage, ReconcileFinding, ReconcileFindingKind,
    ReconcilePage, Result, ShardReconcilePage, TEMPORARY_OWNER_LEASE_TIMEOUT_MS, catalog,
    mime::{self, PROBE_BYTES, TAIL_BYTES},
    model::validate_owner,
    secure_fs::{SecureDirEntry, SecureDirectory, SecureReadDir},
};

const OBJECTS_DIRECTORY: &str = "objects";
const STAGING_DIRECTORY: &str = ".staging";
const QUARANTINE_DIRECTORY: &str = "quarantine";
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const BACKUP_OWNER_TYPE: &str = "lorepia-backup";
const IMPORT_TEMPORARY_OWNER_TYPE: &str = "lorepia-import-session";

#[derive(Clone, Debug)]
pub struct AssetStore {
    inner: Arc<StoreInner>,
}

#[derive(Debug)]
struct StoreInner {
    root_directory: SecureDirectory,
    objects_directory: SecureDirectory,
    staging_directory: SecureDirectory,
    quarantine_directory: SecureDirectory,
    catalog: PathBuf,
    limits: AssetLimits,
    mutation_lock: Mutex<()>,
}

#[derive(Debug)]
pub struct AssetReader {
    file: File,
    object: AssetObject,
}

pub struct ShardReconciler {
    store: AssetStore,
    shard: String,
    directory: Option<SecureDirectory>,
    entries: Option<SecureReadDir>,
    pending: VecDeque<SecureDirEntry>,
}

impl AssetReader {
    pub fn object(&self) -> &AssetObject {
        &self.object
    }
}

impl Read for AssetReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buffer)
    }
}

impl ShardReconciler {
    pub fn next_page<C>(&mut self, limit: u16, mut is_cancelled: C) -> Result<ShardReconcilePage>
    where
        C: FnMut() -> bool,
    {
        validate_page_size(limit)?;
        let store = self.store.clone();
        let _guard = store.mutation_guard()?;
        while self.pending.len() < usize::from(limit) + 1 {
            let Some(entries) = self.entries.as_mut() else {
                break;
            };
            match entries.next() {
                Some(Ok(entry)) => self.pending.push_back(entry),
                Some(Err(error)) => return Err(error),
                None => {
                    self.entries = None;
                    break;
                }
            }
        }
        let connection = store.connection()?;
        let now = now_ms()?;
        let mut findings = Vec::new();
        let mut examined = 0u16;
        let mut last_name = None;
        while examined < limit && !self.pending.is_empty() {
            if is_cancelled() {
                return Err(AssetError::Cancelled);
            }
            let entry = self.pending.front().expect("pending entry exists");
            let directory = self.directory.as_ref().expect("entries have a directory");
            if let Some(finding) =
                store.reconcile_shard_entry(&connection, &self.shard, directory, entry, now)?
            {
                findings.push(finding);
            }
            last_name = Some(entry.file_name().to_string_lossy().into_owned());
            self.pending.pop_front();
            examined += 1;
        }
        let has_more = !self.pending.is_empty() || self.entries.is_some();
        let next_cursor = has_more
            .then(|| last_name.expect("a page with more entries after processing is non-empty"));
        Ok(ShardReconcilePage {
            examined,
            findings,
            next_cursor,
            has_more,
        })
    }
}

impl AssetStore {
    pub fn open(root: impl AsRef<Path>, limits: AssetLimits) -> Result<Self> {
        let root_directory = SecureDirectory::open_root(root.as_ref())?;
        let objects_directory =
            root_directory.open_or_create_child(OsStr::new(OBJECTS_DIRECTORY))?;
        let staging_directory =
            root_directory.open_or_create_child(OsStr::new(STAGING_DIRECTORY))?;
        let quarantine_directory =
            root_directory.open_or_create_child(OsStr::new(QUARANTINE_DIRECTORY))?;
        root_directory.sync()?;
        let root = root_directory.path().to_path_buf();
        let catalog_path = root.join(catalog::CATALOG_FILE_NAME);
        root_directory.ensure_path_identity()?;
        reject_symlink_if_present(&catalog_path)?;
        catalog::initialize(&catalog_path)?;
        root_directory.ensure_path_identity()?;
        let store = Self {
            inner: Arc::new(StoreInner {
                root_directory,
                objects_directory,
                staging_directory,
                quarantine_directory,
                catalog: catalog_path,
                limits,
                mutation_lock: Mutex::new(()),
            }),
        };
        store.recover_quarantine_intents()?;
        store.cleanup_expired_backup_snapshots()?;
        store.recover_expired_temporary_owner_sessions()?;
        Ok(store)
    }

    pub fn limits(&self) -> &AssetLimits {
        &self.inner.limits
    }

    /// Pins the exact active object set and creates a consistent catalog snapshot.
    ///
    /// The short mutation lock covers pin creation and the catalog snapshot only. New assets
    /// added after this method returns are excluded. Reference deletion and mark/sweep may run
    /// during object copying, but persisted pins keep every snapshotted object alive while the
    /// renewable session lease is current. Call [`renew_backup_snapshot`](Self::renew_backup_snapshot)
    /// at bounded progress points. Cancellation remains resumable for 24 hours; explicit abandon,
    /// snapshot failure, or the next bounded stale-session maintenance pass releases the pins.
    /// The session id must be 32 lowercase hexadecimal characters so it is safe and portable in
    /// journals.
    pub fn begin_backup_snapshot<C>(
        &self,
        session_id: &str,
        target: impl AsRef<Path>,
        mut continue_copy: C,
    ) -> Result<u64>
    where
        C: FnMut(u64, u64) -> bool,
    {
        validate_backup_session_id(session_id)?;
        let target = target.as_ref();
        let parent = target.parent().ok_or(AssetError::InvalidInput {
            field: "catalog snapshot path",
            reason: "must have a parent directory",
        })?;
        fs::create_dir_all(parent)?;
        reject_symlink_if_present(target)?;
        if target.exists() {
            return Err(AssetError::UnsafeFilesystem {
                path: target.display().to_string(),
                reason: "catalog snapshot target already exists".to_owned(),
            });
        }

        let _guard = self.mutation_guard()?;
        let mut source = self.connection()?;
        let now = now_ms()?;
        let transaction = source.transaction_with_behavior(TransactionBehavior::Immediate)?;
        cleanup_stale_backup_sessions_in_transaction(
            &transaction,
            now.saturating_sub(BACKUP_SNAPSHOT_LEASE_TIMEOUT_MS),
            MAX_BACKUP_SNAPSHOT_SESSIONS,
        )?;
        let session_exists: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM asset_backup_sessions WHERE session_id = ?1
             )",
            params![session_id],
            |row| row.get(0),
        )?;
        if !session_exists {
            let live_sessions: i64 =
                transaction.query_row("SELECT count(*) FROM asset_backup_sessions", [], |row| {
                    row.get(0)
                })?;
            if live_sessions >= i64::from(MAX_BACKUP_SNAPSHOT_SESSIONS) {
                return Err(AssetError::InvalidInput {
                    field: "backup snapshot session",
                    reason: "live session limit of 128 has been reached",
                });
            }
            transaction.execute(
                "INSERT INTO asset_backup_sessions(
                    session_id, created_at_ms, lease_updated_at_ms
                 ) VALUES (?1, ?2, ?2)",
                params![session_id, now],
            )?;
        } else {
            transaction.execute(
                "UPDATE asset_backup_sessions
                 SET lease_updated_at_ms = max(lease_updated_at_ms, ?2)
                 WHERE session_id = ?1",
                params![session_id, now],
            )?;
            transaction.execute(
                "DELETE FROM asset_refs WHERE owner_type = ?1 AND owner_id = ?2",
                params![BACKUP_OWNER_TYPE, session_id],
            )?;
        }
        transaction.execute(
            "INSERT OR IGNORE INTO asset_refs(owner_type, owner_id, hash, created_at_ms)
             SELECT ?1, ?2, hash, ?3 FROM asset_objects WHERE state = 'active'",
            params![BACKUP_OWNER_TYPE, session_id, now],
        )?;
        transaction.commit()?;

        let result = (|| {
            let mut destination = Connection::open(target)?;
            destination.pragma_update(None, "journal_mode", "DELETE")?;
            destination.pragma_update(None, "synchronous", "FULL")?;
            let snapshot = backup::Backup::new(&source, &mut destination)?;
            loop {
                let progress = snapshot.progress();
                let total = u64::try_from(progress.pagecount.max(0)).unwrap_or(0);
                let remaining = u64::try_from(progress.remaining.max(0)).unwrap_or(0);
                if !continue_copy(total.saturating_sub(remaining), total) {
                    return Err(AssetError::SnapshotCancelled);
                }
                match snapshot.step(128)? {
                    backup::StepResult::Done => break,
                    backup::StepResult::More => {}
                    backup::StepResult::Busy | backup::StepResult::Locked => {
                        std::thread::yield_now();
                    }
                    _ => std::thread::yield_now(),
                }
            }
            drop(snapshot);
            // Backup leases are operational metadata. No export pin from any concurrent session
            // may be restored as a permanent reference in another installation.
            destination.execute(
                "DELETE FROM asset_refs WHERE owner_type = ?1",
                params![BACKUP_OWNER_TYPE],
            )?;
            destination.execute("DELETE FROM asset_backup_sessions", [])?;
            destination.execute(
                "DELETE FROM asset_refs
                 WHERE owner_type = ?1 OR EXISTS(
                    SELECT 1 FROM asset_temporary_owner_sessions s
                    WHERE s.owner_type = asset_refs.owner_type
                      AND s.owner_id = asset_refs.owner_id
                 )",
                params![IMPORT_TEMPORARY_OWNER_TYPE],
            )?;
            destination.execute("DELETE FROM asset_temporary_owner_sessions", [])?;
            destination.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
            destination.pragma_update(None, "journal_mode", "DELETE")?;
            drop(destination);
            let file = File::open(target)?;
            file.sync_all()?;
            Ok(file.metadata()?.len())
        })();

        if result.is_err() {
            match fs::remove_file(target) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => {}
            }
            abandon_backup_snapshot_on_connection(&mut source, session_id)?;
        }
        result
    }

    /// Renews a backup lease and returns its current pin count.
    ///
    /// `None` means the lease was explicitly abandoned or removed as stale. A caller must not
    /// resume from its catalog snapshot in that case.
    pub fn renew_backup_snapshot(&self, session_id: &str) -> Result<Option<BackupSnapshotLease>> {
        validate_backup_session_id(session_id)?;
        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        let now = now_ms()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "UPDATE asset_backup_sessions
             SET lease_updated_at_ms = max(lease_updated_at_ms, ?2)
             WHERE session_id = ?1",
            params![session_id, now],
        )?;
        if updated == 0 {
            transaction.commit()?;
            return Ok(None);
        }
        let (pins, lease_updated_at_ms): (i64, i64) = transaction.query_row(
            "SELECT
                (SELECT count(*) FROM asset_refs WHERE owner_type = ?1 AND owner_id = ?2),
                lease_updated_at_ms
             FROM asset_backup_sessions WHERE session_id = ?2",
            params![BACKUP_OWNER_TYPE, session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        transaction.commit()?;
        Ok(Some(BackupSnapshotLease {
            session_id: session_id.to_owned(),
            pinned_objects: u64::try_from(pins).map_err(|_| AssetError::IncompatibleCatalog {
                reason: "backup snapshot pin count is negative",
            })?,
            lease_updated_at_ms,
        }))
    }

    /// Releases persisted object pins and the lease for an explicitly abandoned backup.
    pub fn abandon_backup_snapshot(&self, session_id: &str) -> Result<u64> {
        validate_backup_session_id(session_id)?;
        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        abandon_backup_snapshot_on_connection(&mut connection, session_id)
    }

    /// Releases persisted object pins after a backup has been fully copied and verified.
    pub fn release_backup_snapshot(&self, session_id: &str) -> Result<u64> {
        self.abandon_backup_snapshot(session_id)
    }

    /// Deletes at most `limit` expired backup leases and all pins owned by those sessions.
    ///
    /// The store admits at most 128 live sessions, so a limit of
    /// [`MAX_BACKUP_SNAPSHOT_SESSIONS`] is one complete maintenance pass. This method is also
    /// called on store open and before creating a snapshot. Active exporters renew at each
    /// bounded object page and before long verification phases.
    pub fn cleanup_stale_backup_snapshots(
        &self,
        stale_before_ms: i64,
        limit: u16,
    ) -> Result<BackupSnapshotCleanup> {
        if stale_before_ms < 0 {
            return Err(AssetError::InvalidInput {
                field: "backup snapshot stale cutoff",
                reason: "must be non-negative milliseconds since epoch",
            });
        }
        if limit == 0 || limit > MAX_BACKUP_SNAPSHOT_SESSIONS {
            return Err(AssetError::InvalidInput {
                field: "backup snapshot cleanup limit",
                reason: "must be between 1 and 128",
            });
        }
        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cleanup =
            cleanup_stale_backup_sessions_in_transaction(&transaction, stale_before_ms, limit)?;
        transaction.commit()?;
        Ok(cleanup)
    }

    /// Runs one complete time-based maintenance pass for the bounded live-session set.
    pub fn cleanup_expired_backup_snapshots(&self) -> Result<BackupSnapshotCleanup> {
        self.cleanup_stale_backup_snapshots(
            now_ms()?.saturating_sub(BACKUP_SNAPSHOT_LEASE_TIMEOUT_MS),
            MAX_BACKUP_SNAPSHOT_SESSIONS,
        )
    }

    /// Returns the logical catalog size used by backup free-space preflight.
    pub fn catalog_snapshot_size_estimate(&self) -> Result<u64> {
        let connection = self.connection()?;
        let page_count: i64 =
            connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
        let page_count =
            u64::try_from(page_count).map_err(|_| AssetError::IncompatibleCatalog {
                reason: "SQLite page count is negative",
            })?;
        let page_size = u64::try_from(page_size).map_err(|_| AssetError::IncompatibleCatalog {
            reason: "SQLite page size is negative",
        })?;
        page_count
            .checked_mul(page_size)
            .ok_or(AssetError::IncompatibleCatalog {
                reason: "SQLite logical size overflowed",
            })
    }

    /// Validates a catalog snapshot without changing its journal mode or catalog rows.
    pub fn validate_catalog_snapshot_file(path: impl AsRef<Path>) -> Result<i64> {
        catalog::validate_snapshot_file(path.as_ref())
    }

    pub fn ingest<R, C>(
        &self,
        reader: &mut R,
        request: IngestRequest,
        mut is_cancelled: C,
    ) -> Result<IngestOutcome>
    where
        R: Read,
        C: FnMut() -> bool,
    {
        validate_request(&request)?;
        let _guard = self.mutation_guard()?;
        let mut staged = self.allocate_staging()?;
        let staging_name = staged.name.clone();

        let result = (|| {
            let copied = copy_and_hash(
                reader,
                staged.file.as_mut().expect("staging file is present"),
                self.inner.limits.max_object_bytes,
                &mut is_cancelled,
            )?;
            staged
                .file
                .as_ref()
                .expect("staging file is present")
                .sync_all()?;
            drop(staged.file.take());

            let mime = mime::validate(
                request.declared_mime,
                &copied.prefix,
                &copied.tail,
                copied.size,
                &self.inner.limits,
            )?;
            let hash = AssetHash::from_digest(&copied.digest);
            let now = now_ms()?;
            let relative_path = relative_object_path(&hash);
            let destination = self.ensure_object_parent(&hash)?;
            let initially_deduplicated = self.prepare_destination(
                &destination,
                PublishExpectation {
                    hash: &hash,
                    size: copied.size,
                    now,
                },
                &mut is_cancelled,
            )?;
            ingest_after_prepare_hook();
            let mut connection = self.connection()?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

            let active_bytes_without_hash: i64 = transaction.query_row(
                "SELECT active_bytes - coalesce((
                    SELECT CASE WHEN state = 'active' THEN size ELSE 0 END
                    FROM asset_objects WHERE hash = ?1
                 ), 0)
                 FROM asset_totals WHERE id = 1",
                params![hash.as_str()],
                |row| row.get(0),
            )?;
            let projected = u64::try_from(active_bytes_without_hash)
                .unwrap_or(u64::MAX)
                .saturating_add(copied.size);
            if projected > self.inner.limits.max_total_bytes {
                return Err(AssetError::LimitExceeded {
                    limit_name: "total asset quota",
                    limit: self.inner.limits.max_total_bytes,
                });
            }

            let deduplicated = if initially_deduplicated
                && self.destination_matches_expectation(
                    &destination,
                    PublishExpectation {
                        hash: &hash,
                        size: copied.size,
                        now,
                    },
                    &mut is_cancelled,
                )? {
                true
            } else {
                self.publish_or_deduplicate(
                    &staging_name,
                    &destination,
                    PublishExpectation {
                        hash: &hash,
                        size: copied.size,
                        now,
                    },
                    &mut is_cancelled,
                )?
            };

            transaction.execute(
                "INSERT INTO asset_objects(
                    hash, size, mime, relative_path, state, quarantine_name,
                    verified_at_ms, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, 'active', NULL, ?5, ?5)
                 ON CONFLICT(hash) DO UPDATE SET
                    size = excluded.size,
                    mime = excluded.mime,
                    relative_path = excluded.relative_path,
                    state = 'active',
                    quarantine_name = NULL,
                    verified_at_ms = excluded.verified_at_ms",
                params![
                    hash.as_str(),
                    encode_size(copied.size)?,
                    mime.as_str(),
                    relative_path,
                    now
                ],
            )?;

            if let Some(owner) = request.owner.as_ref() {
                transaction.execute(
                    "INSERT OR IGNORE INTO asset_refs(
                        owner_type, owner_id, hash, created_at_ms
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![owner.owner_type, owner.owner_id, hash.as_str(), now],
                )?;
            }
            destination.directory.sync()?;
            transaction.commit()?;

            Ok(IngestOutcome {
                object: self
                    .get_object(&hash)?
                    .ok_or_else(|| AssetError::NotFound {
                        hash: hash.to_string(),
                    })?,
                deduplicated,
            })
        })();

        let cleanup = self.cleanup_staged_file(&mut staged);
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    pub fn ingest_uncancelled<R: Read>(
        &self,
        reader: &mut R,
        request: IngestRequest,
    ) -> Result<IngestOutcome> {
        self.ingest(reader, request, || false)
    }

    pub fn get_object(&self, hash: &AssetHash) -> Result<Option<AssetObject>> {
        let connection = self.connection()?;
        catalog::get_object(&connection, hash)
    }

    pub fn list_objects(&self, after: Option<&AssetHash>, limit: u16) -> Result<AssetObjectPage> {
        validate_page_size(limit)?;
        let connection = self.connection()?;
        let fetch = i64::from(limit) + 1;
        let mut objects = Vec::new();
        if let Some(after) = after {
            let mut statement = connection.prepare(
                "SELECT hash, size, mime, relative_path, state, verified_at_ms, created_at_ms
                 FROM asset_objects WHERE hash > ?1 ORDER BY hash LIMIT ?2",
            )?;
            let rows =
                statement.query_map(params![after.as_str(), fetch], catalog::decode_object)?;
            for row in rows {
                objects.push(row?);
            }
        } else {
            let mut statement = connection.prepare(
                "SELECT hash, size, mime, relative_path, state, verified_at_ms, created_at_ms
                 FROM asset_objects ORDER BY hash LIMIT ?1",
            )?;
            let rows = statement.query_map(params![fetch], catalog::decode_object)?;
            for row in rows {
                objects.push(row?);
            }
        }
        let has_more = objects.len() > usize::from(limit);
        objects.truncate(usize::from(limit));
        let next_cursor = has_more.then(|| objects.last().expect("non-empty page").hash.clone());
        Ok(AssetObjectPage {
            objects,
            next_cursor,
        })
    }

    pub fn add_refs(&self, refs: &[AssetReference]) -> Result<()> {
        if refs.is_empty() {
            return Ok(());
        }
        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = now_ms()?;
        for reference in refs {
            validate_owner(&reference.owner)?;
            let state: Option<String> = transaction
                .query_row(
                    "SELECT state FROM asset_objects WHERE hash = ?1",
                    params![reference.hash.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            match state.as_deref() {
                None => {
                    return Err(AssetError::NotFound {
                        hash: reference.hash.to_string(),
                    });
                }
                Some("active") => {}
                Some(state) => {
                    return Err(AssetError::NotActive {
                        hash: reference.hash.to_string(),
                        state: state.to_owned(),
                    });
                }
            }
            transaction.execute(
                "INSERT OR IGNORE INTO asset_refs(owner_type, owner_id, hash, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    reference.owner.owner_type,
                    reference.owner.owner_id,
                    reference.hash.as_str(),
                    now
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn remove_refs(&self, refs: &[AssetReference]) -> Result<u64> {
        if refs.is_empty() {
            return Ok(0);
        }
        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut removed = 0u64;
        for reference in refs {
            validate_owner(&reference.owner)?;
            removed = removed.saturating_add(transaction.execute(
                "DELETE FROM asset_refs
                 WHERE owner_type = ?1 AND owner_id = ?2 AND hash = ?3",
                params![
                    reference.owner.owner_type,
                    reference.owner.owner_id,
                    reference.hash.as_str()
                ],
            )? as u64);
        }
        transaction.commit()?;
        Ok(removed)
    }

    pub fn remove_owner_refs(&self, owner: &AssetOwner) -> Result<u64> {
        validate_owner(owner)?;
        let _guard = self.mutation_guard()?;
        let connection = self.connection()?;
        let removed = connection.execute(
            "DELETE FROM asset_refs WHERE owner_type = ?1 AND owner_id = ?2",
            params![owner.owner_type, owner.owner_id],
        )?;
        Ok(removed as u64)
    }

    /// Durably marks one generated temporary owner as live before any staging path or reference
    /// is created. A fresh service instance can therefore distinguish active work from crash
    /// residue. Sessions are bounded globally and must use unguessable owner ids at the caller.
    pub fn begin_temporary_owner_session(&self, temporary_owner: &AssetOwner) -> Result<()> {
        validate_owner(temporary_owner)?;
        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sessions: i64 = transaction.query_row(
            "SELECT count(*) FROM asset_temporary_owner_sessions",
            [],
            |row| row.get(0),
        )?;
        if sessions >= i64::from(MAX_TEMPORARY_OWNER_SESSIONS) {
            return Err(AssetError::InvalidInput {
                field: "temporary owner session",
                reason: "live session limit of 128 has been reached",
            });
        }
        let now = now_ms()?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO asset_temporary_owner_sessions(
                owner_type, owner_id, created_at_ms, lease_updated_at_ms
             ) VALUES (?1, ?2, ?3, ?3)",
            params![temporary_owner.owner_type, temporary_owner.owner_id, now],
        )?;
        if inserted != 1 {
            return Err(AssetError::InvalidInput {
                field: "temporary owner session",
                reason: "session already exists",
            });
        }
        transaction.commit()?;
        Ok(())
    }

    /// Renews a live temporary-owner lease. `false` means recovery already claimed the session;
    /// the importer must stop and roll back instead of publishing a success receipt.
    pub fn renew_temporary_owner_session(&self, temporary_owner: &AssetOwner) -> Result<bool> {
        validate_owner(temporary_owner)?;
        let _guard = self.mutation_guard()?;
        let connection = self.connection()?;
        let now = now_ms()?;
        let updated = connection.execute(
            "UPDATE asset_temporary_owner_sessions
             SET lease_updated_at_ms = max(lease_updated_at_ms, ?3)
             WHERE owner_type = ?1 AND owner_id = ?2",
            params![temporary_owner.owner_type, temporary_owner.owner_id, now],
        )?;
        Ok(updated == 1)
    }

    /// Returns whether recovery must currently preserve this staging session.
    pub fn temporary_owner_session_is_live(&self, temporary_owner: &AssetOwner) -> Result<bool> {
        validate_owner(temporary_owner)?;
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM asset_temporary_owner_sessions
                    WHERE owner_type = ?1 AND owner_id = ?2
                 )",
                params![temporary_owner.owner_type, temporary_owner.owner_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Ends a valid metadata-only temporary session. It fails closed if any asset reference was
    /// unexpectedly admitted, so callers cannot silently orphan a partially imported batch.
    pub fn finish_empty_temporary_owner_session(&self, temporary_owner: &AssetOwner) -> Result<()> {
        validate_owner(temporary_owner)?;
        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let refs: i64 = transaction.query_row(
            "SELECT count(*) FROM asset_refs WHERE owner_type = ?1 AND owner_id = ?2",
            params![temporary_owner.owner_type, temporary_owner.owner_id],
            |row| row.get(0),
        )?;
        if refs != 0 {
            return Err(AssetError::InvalidInput {
                field: "temporary owner session",
                reason: "empty session unexpectedly owns asset references",
            });
        }
        let removed = transaction.execute(
            "DELETE FROM asset_temporary_owner_sessions
             WHERE owner_type = ?1 AND owner_id = ?2",
            params![temporary_owner.owner_type, temporary_owner.owner_id],
        )?;
        if removed != 1 {
            return Err(AssetError::InvalidInput {
                field: "temporary owner session",
                reason: "session is not live",
            });
        }
        transaction.commit()?;
        Ok(())
    }

    /// Atomically promotes every reference held by a unique temporary owner to the final owner.
    ///
    /// Importers should ingest each object with a per-session temporary owner in a reserved owner
    /// type, then call this only after the complete batch has passed validation. Temporary and
    /// final owner types must differ, preventing a successful reference from remaining inside the
    /// namespace that startup recovery purges. A process failure observes either all final
    /// references or all temporary references; it can never expose a partially promoted batch.
    pub fn commit_temporary_owner_refs(
        &self,
        temporary_owner: &AssetOwner,
        final_owner: &AssetOwner,
        expected_unique_refs: u64,
    ) -> Result<u64> {
        validate_owner(temporary_owner)?;
        validate_owner(final_owner)?;
        if temporary_owner.owner_type == final_owner.owner_type {
            return Err(AssetError::InvalidInput {
                field: "temporary owner",
                reason: "temporary and final owner types must differ",
            });
        }
        if expected_unique_refs == 0 {
            return Err(AssetError::InvalidInput {
                field: "temporary owner references",
                reason: "expected unique reference count must be greater than zero",
            });
        }

        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let session_is_live: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM asset_temporary_owner_sessions
                WHERE owner_type = ?1 AND owner_id = ?2
             )",
            params![temporary_owner.owner_type, temporary_owner.owner_id],
            |row| row.get(0),
        )?;
        if !session_is_live {
            return Err(AssetError::InvalidInput {
                field: "temporary owner session",
                reason: "session is not live",
            });
        }
        let reference_count: i64 = transaction.query_row(
            "SELECT count(*) FROM asset_refs WHERE owner_type = ?1 AND owner_id = ?2",
            params![temporary_owner.owner_type, temporary_owner.owner_id],
            |row| row.get(0),
        )?;
        let expected_unique_refs =
            i64::try_from(expected_unique_refs).map_err(|_| AssetError::InvalidInput {
                field: "temporary owner references",
                reason: "expected unique reference count is too large",
            })?;
        if reference_count != expected_unique_refs {
            return Err(AssetError::InvalidInput {
                field: "temporary owner references",
                reason: "admitted unique reference count did not match the expected batch",
            });
        }
        let invalid: Option<(String, String)> = transaction
            .query_row(
                "SELECT o.hash, o.state
                 FROM asset_refs r
                 JOIN asset_objects o ON o.hash = r.hash
                 WHERE r.owner_type = ?1 AND r.owner_id = ?2 AND o.state != 'active'
                 ORDER BY o.hash LIMIT 1",
                params![temporary_owner.owner_type, temporary_owner.owner_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((hash, state)) = invalid {
            return Err(AssetError::NotActive { hash, state });
        }

        let now = now_ms()?;
        transaction.execute(
            "INSERT OR IGNORE INTO asset_refs(owner_type, owner_id, hash, created_at_ms)
             SELECT ?1, ?2, hash, ?3 FROM asset_refs
             WHERE owner_type = ?4 AND owner_id = ?5",
            params![
                final_owner.owner_type,
                final_owner.owner_id,
                now,
                temporary_owner.owner_type,
                temporary_owner.owner_id
            ],
        )?;
        let promoted = transaction.execute(
            "DELETE FROM asset_refs WHERE owner_type = ?1 AND owner_id = ?2",
            params![temporary_owner.owner_type, temporary_owner.owner_id],
        )? as u64;
        if promoted == 0 || promoted != expected_unique_refs as u64 {
            return Err(AssetError::IncompatibleCatalog {
                reason: "temporary owner promotion count changed inside one transaction",
            });
        }
        let removed_session = transaction.execute(
            "DELETE FROM asset_temporary_owner_sessions
             WHERE owner_type = ?1 AND owner_id = ?2",
            params![temporary_owner.owner_type, temporary_owner.owner_id],
        )?;
        if removed_session != 1 {
            return Err(AssetError::IncompatibleCatalog {
                reason: "temporary owner session disappeared during promotion",
            });
        }
        transaction.commit()?;
        Ok(promoted)
    }

    /// Rolls back one temporary-owner session without touching objects referenced by other owners.
    ///
    /// Each orphaned active object is unlinked and directory-synced before its reference and
    /// catalog row are deleted in one SQLite transaction. If the process stops after the unlink,
    /// the temporary reference remains durable, so calling this method again completes recovery.
    pub fn rollback_temporary_owner(&self, temporary_owner: &AssetOwner) -> Result<u64> {
        validate_owner(temporary_owner)?;
        let _guard = self.mutation_guard()?;
        self.rollback_temporary_owner_locked(temporary_owner)
    }

    /// Recovers orphaned or expired temporary sessions in one reserved owner namespace.
    ///
    /// The importer calls this before admitting a new request. Session discovery comes from the
    /// same durable catalog transaction that admitted each object, rather than from best-effort
    /// files in an external staging directory.
    pub fn recover_temporary_owner_type(&self, owner_type: &str) -> Result<u64> {
        self.recover_temporary_owner_type_before(
            owner_type,
            now_ms()?.saturating_sub(TEMPORARY_OWNER_LEASE_TIMEOUT_MS),
        )
    }

    /// Deterministic cutoff variant used by startup maintenance and recovery tests. A candidate
    /// lease is claimed with an IMMEDIATE transaction before files are touched, so renewal either
    /// wins first or observes that recovery owns the session and forces the import to abort.
    pub fn recover_temporary_owner_type_before(
        &self,
        owner_type: &str,
        stale_before_ms: i64,
    ) -> Result<u64> {
        if stale_before_ms < 0 {
            return Err(AssetError::InvalidInput {
                field: "temporary owner stale cutoff",
                reason: "must be non-negative milliseconds since epoch",
            });
        }
        let validated = AssetOwner::new(owner_type, "recovery-probe")?;
        let _guard = self.mutation_guard()?;
        let mut removed = 0u64;
        loop {
            let mut connection = self.connection()?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let candidate: Option<(String, bool)> = transaction
                .query_row(
                    "SELECT owner_id, has_lease FROM (
                        SELECT owner_id, 1 AS has_lease
                        FROM asset_temporary_owner_sessions
                        WHERE owner_type = ?1 AND lease_updated_at_ms < ?2
                        UNION ALL
                        SELECT DISTINCT r.owner_id, 0 AS has_lease
                        FROM asset_refs r
                        WHERE r.owner_type = ?1
                          AND NOT EXISTS(
                            SELECT 1 FROM asset_temporary_owner_sessions s
                            WHERE s.owner_type = r.owner_type AND s.owner_id = r.owner_id
                          )
                     ) ORDER BY owner_id LIMIT 1",
                    params![validated.owner_type, stale_before_ms],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((owner_id, has_lease)) = candidate else {
                transaction.commit()?;
                break;
            };
            if has_lease {
                let claimed = transaction.execute(
                    "DELETE FROM asset_temporary_owner_sessions
                     WHERE owner_type = ?1 AND owner_id = ?2 AND lease_updated_at_ms < ?3",
                    params![validated.owner_type, owner_id, stale_before_ms],
                )?;
                if claimed != 1 {
                    transaction.commit()?;
                    continue;
                }
            }
            transaction.commit()?;
            let owner = AssetOwner::new(validated.owner_type.clone(), owner_id)?;
            removed = removed.saturating_add(self.rollback_temporary_owner_locked(&owner)?);
        }
        Ok(removed)
    }

    fn recover_expired_temporary_owner_sessions(&self) -> Result<u64> {
        let cutoff = now_ms()?.saturating_sub(TEMPORARY_OWNER_LEASE_TIMEOUT_MS);
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT DISTINCT owner_type FROM asset_temporary_owner_sessions
             WHERE lease_updated_at_ms < ?1 ORDER BY owner_type LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![cutoff, i64::from(MAX_TEMPORARY_OWNER_SESSIONS)],
            |row| row.get::<_, String>(0),
        )?;
        let mut owner_types = Vec::new();
        for row in rows {
            owner_types.push(row?);
        }
        drop(statement);
        drop(connection);

        let mut removed = 0u64;
        for owner_type in owner_types {
            removed = removed
                .saturating_add(self.recover_temporary_owner_type_before(&owner_type, cutoff)?);
        }
        Ok(removed)
    }

    fn rollback_temporary_owner_locked(&self, temporary_owner: &AssetOwner) -> Result<u64> {
        let mut removed_objects = 0u64;
        loop {
            let mut connection = self.connection()?;
            let next_hash: Option<String> = connection
                .query_row(
                    "SELECT hash FROM asset_refs
                     WHERE owner_type = ?1 AND owner_id = ?2 ORDER BY hash LIMIT 1",
                    params![temporary_owner.owner_type, temporary_owner.owner_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(next_hash) = next_hash else {
                break;
            };
            let hash = AssetHash::parse(next_hash)?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let object_state: Option<String> = transaction
                .query_row(
                    "SELECT state FROM asset_objects WHERE hash = ?1",
                    params![hash.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let has_other_reference: bool = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM asset_refs
                    WHERE hash = ?1
                      AND NOT (owner_type = ?2 AND owner_id = ?3)
                 )",
                params![
                    hash.as_str(),
                    temporary_owner.owner_type,
                    temporary_owner.owner_id
                ],
                |row| row.get(0),
            )?;

            if object_state.as_deref() == Some("active") && !has_other_reference {
                if let Some(locator) = self.object_locator(&hash, false)? {
                    match locator.directory.remove_file(&locator.name) {
                        Ok(()) => locator.directory.sync()?,
                        Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                rollback_failpoint("after_object_unlink")?;
            }

            transaction.execute(
                "DELETE FROM asset_refs
                 WHERE owner_type = ?1 AND owner_id = ?2 AND hash = ?3",
                params![
                    temporary_owner.owner_type,
                    temporary_owner.owner_id,
                    hash.as_str()
                ],
            )?;
            if object_state.as_deref() == Some("active") && !has_other_reference {
                removed_objects = removed_objects.saturating_add(transaction.execute(
                    "DELETE FROM asset_objects
                     WHERE hash = ?1
                       AND NOT EXISTS(SELECT 1 FROM asset_refs WHERE hash = ?1)",
                    params![hash.as_str()],
                )? as u64);
            }
            transaction.commit()?;
        }
        let connection = self.connection()?;
        connection.execute(
            "DELETE FROM asset_temporary_owner_sessions
             WHERE owner_type = ?1 AND owner_id = ?2",
            params![temporary_owner.owner_type, temporary_owner.owner_id],
        )?;
        Ok(removed_objects)
    }

    pub fn stats(&self) -> Result<AssetStats> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT object_count, active_bytes, reference_count,
                        missing_count, quarantine_count, staging_count
                 FROM asset_totals WHERE id = 1",
                [],
                |row| {
                    let object_count: i64 = row.get(0)?;
                    let active_bytes: i64 = row.get(1)?;
                    let reference_count: i64 = row.get(2)?;
                    let missing_count: i64 = row.get(3)?;
                    let quarantined_count: i64 = row.get(4)?;
                    let staging_count: i64 = row.get(5)?;
                    Ok(AssetStats {
                        object_count: object_count as u64,
                        active_bytes: active_bytes as u64,
                        reference_count: reference_count as u64,
                        missing_count: missing_count as u64,
                        quarantined_count: quarantined_count as u64,
                        staging_count: staging_count as u64,
                    })
                },
            )
            .map_err(Into::into)
    }

    /// Explicitly repeats the O(N) database-only audit that catalog open and restore validation
    /// perform. Ingest itself continues to use the trigger-maintained O(1) quota ledger.
    pub fn verify_catalog_ledger(&self) -> Result<AssetStats> {
        let connection = self.connection()?;
        catalog::verify_derived_totals(&connection)
    }

    pub fn open_object(&self, hash: &AssetHash) -> Result<AssetReader> {
        let object = self.get_object(hash)?.ok_or_else(|| AssetError::NotFound {
            hash: hash.to_string(),
        })?;
        if object.state != AssetState::Active {
            return Err(AssetError::NotActive {
                hash: hash.to_string(),
                state: object.state.as_str().to_owned(),
            });
        }
        let locator = self
            .object_locator(hash, false)?
            .ok_or_else(|| AssetError::NotFound {
                hash: hash.to_string(),
            })?;
        let file = locator.directory.open_file(&locator.name)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() != object.size {
            return Err(AssetError::UnsafeFilesystem {
                path: locator.path().display().to_string(),
                reason: "object is not a regular file with the catalogued size".to_owned(),
            });
        }
        Ok(AssetReader { file, object })
    }

    pub fn export_object<W, C>(
        &self,
        hash: &AssetHash,
        writer: &mut W,
        mut is_cancelled: C,
    ) -> Result<ExportedObject>
    where
        W: Write,
        C: FnMut() -> bool,
    {
        let mut reader = self.open_object(hash)?;
        let mut buffer = [0u8; COPY_BUFFER_BYTES];
        let mut bytes_written = 0u64;
        loop {
            if is_cancelled() {
                return Err(AssetError::Cancelled);
            }
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read])?;
            bytes_written = bytes_written.saturating_add(read as u64);
        }
        writer.flush()?;
        Ok(ExportedObject {
            hash: hash.clone(),
            bytes_written,
        })
    }

    pub fn reconcile_catalog_page<C>(
        &self,
        after: Option<&AssetHash>,
        limit: u16,
        mut is_cancelled: C,
    ) -> Result<ReconcilePage>
    where
        C: FnMut() -> bool,
    {
        validate_page_size(limit)?;
        let _guard = self.mutation_guard()?;
        let page = self.list_objects(after, limit.saturating_add(1).min(MAX_PAGE_SIZE))?;
        let mut objects = page.objects;
        let has_more = objects.len() > usize::from(limit) || page.next_cursor.is_some();
        objects.truncate(usize::from(limit));
        let mut findings = Vec::new();
        let now = now_ms()?;
        let connection = self.connection()?;

        for object in &objects {
            if is_cancelled() {
                return Err(AssetError::Cancelled);
            }
            let expected_relative = relative_object_path(&object.hash);
            if object.relative_path != expected_relative {
                connection.execute(
                    "UPDATE asset_objects SET relative_path = ?2 WHERE hash = ?1",
                    params![object.hash.as_str(), expected_relative],
                )?;
                findings.push(ReconcileFinding {
                    hash: Some(object.hash.clone()),
                    kind: ReconcileFindingKind::MetadataPathRepaired,
                    detail: "catalog path was reset to the hash-derived locator".to_owned(),
                });
            }

            let locator = self.object_locator(&object.hash, false)?;
            let opened = match locator.as_ref() {
                Some(locator) => locator.directory.open_file(&locator.name),
                None => Err(AssetError::Io(std::io::Error::from(ErrorKind::NotFound))),
            };
            match opened {
                Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                    if object.state != AssetState::Missing {
                        connection.execute(
                            "UPDATE asset_objects
                             SET state = 'missing', quarantine_name = NULL
                             WHERE hash = ?1",
                            params![object.hash.as_str()],
                        )?;
                        findings.push(ReconcileFinding {
                            hash: Some(object.hash.clone()),
                            kind: ReconcileFindingKind::Missing,
                            detail: "catalogue entry has no object file".to_owned(),
                        });
                    }
                }
                Err(AssetError::UnsafeFilesystem { .. }) => {
                    findings.push(ReconcileFinding {
                        hash: Some(object.hash.clone()),
                        kind: ReconcileFindingKind::UnsafeEntry,
                        detail: "object locator is not a regular no-follow file".to_owned(),
                    });
                }
                Err(error) => return Err(error),
                Ok(file) => {
                    let locator = locator.as_ref().expect("an opened object has a locator");
                    let (actual_hash, actual_size) = hash_open_file(
                        file,
                        &mut is_cancelled,
                        self.inner.limits.max_object_bytes,
                    )?;
                    if actual_hash != object.hash || actual_size != object.size {
                        self.quarantine_file(
                            &locator.directory,
                            &locator.name,
                            &relative_object_path(&object.hash),
                            Some(&object.hash),
                            "catalog hash or size mismatch",
                            now,
                        )?;
                        findings.push(ReconcileFinding {
                            hash: Some(object.hash.clone()),
                            kind: ReconcileFindingKind::Corrupt,
                            detail: "corrupt object moved to quarantine".to_owned(),
                        });
                    } else {
                        connection.execute(
                            "UPDATE asset_objects
                             SET state = 'active', quarantine_name = NULL, verified_at_ms = ?2
                             WHERE hash = ?1",
                            params![object.hash.as_str(), now],
                        )?;
                        if object.state != AssetState::Active {
                            findings.push(ReconcileFinding {
                                hash: Some(object.hash.clone()),
                                kind: ReconcileFindingKind::Restored,
                                detail: "valid object file restored the active state".to_owned(),
                            });
                        }
                    }
                }
            }
        }
        let next_cursor = has_more.then(|| objects.last().expect("non-empty page").hash.clone());
        Ok(ReconcilePage {
            checked: objects.len() as u16,
            findings,
            next_cursor,
        })
    }

    pub fn mark_sweep_page<C>(
        &self,
        after: Option<&AssetHash>,
        limit: u16,
        orphaned_before_ms: i64,
        mut is_cancelled: C,
    ) -> Result<MarkSweepPage>
    where
        C: FnMut() -> bool,
    {
        validate_page_size(limit)?;
        if orphaned_before_ms < 0 {
            return Err(AssetError::InvalidInput {
                field: "orphan cutoff",
                reason: "must not be negative",
            });
        }
        let _guard = self.mutation_guard()?;
        let mut connection = self.connection()?;
        let fetch = i64::from(limit) + 1;
        let mut hashes = Vec::new();
        if let Some(after) = after {
            let mut statement = connection.prepare(
                "SELECT hash FROM asset_objects o
                 WHERE hash > ?1 AND created_at_ms <= ?2
                   AND NOT EXISTS(SELECT 1 FROM asset_refs r WHERE r.hash = o.hash)
                 ORDER BY hash LIMIT ?3",
            )?;
            let rows = statement
                .query_map(params![after.as_str(), orphaned_before_ms, fetch], |row| {
                    row.get::<_, String>(0)
                })?;
            for row in rows {
                hashes.push(AssetHash::parse(row?)?);
            }
        } else {
            let mut statement = connection.prepare(
                "SELECT hash FROM asset_objects o
                 WHERE created_at_ms <= ?1
                   AND NOT EXISTS(SELECT 1 FROM asset_refs r WHERE r.hash = o.hash)
                 ORDER BY hash LIMIT ?2",
            )?;
            let rows = statement.query_map(params![orphaned_before_ms, fetch], |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                hashes.push(AssetHash::parse(row?)?);
            }
        }
        let has_more = hashes.len() > usize::from(limit);
        hashes.truncate(usize::from(limit));
        let next_cursor = has_more.then(|| hashes.last().expect("non-empty page").clone());
        let mut removed = Vec::with_capacity(hashes.len());
        for hash in hashes {
            if is_cancelled() {
                return Err(AssetError::Cancelled);
            }
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let remains_orphan: bool = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM asset_objects o
                    WHERE o.hash = ?1 AND o.created_at_ms <= ?2
                      AND NOT EXISTS(SELECT 1 FROM asset_refs r WHERE r.hash = o.hash)
                 )",
                params![hash.as_str(), orphaned_before_ms],
                |row| row.get(0),
            )?;
            if !remains_orphan {
                transaction.commit()?;
                continue;
            }
            if let Some(locator) = self.object_locator(&hash, false)? {
                match locator.directory.open_file(&locator.name) {
                    Ok(_) => {
                        locator.directory.remove_file(&locator.name)?;
                        locator.directory.sync()?;
                    }
                    Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            transaction.execute(
                "DELETE FROM asset_objects
                 WHERE hash = ?1
                   AND NOT EXISTS(SELECT 1 FROM asset_refs WHERE hash = ?1)",
                params![hash.as_str()],
            )?;
            transaction.commit()?;
            removed.push(hash);
        }
        Ok(MarkSweepPage {
            removed,
            next_cursor,
        })
    }

    pub fn cleanup_staging_page(
        &self,
        after: Option<&str>,
        limit: u16,
        created_before_ms: i64,
    ) -> Result<CleanupPage> {
        self.cleanup_internal(
            "asset_staging",
            &self.inner.staging_directory,
            after,
            limit,
            created_before_ms,
        )
    }

    pub fn purge_quarantine_page(
        &self,
        after: Option<&str>,
        limit: u16,
        created_before_ms: i64,
    ) -> Result<CleanupPage> {
        self.cleanup_internal(
            "asset_quarantine",
            &self.inner.quarantine_directory,
            after,
            limit,
            created_before_ms,
        )
    }

    /// Opens an explicit bounded reconciliation cursor for one of the 65,536 leaf shards.
    /// Startup never calls this operation and the cursor reads at most `limit + 1` entries per
    /// `next_page` invocation.
    pub fn begin_shard_reconciliation(&self, shard: &str) -> Result<ShardReconciler> {
        validate_shard(shard)?;
        let directory = match self
            .inner
            .objects_directory
            .open_child(OsStr::new(&shard[..2]))
        {
            Ok(first) => match first.open_child(OsStr::new(&shard[2..])) {
                Ok(second) => Some(second),
                Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            },
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let entries = directory
            .as_ref()
            .map(SecureDirectory::read_dir)
            .transpose()?;
        Ok(ShardReconciler {
            store: self.clone(),
            shard: shard.to_owned(),
            directory,
            entries,
            pending: VecDeque::new(),
        })
    }

    fn reconcile_shard_entry(
        &self,
        connection: &Connection,
        shard: &str,
        directory: &SecureDirectory,
        entry: &SecureDirEntry,
        now: i64,
    ) -> Result<Option<ReconcileFinding>> {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Ok(Some(ReconcileFinding {
                hash: None,
                kind: ReconcileFindingKind::UnsafeEntry,
                detail: "non-UTF-8 shard entry left untouched".to_owned(),
            }));
        };
        match directory.open_file(OsStr::new(&name)) {
            Ok(_) => {}
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(AssetError::UnsafeFilesystem { .. }) => {
                return Ok(Some(ReconcileFinding {
                    hash: None,
                    kind: ReconcileFindingKind::UnsafeEntry,
                    detail: format!("unsafe entry left untouched: {name}"),
                }));
            }
            Err(error) => return Err(error),
        }
        let hash = AssetHash::from_str(&name).ok();
        let is_expected_hash = hash
            .as_ref()
            .is_some_and(|hash| hash.as_str().starts_with(shard));
        let catalogued = if let Some(hash) = hash.as_ref().filter(|_| is_expected_hash) {
            catalog::get_object(connection, hash)?.is_some()
        } else {
            false
        };
        if catalogued {
            return Ok(None);
        }
        let reason = if is_expected_hash {
            "filesystem object has no catalog row"
        } else {
            "object shard contains an invalid filename"
        };
        let source_relative_path = format!("objects/{}/{}/{}", &shard[..2], &shard[2..], name);
        self.quarantine_file(
            directory,
            OsStr::new(&name),
            &source_relative_path,
            hash.as_ref(),
            reason,
            now,
        )?;
        Ok(Some(ReconcileFinding {
            hash,
            kind: if is_expected_hash {
                ReconcileFindingKind::FilesystemOrphanQuarantined
            } else {
                ReconcileFindingKind::InvalidEntryQuarantined
            },
            detail: reason.to_owned(),
        }))
    }

    fn cleanup_internal(
        &self,
        table: &'static str,
        directory: &SecureDirectory,
        after: Option<&str>,
        limit: u16,
        created_before_ms: i64,
    ) -> Result<CleanupPage> {
        validate_page_size(limit)?;
        if created_before_ms < 0 {
            return Err(AssetError::InvalidInput {
                field: "cleanup cutoff",
                reason: "must not be negative",
            });
        }
        let _guard = self.mutation_guard()?;
        let connection = self.connection()?;
        let query = match (table, after.is_some()) {
            ("asset_staging", true) => {
                "SELECT name FROM asset_staging
                 WHERE name > ?1 AND created_at_ms <= ?2 ORDER BY name LIMIT ?3"
            }
            ("asset_staging", false) => {
                "SELECT name FROM asset_staging
                 WHERE created_at_ms <= ?2 ORDER BY name LIMIT ?3"
            }
            ("asset_quarantine", true) => {
                "SELECT name FROM asset_quarantine
                 WHERE name > ?1 AND created_at_ms <= ?2 ORDER BY name LIMIT ?3"
            }
            ("asset_quarantine", false) => {
                "SELECT name FROM asset_quarantine
                 WHERE created_at_ms <= ?2 ORDER BY name LIMIT ?3"
            }
            _ => unreachable!("internal table is fixed"),
        };
        let fetch = i64::from(limit) + 1;
        let after_value = after.unwrap_or("");
        let mut statement = connection.prepare(query)?;
        let rows = statement.query_map(params![after_value, created_before_ms, fetch], |row| {
            row.get::<_, String>(0)
        })?;
        let mut names = Vec::new();
        for row in rows {
            names.push(row?);
        }
        drop(statement);
        let has_more = names.len() > usize::from(limit);
        names.truncate(usize::from(limit));
        for name in &names {
            validate_internal_name(name)?;
            if table == "asset_staging" {
                match directory.open_file(OsStr::new(name)) {
                    Ok(_) => {
                        directory.remove_file(OsStr::new(name))?;
                        directory.sync()?;
                    }
                    Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                        directory.sync()?;
                    }
                    Err(error) => return Err(error),
                }
                connection.execute("DELETE FROM asset_staging WHERE name = ?1", params![name])?;
            } else {
                self.purge_quarantine_file(name)?;
            }
        }
        let next_cursor = has_more.then(|| names.last().expect("non-empty page").clone());
        Ok(CleanupPage {
            removed_names: names,
            next_cursor,
        })
    }

    fn allocate_staging(&self) -> Result<StagedFile> {
        let now = now_ms()?;
        for _ in 0..8 {
            let name = format!("{}.partial", Uuid::new_v4().simple());
            let connection = self.connection()?;
            let inserted = connection.execute(
                "INSERT OR IGNORE INTO asset_staging(name, created_at_ms) VALUES (?1, ?2)",
                params![name, now],
            )?;
            if inserted == 0 {
                continue;
            }
            match self
                .inner
                .staging_directory
                .create_new_file(OsStr::new(&name))
            {
                Ok(file) => {
                    return Ok(StagedFile {
                        name,
                        directory: self.inner.staging_directory.clone(),
                        file: Some(file),
                    });
                }
                Err(AssetError::Io(error)) => {
                    connection
                        .execute("DELETE FROM asset_staging WHERE name = ?1", params![name])?;
                    if error.kind() != ErrorKind::AlreadyExists {
                        return Err(error.into());
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(AssetError::InvalidInput {
            field: "staging file",
            reason: "could not allocate a unique staging name",
        })
    }

    fn remove_staging_record(&self, name: &str) -> Result<()> {
        let connection = self.connection()?;
        connection.execute("DELETE FROM asset_staging WHERE name = ?1", params![name])?;
        Ok(())
    }

    fn cleanup_staged_file(&self, staged: &mut StagedFile) -> Result<()> {
        drop(staged.file.take());
        staging_cleanup_failpoint("before_unlink")?;
        match staged.directory.remove_file(OsStr::new(&staged.name)) {
            Ok(()) => {}
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        staged.directory.sync()?;
        staging_cleanup_failpoint("before_record_delete")?;
        self.remove_staging_record(&staged.name)
    }

    fn prepare_destination<C>(
        &self,
        destination: &ObjectLocator,
        expected: PublishExpectation<'_>,
        is_cancelled: &mut C,
    ) -> Result<bool>
    where
        C: FnMut() -> bool,
    {
        let file = match destination.directory.open_file(&destination.name) {
            Ok(file) => file,
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let (actual_hash, actual_size) =
            hash_open_file(file, is_cancelled, self.inner.limits.max_object_bytes)?;
        if actual_hash == *expected.hash && actual_size == expected.size {
            return Ok(true);
        }
        self.quarantine_file(
            &destination.directory,
            &destination.name,
            &relative_object_path(expected.hash),
            Some(expected.hash),
            "object path content did not match its hash",
            expected.now,
        )?;
        Ok(false)
    }

    fn publish_or_deduplicate<C>(
        &self,
        staging_name: &str,
        destination: &ObjectLocator,
        expected: PublishExpectation<'_>,
        is_cancelled: &mut C,
    ) -> Result<bool>
    where
        C: FnMut() -> bool,
    {
        match self.inner.staging_directory.hard_link_to(
            OsStr::new(staging_name),
            &destination.directory,
            &destination.name,
        ) {
            Ok(()) => Ok(false),
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::AlreadyExists => {
                let file = destination.directory.open_file(&destination.name)?;
                let (actual_hash, actual_size) =
                    hash_open_file(file, is_cancelled, self.inner.limits.max_object_bytes)?;
                if actual_hash == *expected.hash && actual_size == expected.size {
                    Ok(true)
                } else {
                    Err(AssetError::HashMetadataConflict {
                        hash: expected.hash.to_string(),
                    })
                }
            }
            Err(error) => Err(error),
        }
    }

    fn destination_matches_expectation<C>(
        &self,
        destination: &ObjectLocator,
        expected: PublishExpectation<'_>,
        is_cancelled: &mut C,
    ) -> Result<bool>
    where
        C: FnMut() -> bool,
    {
        let file = match destination.directory.open_file(&destination.name) {
            Ok(file) => file,
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let (actual_hash, actual_size) =
            hash_open_file(file, is_cancelled, self.inner.limits.max_object_bytes)?;
        if actual_hash == *expected.hash && actual_size == expected.size {
            Ok(true)
        } else {
            Err(AssetError::HashMetadataConflict {
                hash: expected.hash.to_string(),
            })
        }
    }

    fn quarantine_file(
        &self,
        source_directory: &SecureDirectory,
        source_name: &OsStr,
        source_relative_path: &str,
        original_hash: Option<&AssetHash>,
        reason: &str,
        now: i64,
    ) -> Result<String> {
        source_directory.open_file(source_name)?;
        let label = original_hash.map_or("unknown", AssetHash::as_str);
        let name = format!("{}-{}.quarantine", label, Uuid::new_v4().simple());
        validate_internal_name(&name)?;
        validate_object_relative_path(source_relative_path)?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO asset_quarantine_intents(
                name, operation, phase, source_relative_path,
                original_hash, reason, created_at_ms
             ) VALUES (?1, 'move', 'prepared', ?2, ?3, ?4, ?5)",
            params![
                name,
                source_relative_path,
                original_hash.map(AssetHash::as_str),
                reason,
                now
            ],
        )?;
        let intent = QuarantineIntent {
            name: name.clone(),
            operation: QuarantineOperation::Move,
            phase: QuarantinePhase::Prepared,
            source_relative_path: Some(source_relative_path.to_owned()),
            original_hash: original_hash.cloned(),
            reason: reason.to_owned(),
            created_at_ms: now,
        };
        quarantine_failpoint("after_move_intent")?;
        let source = ObjectLocator {
            directory: source_directory.clone(),
            name: source_name.to_os_string(),
        };
        self.complete_quarantine_move(&intent, Some(&source))?;
        Ok(name)
    }

    fn ensure_object_parent(&self, hash: &AssetHash) -> Result<ObjectLocator> {
        self.object_locator(hash, true)?
            .ok_or(AssetError::IncompatibleCatalog {
                reason: "object shard could not be created",
            })
    }

    fn object_locator(&self, hash: &AssetHash, create: bool) -> Result<Option<ObjectLocator>> {
        let first = if create {
            self.inner
                .objects_directory
                .open_or_create_child(OsStr::new(&hash.as_str()[..2]))?
        } else {
            match self
                .inner
                .objects_directory
                .open_child(OsStr::new(&hash.as_str()[..2]))
            {
                Ok(directory) => directory,
                Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        };
        let second = if create {
            first.open_or_create_child(OsStr::new(&hash.as_str()[2..4]))?
        } else {
            match first.open_child(OsStr::new(&hash.as_str()[2..4])) {
                Ok(directory) => directory,
                Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        };
        if create {
            self.inner.objects_directory.sync()?;
            first.sync()?;
        }
        Ok(Some(ObjectLocator {
            directory: second,
            name: OsString::from(hash.as_str()),
        }))
    }

    fn recover_quarantine_intents(&self) -> Result<()> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT name, operation, phase, source_relative_path,
                    original_hash, reason, created_at_ms
             FROM asset_quarantine_intents ORDER BY created_at_ms, name",
        )?;
        let rows = statement.query_map([], decode_quarantine_intent)?;
        let mut intents = Vec::new();
        for row in rows {
            intents.push(row?);
        }
        drop(statement);
        drop(connection);

        for intent in intents {
            match intent.operation {
                QuarantineOperation::Move => {
                    let source_path = intent.source_relative_path.as_deref().ok_or(
                        AssetError::IncompatibleCatalog {
                            reason: "quarantine move intent has no source locator",
                        },
                    )?;
                    let source = self.resolve_object_relative_path(source_path)?;
                    self.complete_quarantine_move(&intent, source.as_ref())?;
                }
                QuarantineOperation::Purge => self.complete_quarantine_purge(&intent)?,
            }
        }
        Ok(())
    }

    fn resolve_object_relative_path(&self, value: &str) -> Result<Option<ObjectLocator>> {
        validate_object_relative_path(value)?;
        let mut parts = value.split('/');
        let _objects = parts.next().expect("validated object prefix");
        let first_name = parts.next().expect("validated first shard");
        let second_name = parts.next().expect("validated second shard");
        let file_name = parts.next().expect("validated object filename");
        let first = match self
            .inner
            .objects_directory
            .open_child(OsStr::new(first_name))
        {
            Ok(directory) => directory,
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let second = match first.open_child(OsStr::new(second_name)) {
            Ok(directory) => directory,
            Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Some(ObjectLocator {
            directory: second,
            name: OsString::from(file_name),
        }))
    }

    fn complete_quarantine_move(
        &self,
        intent: &QuarantineIntent,
        source: Option<&ObjectLocator>,
    ) -> Result<()> {
        if intent.phase == QuarantinePhase::Prepared {
            let target_exists = self
                .inner
                .quarantine_directory
                .file_exists(OsStr::new(&intent.name))?;
            let source_exists = source
                .map(|source| source.directory.file_exists(&source.name))
                .transpose()?
                .unwrap_or(false);
            match (source_exists, target_exists) {
                (true, false) => {
                    let source = source.expect("an existing source has a locator");
                    source.directory.move_file_no_replace(
                        &source.name,
                        &self.inner.quarantine_directory,
                        OsStr::new(&intent.name),
                    )?;
                }
                (false, true) => {}
                (true, true) => {
                    let source = source.expect("an existing source has a locator");
                    if !source.directory.same_file_as(
                        &source.name,
                        &self.inner.quarantine_directory,
                        OsStr::new(&intent.name),
                    )? {
                        return Err(AssetError::UnsafeFilesystem {
                            path: source.path().display().to_string(),
                            reason: "quarantine recovery found two different files for one intent"
                                .to_owned(),
                        });
                    }
                    source.directory.remove_file(&source.name)?;
                    source.directory.sync()?;
                }
                (false, false) => {
                    return Err(AssetError::UnsafeFilesystem {
                        path: intent
                            .source_relative_path
                            .as_deref()
                            .unwrap_or("<missing quarantine source>")
                            .to_owned(),
                        reason: "quarantine intent lost both its source and destination".to_owned(),
                    });
                }
            }
            quarantine_failpoint("after_filesystem_move")?;
            let connection = self.connection()?;
            connection.execute(
                "UPDATE asset_quarantine_intents SET phase = 'moved' WHERE name = ?1",
                params![intent.name],
            )?;
        }

        quarantine_failpoint("before_move_finalize")?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO asset_quarantine(name, original_hash, reason, created_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET
                original_hash = excluded.original_hash,
                reason = excluded.reason,
                created_at_ms = excluded.created_at_ms",
            params![
                intent.name,
                intent.original_hash.as_ref().map(AssetHash::as_str),
                intent.reason,
                intent.created_at_ms
            ],
        )?;
        if let Some(hash) = intent.original_hash.as_ref() {
            transaction.execute(
                "UPDATE asset_objects
                 SET state = 'quarantined', quarantine_name = ?2
                 WHERE hash = ?1",
                params![hash.as_str(), intent.name],
            )?;
        }
        transaction.execute(
            "DELETE FROM asset_quarantine_intents WHERE name = ?1",
            params![intent.name],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn purge_quarantine_file(&self, name: &str) -> Result<()> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT original_hash, reason, created_at_ms
                 FROM asset_quarantine WHERE name = ?1",
                params![name],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((original_hash, reason, created_at_ms)) = row else {
            return Ok(());
        };
        let original_hash = original_hash.map(AssetHash::parse).transpose()?;
        connection.execute(
            "INSERT INTO asset_quarantine_intents(
                name, operation, phase, source_relative_path,
                original_hash, reason, created_at_ms
             ) VALUES (?1, 'purge', 'prepared', NULL, ?2, ?3, ?4)",
            params![
                name,
                original_hash.as_ref().map(AssetHash::as_str),
                reason,
                created_at_ms
            ],
        )?;
        let intent = QuarantineIntent {
            name: name.to_owned(),
            operation: QuarantineOperation::Purge,
            phase: QuarantinePhase::Prepared,
            source_relative_path: None,
            original_hash,
            reason,
            created_at_ms,
        };
        quarantine_failpoint("after_purge_intent")?;
        self.complete_quarantine_purge(&intent)
    }

    fn complete_quarantine_purge(&self, intent: &QuarantineIntent) -> Result<()> {
        if intent.phase == QuarantinePhase::Prepared {
            match self
                .inner
                .quarantine_directory
                .open_file(OsStr::new(&intent.name))
            {
                Ok(_) => {
                    self.inner
                        .quarantine_directory
                        .remove_file(OsStr::new(&intent.name))?;
                    self.inner.quarantine_directory.sync()?;
                }
                Err(AssetError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                    self.inner.quarantine_directory.sync()?;
                }
                Err(error) => return Err(error),
            }
            quarantine_failpoint("after_quarantine_unlink")?;
            let connection = self.connection()?;
            connection.execute(
                "UPDATE asset_quarantine_intents SET phase = 'moved' WHERE name = ?1",
                params![intent.name],
            )?;
        }

        quarantine_failpoint("before_purge_finalize")?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE asset_objects
             SET state = 'missing', quarantine_name = NULL
             WHERE state = 'quarantined' AND quarantine_name = ?1",
            params![intent.name],
        )?;
        transaction.execute(
            "DELETE FROM asset_quarantine WHERE name = ?1",
            params![intent.name],
        )?;
        transaction.execute(
            "DELETE FROM asset_quarantine_intents WHERE name = ?1",
            params![intent.name],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn connection(&self) -> Result<Connection> {
        self.inner.root_directory.ensure_path_identity()?;
        reject_symlink_if_present(&self.inner.catalog)?;
        catalog::connect(&self.inner.catalog)
    }

    fn mutation_guard(&self) -> Result<MutexGuard<'_, ()>> {
        match self.inner.mutation_lock.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::WouldBlock) => Err(AssetError::MutationBusy),
            Err(TryLockError::Poisoned(_)) => Err(AssetError::LockPoisoned),
        }
    }
}

#[derive(Debug)]
struct ObjectLocator {
    directory: SecureDirectory,
    name: OsString,
}

impl ObjectLocator {
    fn path(&self) -> PathBuf {
        self.directory.path().join(&self.name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuarantineOperation {
    Move,
    Purge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuarantinePhase {
    Prepared,
    Moved,
}

#[derive(Debug)]
struct QuarantineIntent {
    name: String,
    operation: QuarantineOperation,
    phase: QuarantinePhase,
    source_relative_path: Option<String>,
    original_hash: Option<AssetHash>,
    reason: String,
    created_at_ms: i64,
}

fn decode_quarantine_intent(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuarantineIntent> {
    let name: String = row.get(0)?;
    let operation: String = row.get(1)?;
    let phase: String = row.get(2)?;
    let source_relative_path: Option<String> = row.get(3)?;
    let original_hash: Option<String> = row.get(4)?;
    let reason: String = row.get(5)?;
    let created_at_ms: i64 = row.get(6)?;
    let decode_error = |value: String, reason: &'static str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(AssetError::UnsafeFilesystem {
                path: value,
                reason: reason.to_owned(),
            }),
        )
    };
    let operation = match operation.as_str() {
        "move" => QuarantineOperation::Move,
        "purge" => QuarantineOperation::Purge,
        _ => return Err(decode_error(operation, "invalid quarantine operation")),
    };
    let phase = match phase.as_str() {
        "prepared" => QuarantinePhase::Prepared,
        "moved" => QuarantinePhase::Moved,
        _ => return Err(decode_error(phase, "invalid quarantine phase")),
    };
    validate_internal_name(&name).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    match (operation, source_relative_path.as_deref()) {
        (QuarantineOperation::Move, Some(path)) => {
            validate_object_relative_path(path).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        }
        (QuarantineOperation::Purge, None) => {}
        _ => {
            return Err(decode_error(
                name,
                "quarantine operation and source locator disagree",
            ));
        }
    }
    if reason.is_empty() || reason.len() > 256 || reason.contains('\0') || created_at_ms < 0 {
        return Err(decode_error(
            name,
            "quarantine intent metadata is outside the bounded contract",
        ));
    }
    let original_hash = original_hash
        .map(AssetHash::parse)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(QuarantineIntent {
        name,
        operation,
        phase,
        source_relative_path,
        original_hash,
        reason,
        created_at_ms,
    })
}

#[derive(Debug)]
struct StagedFile {
    name: String,
    directory: SecureDirectory,
    file: Option<File>,
}

#[derive(Clone, Copy)]
struct PublishExpectation<'a> {
    hash: &'a AssetHash,
    size: u64,
    now: i64,
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        drop(self.file.take());
    }
}

struct CopiedObject {
    digest: Vec<u8>,
    size: u64,
    prefix: Vec<u8>,
    tail: Vec<u8>,
}

fn copy_and_hash<R, C>(
    reader: &mut R,
    writer: &mut File,
    max_bytes: u64,
    is_cancelled: &mut C,
) -> Result<CopiedObject>
where
    R: Read,
    C: FnMut() -> bool,
{
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut prefix = Vec::with_capacity(PROBE_BYTES);
    let mut tail = Vec::with_capacity(TAIL_BYTES);
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    loop {
        if is_cancelled() {
            return Err(AssetError::Cancelled);
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .ok_or(AssetError::LimitExceeded {
                limit_name: "per-object quota",
                limit: max_bytes,
            })?;
        if size > max_bytes {
            return Err(AssetError::LimitExceeded {
                limit_name: "per-object quota",
                limit: max_bytes,
            });
        }
        let chunk = &buffer[..read];
        writer.write_all(chunk)?;
        hasher.update(chunk);
        if prefix.len() < PROBE_BYTES {
            let remaining = PROBE_BYTES - prefix.len();
            prefix.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        }
        update_tail(&mut tail, chunk);
    }
    if size == 0 {
        return Err(AssetError::UnsupportedContent);
    }
    Ok(CopiedObject {
        digest: hasher.finalize().to_vec(),
        size,
        prefix,
        tail,
    })
}

fn update_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.len() >= TAIL_BYTES {
        tail.clear();
        tail.extend_from_slice(&chunk[chunk.len() - TAIL_BYTES..]);
        return;
    }
    tail.extend_from_slice(chunk);
    if tail.len() > TAIL_BYTES {
        tail.drain(..tail.len() - TAIL_BYTES);
    }
}

fn hash_open_file<C>(
    mut file: File,
    is_cancelled: &mut C,
    max_bytes: u64,
) -> Result<(AssetHash, u64)>
where
    C: FnMut() -> bool,
{
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    loop {
        if is_cancelled() {
            return Err(AssetError::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .ok_or(AssetError::LimitExceeded {
                limit_name: "per-object quota",
                limit: max_bytes,
            })?;
        if size > max_bytes {
            return Err(AssetError::LimitExceeded {
                limit_name: "per-object quota",
                limit: max_bytes,
            });
        }
        hasher.update(&buffer[..read]);
    }
    Ok((AssetHash::from_digest(&hasher.finalize()), size))
}

fn reject_symlink_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) =>
        {
            Err(AssetError::UnsafeFilesystem {
                path: path.display().to_string(),
                reason: "symlinks and reparse points are not accepted".to_owned(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn relative_object_path(hash: &AssetHash) -> String {
    format!(
        "objects/{}/{}/{}",
        &hash.as_str()[..2],
        &hash.as_str()[2..4],
        hash.as_str()
    )
}

fn validate_request(request: &IngestRequest) -> Result<()> {
    if let Some(name) = request.source_name.as_ref()
        && (name.len() > MAX_SOURCE_NAME_BYTES || name.contains('\0'))
    {
        return Err(AssetError::InvalidInput {
            field: "source name",
            reason: "must be at most 4096 bytes and contain no NUL",
        });
    }
    if let Some(owner) = request.owner.as_ref() {
        validate_owner(owner)?;
    }
    Ok(())
}

fn validate_backup_session_id(value: &str) -> Result<()> {
    if value.len() != 32
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(AssetError::InvalidInput {
            field: "backup session id",
            reason: "must be exactly 32 lowercase hexadecimal characters",
        });
    }
    Ok(())
}

fn abandon_backup_snapshot_on_connection(
    connection: &mut Connection,
    session_id: &str,
) -> Result<u64> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let removed = transaction.execute(
        "DELETE FROM asset_refs WHERE owner_type = ?1 AND owner_id = ?2",
        params![BACKUP_OWNER_TYPE, session_id],
    )?;
    transaction.execute(
        "DELETE FROM asset_backup_sessions WHERE session_id = ?1",
        params![session_id],
    )?;
    transaction.commit()?;
    Ok(removed as u64)
}

fn cleanup_stale_backup_sessions_in_transaction(
    transaction: &Transaction<'_>,
    stale_before_ms: i64,
    limit: u16,
) -> Result<BackupSnapshotCleanup> {
    let sessions = {
        let mut statement = transaction.prepare(
            "SELECT session_id FROM asset_backup_sessions
             WHERE lease_updated_at_ms <= ?1
             ORDER BY lease_updated_at_ms, session_id
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![stale_before_ms, i64::from(limit)], |row| {
            row.get::<_, String>(0)
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        sessions
    };
    let mut released_pins = 0u64;
    for session_id in &sessions {
        let removed = transaction.execute(
            "DELETE FROM asset_refs WHERE owner_type = ?1 AND owner_id = ?2",
            params![BACKUP_OWNER_TYPE, session_id],
        )?;
        released_pins =
            released_pins
                .checked_add(removed as u64)
                .ok_or(AssetError::IncompatibleCatalog {
                    reason: "backup cleanup pin count overflowed",
                })?;
        transaction.execute(
            "DELETE FROM asset_backup_sessions WHERE session_id = ?1",
            params![session_id],
        )?;
    }
    Ok(BackupSnapshotCleanup {
        removed_sessions: sessions,
        released_pins,
    })
}

fn validate_page_size(limit: u16) -> Result<()> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(AssetError::InvalidInput {
            field: "page size",
            reason: "must be between 1 and 1000",
        });
    }
    Ok(())
}

fn validate_shard(shard: &str) -> Result<()> {
    if shard.len() != 4
        || !shard
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(AssetError::InvalidInput {
            field: "object shard",
            reason: "must be four lowercase hexadecimal characters",
        });
    }
    Ok(())
}

fn validate_internal_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 160
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
    {
        return Err(AssetError::InvalidInput {
            field: "internal filename",
            reason: "contains unsafe characters",
        });
    }
    Ok(())
}

fn validate_object_relative_path(value: &str) -> Result<()> {
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.len() != 4 || parts[0] != OBJECTS_DIRECTORY {
        return Err(AssetError::IncompatibleCatalog {
            reason: "quarantine intent has an invalid object locator",
        });
    }
    let shard = format!("{}{}", parts[1], parts[2]);
    validate_shard(&shard).map_err(|_| AssetError::IncompatibleCatalog {
        reason: "quarantine intent has an invalid object shard",
    })?;
    validate_internal_name(parts[3]).map_err(|_| AssetError::IncompatibleCatalog {
        reason: "quarantine intent has an invalid object filename",
    })
}

#[cfg(test)]
std::thread_local! {
    static STAGING_CLEANUP_FAILPOINT: std::cell::RefCell<Option<&'static str>> = const {
        std::cell::RefCell::new(None)
    };
    static QUARANTINE_FAILPOINT: std::cell::RefCell<Option<&'static str>> = const {
        std::cell::RefCell::new(None)
    };
    static ROLLBACK_FAILPOINT: std::cell::RefCell<Option<&'static str>> = const {
        std::cell::RefCell::new(None)
    };
    static INGEST_AFTER_PREPARE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn ingest_after_prepare_hook() {
    INGEST_AFTER_PREPARE_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn ingest_after_prepare_hook() {}

#[cfg(test)]
fn staging_cleanup_failpoint(stage: &'static str) -> Result<()> {
    let should_fail = STAGING_CLEANUP_FAILPOINT.with(|value| {
        let mut value = value.borrow_mut();
        if value.as_ref() == Some(&stage) {
            value.take();
            true
        } else {
            false
        }
    });
    if should_fail {
        return Err(AssetError::Io(std::io::Error::other(format!(
            "injected staging cleanup failure at {stage}"
        ))));
    }
    Ok(())
}

#[cfg(not(test))]
fn staging_cleanup_failpoint(_stage: &'static str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
fn quarantine_failpoint(stage: &'static str) -> Result<()> {
    let should_fail = QUARANTINE_FAILPOINT.with(|value| {
        let mut value = value.borrow_mut();
        if value.as_ref() == Some(&stage) {
            value.take();
            true
        } else {
            false
        }
    });
    if should_fail {
        return Err(AssetError::Io(std::io::Error::other(format!(
            "injected quarantine failure at {stage}"
        ))));
    }
    Ok(())
}

#[cfg(not(test))]
fn quarantine_failpoint(_stage: &'static str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
fn rollback_failpoint(stage: &'static str) -> Result<()> {
    let should_fail = ROLLBACK_FAILPOINT.with(|value| {
        let mut value = value.borrow_mut();
        if value.as_ref() == Some(&stage) {
            value.take();
            true
        } else {
            false
        }
    });
    if should_fail {
        return Err(AssetError::Io(std::io::Error::other(format!(
            "injected rollback failure at {stage}"
        ))));
    }
    Ok(())
}

#[cfg(not(test))]
fn rollback_failpoint(_stage: &'static str) -> Result<()> {
    Ok(())
}

fn encode_size(size: u64) -> Result<i64> {
    i64::try_from(size).map_err(|_| AssetError::InvalidInput {
        field: "asset size",
        reason: "must fit in SQLite's signed 64-bit INTEGER range",
    })
}

fn now_ms() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AssetError::Io(std::io::Error::other(error)))?;
    i64::try_from(elapsed.as_millis()).map_err(|_| AssetError::InvalidInput {
        field: "system time",
        reason: "milliseconds since epoch do not fit in signed 64-bit integer",
    })
}

#[cfg(test)]
mod durability_tests {
    use std::{
        fs,
        io::{Cursor, Read},
        sync::mpsc,
        thread,
    };

    use rusqlite::params;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::*;
    use crate::AssetMime;

    fn limits() -> AssetLimits {
        AssetLimits::new(2 * 1024 * 1024, 32 * 1024 * 1024).expect("limits")
    }

    fn png() -> Vec<u8> {
        let mut bytes = vec![0u8; 33];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[8..12].copy_from_slice(&13u32.to_be_bytes());
        bytes[12..16].copy_from_slice(b"IHDR");
        bytes[16..20].copy_from_slice(&2u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&2u32.to_be_bytes());
        bytes
    }

    fn intent_count(root: &Path) -> i64 {
        Connection::open(root.join(catalog::CATALOG_FILE_NAME))
            .expect("catalog")
            .query_row("SELECT count(*) FROM asset_quarantine_intents", [], |row| {
                row.get(0)
            })
            .expect("intent count")
    }

    #[test]
    fn staging_row_is_not_deleted_until_unlink_and_directory_sync_succeed() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("assets");
        let store = AssetStore::open(&root, limits()).expect("store");
        STAGING_CLEANUP_FAILPOINT.with(|value| *value.borrow_mut() = Some("before_record_delete"));

        let mut source = Cursor::new(png());
        assert!(matches!(
            store.ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png)),
            Err(AssetError::Io(_))
        ));
        assert_eq!(store.stats().expect("stats").staging_count, 1);
        assert_eq!(
            fs::read_dir(root.join(STAGING_DIRECTORY))
                .expect("staging directory")
                .count(),
            0,
            "the file is durably removed before its recovery row"
        );

        store
            .cleanup_staging_page(None, 10, i64::MAX)
            .expect("retry catalog-driven cleanup");
        assert_eq!(store.stats().expect("stats").staging_count, 0);
    }

    #[test]
    fn temporary_owner_rollback_retries_after_unlink_before_catalog_commit() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("assets");
        let store = AssetStore::open(&root, limits()).expect("store");
        let session =
            AssetOwner::new("lorepia-import-session", "crash-window").expect("session owner");
        let mut source = Cursor::new(png());
        let object = store
            .ingest_uncancelled(
                &mut source,
                IngestRequest::new(AssetMime::Png).with_owner(session.clone()),
            )
            .expect("session ingest")
            .object;
        ROLLBACK_FAILPOINT.with(|value| *value.borrow_mut() = Some("after_object_unlink"));

        assert!(matches!(
            store.rollback_temporary_owner(&session),
            Err(AssetError::Io(_))
        ));
        let interrupted = store.stats().expect("interrupted stats");
        assert_eq!(interrupted.object_count, 1);
        assert_eq!(interrupted.reference_count, 1);
        assert_eq!(interrupted.active_bytes, object.size);
        drop(store);

        let recovered = AssetStore::open(&root, limits()).expect("reopen store");
        assert_eq!(
            recovered
                .recover_temporary_owner_type("lorepia-import-session")
                .expect("retry rollback"),
            1
        );
        let stats = recovered.stats().expect("recovered stats");
        assert_eq!(stats.object_count, 0);
        assert_eq!(stats.reference_count, 0);
        assert_eq!(stats.active_bytes, 0);
    }

    #[test]
    fn concurrent_rollback_after_dedupe_probe_republishes_before_catalog_commit() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("assets");
        let rollback_store = AssetStore::open(&root, limits()).expect("rollback store");
        let ingest_store = AssetStore::open(&root, limits()).expect("independent ingest store");
        let temporary_owner =
            AssetOwner::new("lorepia-import-session", "rollback-race").expect("session owner");
        rollback_store
            .begin_temporary_owner_session(&temporary_owner)
            .expect("live session");
        let bytes = png();
        let mut initial_source = Cursor::new(bytes.clone());
        let initial = rollback_store
            .ingest_uncancelled(
                &mut initial_source,
                IngestRequest::new(AssetMime::Png).with_owner(temporary_owner.clone()),
            )
            .expect("initial session object");

        let final_owner = AssetOwner::new("character", "dedupe-race").expect("final owner");
        let (prepared_tx, prepared_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let ingest_bytes = bytes.clone();
        let ingest = thread::spawn(move || {
            INGEST_AFTER_PREPARE_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    prepared_tx.send(()).expect("announce prepared destination");
                    release_rx.recv().expect("release ingest");
                }));
            });
            let mut source = Cursor::new(ingest_bytes);
            ingest_store.ingest_uncancelled(
                &mut source,
                IngestRequest::new(AssetMime::Png).with_owner(final_owner),
            )
        });
        prepared_rx.recv().expect("dedupe probe reached barrier");

        assert_eq!(
            rollback_store
                .rollback_temporary_owner(&temporary_owner)
                .expect("concurrent rollback"),
            1
        );
        release_tx.send(()).expect("release ingest barrier");
        let outcome = ingest
            .join()
            .expect("ingest thread")
            .expect("republication");
        assert!(!outcome.deduplicated);
        assert_eq!(outcome.object.hash, initial.object.hash);

        let mut reader = rollback_store
            .open_object(&outcome.object.hash)
            .expect("active object has a file");
        let mut exported = Vec::new();
        reader.read_to_end(&mut exported).expect("read object");
        assert_eq!(exported, bytes);
        let stats = rollback_store.stats().expect("consistent stats");
        assert_eq!(stats.object_count, 1);
        assert_eq!(stats.reference_count, 1);
        assert_eq!(stats.missing_count, 0);
    }

    #[test]
    fn quarantine_move_intents_recover_every_durable_crash_window() {
        for stage in [
            "after_move_intent",
            "after_filesystem_move",
            "before_move_finalize",
        ] {
            let temp = TempDir::new().expect("tempdir");
            let root = temp.path().join("assets");
            let store = AssetStore::open(&root, limits()).expect("store");
            let bytes = png();
            let hash = hex::encode(Sha256::digest(&bytes));
            let final_path = root
                .join(OBJECTS_DIRECTORY)
                .join(&hash[..2])
                .join(&hash[2..4])
                .join(&hash);
            fs::create_dir_all(final_path.parent().expect("object parent")).expect("shards");
            fs::write(&final_path, b"corrupt").expect("corrupt object");
            QUARANTINE_FAILPOINT.with(|value| *value.borrow_mut() = Some(stage));

            let mut source = Cursor::new(bytes);
            assert!(matches!(
                store.ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png)),
                Err(AssetError::Io(_))
            ));
            assert_eq!(intent_count(&root), 1, "stage {stage}");
            drop(store);

            let recovered = AssetStore::open(&root, limits()).expect("recover move intent");
            assert_eq!(intent_count(&root), 0, "stage {stage}");
            assert_eq!(
                recovered.stats().expect("stats").quarantined_count,
                1,
                "stage {stage}"
            );
            assert!(!final_path.exists(), "stage {stage}");
        }
    }

    #[test]
    fn quarantine_purge_intents_recover_every_durable_crash_window() {
        for stage in [
            "after_purge_intent",
            "after_quarantine_unlink",
            "before_purge_finalize",
        ] {
            let temp = TempDir::new().expect("tempdir");
            let root = temp.path().join("assets");
            let store = AssetStore::open(&root, limits()).expect("store");
            let name = "unknown-deadbeef.quarantine";
            let mut file = store
                .inner
                .quarantine_directory
                .create_new_file(OsStr::new(name))
                .expect("quarantine file");
            file.write_all(b"quarantine").expect("write quarantine");
            file.sync_all().expect("sync quarantine");
            drop(file);
            store.inner.quarantine_directory.sync().expect("sync dir");
            store
                .connection()
                .expect("catalog")
                .execute(
                    "INSERT INTO asset_quarantine(name, original_hash, reason, created_at_ms)
                     VALUES (?1, NULL, 'test', 0)",
                    params![name],
                )
                .expect("quarantine row");
            QUARANTINE_FAILPOINT.with(|value| *value.borrow_mut() = Some(stage));

            assert!(matches!(
                store.purge_quarantine_page(None, 10, i64::MAX),
                Err(AssetError::Io(_))
            ));
            assert_eq!(intent_count(&root), 1, "stage {stage}");
            drop(store);

            let recovered = AssetStore::open(&root, limits()).expect("recover purge intent");
            assert_eq!(intent_count(&root), 0, "stage {stage}");
            assert_eq!(
                recovered.stats().expect("stats").quarantined_count,
                0,
                "stage {stage}"
            );
            assert!(!root.join(QUARANTINE_DIRECTORY).join(name).exists());
        }
    }
}
