use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

use lorepia_assets::{AssetLimits, AssetMime, AssetStore, IngestRequest};
use lorepia_backup::{
    BackupError, BackupManifest, Control, ExportOptions, ExportRequest, MAX_BACKUP_JOURNAL_BYTES,
    MAX_BACKUP_MANIFEST_BYTES, Phase, RestoreOptions, RestorePolicy, RestoreRequest,
    SecretSentinel, SpaceProbe, UnknownSpacePolicy, abandon_export, export_backup,
    partial_path_for, restore_backup, restore_journal_path_for,
};
use lorepia_storage::{CharacterId, CreateChat, Store, TimestampMillis};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

#[derive(Clone, Copy)]
struct FixedSpace(Option<u64>);

impl SpaceProbe for FixedSpace {
    fn available_bytes(&self, _path: &Path) -> std::io::Result<Option<u64>> {
        Ok(self.0)
    }
}

struct SequenceSpace {
    calls: AtomicUsize,
    values: Vec<Option<u64>>,
}

impl SequenceSpace {
    fn new(values: Vec<Option<u64>>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            values,
        }
    }
}

impl SpaceProbe for SequenceSpace {
    fn available_bytes(&self, _path: &Path) -> std::io::Result<Option<u64>> {
        let index = self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.values[index.min(self.values.len() - 1)])
    }
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    product: Store,
    assets: AssetStore,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let product = Store::open(root.join("source.sqlite3")).unwrap();
        let limits = AssetLimits::new(2 * 1024 * 1024, 64 * 1024 * 1024).unwrap();
        let assets = AssetStore::open(root.join("source-assets"), limits).unwrap();
        Self {
            _temp: temp,
            root,
            product,
            assets,
        }
    }

    fn add_chat(&self, title: &str, timestamp: i64) {
        self.product
            .create_chat(CreateChat {
                character_id: CharacterId::parse("backup-test-character").unwrap(),
                title: title.to_owned(),
                at_ms: TimestampMillis::new(timestamp).unwrap(),
            })
            .unwrap();
    }

    fn add_asset(&self, padding: usize) -> String {
        let mut input = Cursor::new(png(2, 3, padding));
        self.assets
            .ingest_uncancelled(&mut input, IngestRequest::new(AssetMime::Png))
            .unwrap()
            .object
            .hash
            .to_string()
    }

    fn export(&self, name: &str) -> PathBuf {
        let destination = self.root.join(name);
        export_backup(
            ExportRequest {
                product: &self.product,
                assets: &self.assets,
                destination: &destination,
                secret_sentinels: &[],
                space_probe: &FixedSpace(Some(u64::MAX)),
            },
            ExportOptions::default(),
            |_| Control::Continue,
        )
        .unwrap();
        destination
    }
}

fn png(width: u32, height: u32, padding: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; 33 + padding];
    bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    bytes[8..12].copy_from_slice(&13u32.to_be_bytes());
    bytes[12..16].copy_from_slice(b"IHDR");
    bytes[16..20].copy_from_slice(&width.to_be_bytes());
    bytes[20..24].copy_from_slice(&height.to_be_bytes());
    bytes
}

fn restore(package: &Path, destination: &Path, policy: RestorePolicy) {
    restore_backup(
        RestoreRequest {
            package,
            destination,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions {
            policy,
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        },
        |_| Control::Continue,
    )
    .unwrap();
}

#[test]
fn empty_database_roundtrip_is_integrity_checked() {
    let fixture = Fixture::new();
    let package = fixture.export("empty.lorepia-backup");
    assert!(package.is_dir());
    assert!(!package.extension().is_some_and(|value| value == "zip"));
    let manifest: BackupManifest =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    assert!(manifest.compatibility_receipts.iter().any(|receipt| {
        receipt.check_id == "BACKUP-007" && receipt.disposition == "not_applicable_by_design"
    }));

    let restored = fixture.root.join("restored");
    restore(&package, &restored, RestorePolicy::FailIfPresent);
    let opened = Store::open(restored.join("product.sqlite3")).unwrap();
    assert!(opened.list_chats(10, None).unwrap().chats.is_empty());
    let assets = AssetStore::open(
        restored.join("assets"),
        AssetLimits::new(2 * 1024 * 1024, 64 * 1024 * 1024).unwrap(),
    )
    .unwrap();
    assert_eq!(assets.stats().unwrap().object_count, 0);
}

#[test]
fn cancel_is_resumable_from_verified_hash_cursor() {
    let fixture = Fixture::new();
    fixture.add_asset(10);
    fixture.add_asset(11);
    let destination = fixture.root.join("resume.lorepia-backup");
    let mut cancelled = false;
    let result = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |progress| {
            if !cancelled && progress.verified_objects == 1 {
                cancelled = true;
                Control::Cancel
            } else {
                Control::Continue
            }
        },
    );
    assert!(matches!(result, Err(BackupError::Cancelled)));
    let partial = partial_path_for(&destination).unwrap();
    assert!(partial.join("progress.json").is_file());
    let progress: lorepia_backup::BackupProgress =
        serde_json::from_slice(&fs::read(partial.join("progress.json")).unwrap()).unwrap();
    assert_eq!(
        fixture
            .assets
            .renew_backup_snapshot(&progress.session_id)
            .unwrap()
            .unwrap()
            .pinned_objects,
        2
    );

    let report = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(report.resumed);
    assert_eq!(report.object_count, 2);
    assert!(
        fixture
            .assets
            .renew_backup_snapshot(&report.session_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn cancelled_export_can_be_explicitly_abandoned_without_leaking_pins() {
    let fixture = Fixture::new();
    fixture.add_asset(10);
    let destination = fixture.root.join("abandon.lorepia-backup");
    let error = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |progress| {
            if progress.phase == Phase::AssetCatalogSnapshot {
                Control::Cancel
            } else {
                Control::Continue
            }
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Cancelled));
    let partial = partial_path_for(&destination).unwrap();
    let progress: lorepia_backup::BackupProgress =
        serde_json::from_slice(&fs::read(partial.join("progress.json")).unwrap()).unwrap();

    assert!(abandon_export(&fixture.assets, &destination).unwrap());
    assert!(!partial.exists());
    assert!(
        fixture
            .assets
            .renew_backup_snapshot(&progress.session_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.assets.stats().unwrap().reference_count, 0);
    assert!(!abandon_export(&fixture.assets, &destination).unwrap());
}

#[test]
fn expired_cancelled_export_restarts_from_a_fresh_snapshot() {
    let fixture = Fixture::new();
    fixture.add_asset(10);
    let destination = fixture.root.join("expired.lorepia-backup");
    let error = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |progress| {
            if progress.phase == Phase::AssetCatalogSnapshot {
                Control::Cancel
            } else {
                Control::Continue
            }
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Cancelled));
    let partial = partial_path_for(&destination).unwrap();
    let progress: lorepia_backup::BackupProgress =
        serde_json::from_slice(&fs::read(partial.join("progress.json")).unwrap()).unwrap();
    let connection =
        rusqlite::Connection::open(fixture.root.join("source-assets/assets.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE asset_backup_sessions SET created_at_ms = 0, lease_updated_at_ms = 0
             WHERE session_id = ?1",
            [&progress.session_id],
        )
        .unwrap();
    drop(connection);

    let report = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(!report.resumed);
    assert_ne!(report.session_id, progress.session_id);
}

#[test]
fn asset_add_after_catalog_snapshot_is_excluded_by_contract() {
    let fixture = Fixture::new();
    let first = fixture.add_asset(1);
    let destination = fixture.root.join("point-in-time.lorepia-backup");
    let mut second = None;
    export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |progress| {
            if progress.phase == Phase::AssetCatalogSnapshot && second.is_none() {
                second = Some(fixture.add_asset(2));
                let sweep = fixture
                    .assets
                    .mark_sweep_page(None, 10, i64::MAX, || false)
                    .unwrap();
                assert!(
                    !sweep.removed.iter().any(|hash| hash.as_str() == first),
                    "snapshot pin must retain the snapshotted object"
                );
            }
            Control::Continue
        },
    )
    .unwrap();
    let manifest: BackupManifest =
        serde_json::from_slice(&fs::read(destination.join("manifest.json")).unwrap()).unwrap();
    let object_hashes = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == "asset_object")
        .map(|entry| entry.sha256.as_str())
        .collect::<Vec<_>>();
    assert_eq!(object_hashes, vec![first.as_str()]);
    assert_ne!(second.unwrap(), first);
}

#[test]
fn concurrent_product_writes_do_not_corrupt_online_snapshot() {
    let fixture = Fixture::new();
    fixture.add_chat("before", 1);
    let running = Arc::new(AtomicBool::new(true));
    let writer_store = fixture.product.clone();
    let writer_running = Arc::clone(&running);
    let writer = thread::spawn(move || {
        let mut timestamp = 2i64;
        while writer_running.load(Ordering::Acquire) && timestamp < 500 {
            let _ = writer_store.create_chat(CreateChat {
                character_id: CharacterId::parse("backup-test-character").unwrap(),
                title: format!("concurrent-{timestamp}"),
                at_ms: TimestampMillis::new(timestamp).unwrap(),
            });
            timestamp += 1;
        }
    });
    let package = fixture.export("concurrent.lorepia-backup");
    running.store(false, Ordering::Release);
    writer.join().unwrap();
    let restored = fixture.root.join("concurrent-restored");
    restore(&package, &restored, RestorePolicy::FailIfPresent);
    let restored_store = Store::open(restored.join("product.sqlite3")).unwrap();
    assert!(
        !restored_store
            .list_chats(100, None)
            .unwrap()
            .chats
            .is_empty()
    );
}

#[test]
fn injected_space_failures_are_fail_closed_and_unknown_is_explicit() {
    let fixture = Fixture::new();
    let destination = fixture.root.join("no-space.lorepia-backup");
    let error = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(0)),
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::InsufficientSpace { .. }));
    assert!(!partial_path_for(&destination).unwrap().exists());
    let retry = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(!retry.resumed);

    let unknown = fixture.root.join("unknown-space.lorepia-backup");
    let error = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &unknown,
            secret_sentinels: &[],
            space_probe: &FixedSpace(None),
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::FreeSpaceUnknown));
    assert!(!partial_path_for(&unknown).unwrap().exists());
}

#[test]
fn empty_pre_journal_orphans_are_recovered_safely() {
    let fixture = Fixture::new();
    let destination = fixture.root.join("empty-orphan.lorepia-backup");
    let partial = partial_path_for(&destination).unwrap();
    fs::create_dir(&partial).unwrap();
    let report = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(!report.resumed);

    let target = fixture.root.join("empty-restore-orphan");
    let staging = fixture.root.join(".empty-restore-orphan.restore-partial");
    fs::create_dir(&staging).unwrap();
    let report = restore_backup(
        RestoreRequest {
            package: &destination,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(!report.resumed);
    assert!(target.join("product.sqlite3").is_file());
}

#[test]
fn oversized_progress_journal_is_rejected_without_bulk_allocation() {
    let fixture = Fixture::new();
    let destination = fixture.root.join("journal-limit.lorepia-backup");
    let partial = partial_path_for(&destination).unwrap();
    fs::create_dir(&partial).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(partial.join("progress.json"))
        .unwrap()
        .set_len(MAX_BACKUP_JOURNAL_BYTES + 1)
        .unwrap();
    let error = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::JournalConflict));

    let package = fixture.export("restore-journal-source.lorepia-backup");
    let target = fixture.root.join("restore-journal-target");
    let restore_journal = restore_journal_path_for(&target).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&restore_journal)
        .unwrap()
        .set_len(MAX_BACKUP_JOURNAL_BYTES + 1)
        .unwrap();
    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::JournalConflict));
}

#[test]
fn pinned_catalog_is_remeasured_before_object_copy() {
    let fixture = Fixture::new();
    let hash = fixture.add_asset(128 * 1024);
    let destination = fixture.root.join("remeasure.lorepia-backup");
    let space = SequenceSpace::new(vec![Some(u64::MAX), Some(0)]);
    let error = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[],
            space_probe: &space,
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::InsufficientSpace { .. }));
    let copied = partial_path_for(&destination)
        .unwrap()
        .join("data/assets/objects")
        .join(&hash[..2])
        .join(&hash[2..4])
        .join(hash);
    assert!(!copied.exists());
}

#[test]
fn truncated_object_and_manifest_hash_mismatch_are_rejected() {
    let fixture = Fixture::new();
    let hash = fixture.add_asset(5);
    let package = fixture.export("corrupt.lorepia-backup");
    let object = package
        .join("data/assets/objects")
        .join(&hash[..2])
        .join(&hash[2..4])
        .join(&hash);
    let mut bytes = fs::read(&object).unwrap();
    bytes.pop();
    fs::write(&object, bytes).unwrap();
    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &fixture.root.join("bad-restore"),
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::EntryMismatch { .. }));

    let clean = fixture.export("bad-manifest.lorepia-backup");
    fs::write(
        clean.join("manifest.sha256"),
        format!("{}\n", "0".repeat(64)),
    )
    .unwrap();
    let error = restore_backup(
        RestoreRequest {
            package: &clean,
            destination: &fixture.root.join("bad-manifest-restore"),
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::EntryMismatch { .. }));
}

#[test]
fn restore_requires_all_fixed_manifest_entries() {
    let fixture = Fixture::new();
    let package = fixture.export("missing-receipt.lorepia-backup");
    let path = package.join("manifest.json");
    let mut manifest: BackupManifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let index = manifest
        .entries
        .iter()
        .position(|entry| entry.path == "progress.json")
        .unwrap();
    let removed = manifest.entries.remove(index);
    manifest.total_entry_bytes -= removed.size;
    fs::remove_file(package.join("progress.json")).unwrap();
    write_manifest(&package, &manifest);

    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &fixture.root.join("missing-receipt-restore"),
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::InvalidManifest { .. }));
}

#[test]
fn oversized_actual_manifest_is_rejected_before_it_is_read_into_memory() {
    let fixture = Fixture::new();
    let package = fixture.export("oversized-manifest.lorepia-backup");
    fs::OpenOptions::new()
        .write(true)
        .open(package.join("manifest.json"))
        .unwrap()
        .set_len(MAX_BACKUP_MANIFEST_BYTES + 1)
        .unwrap();

    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &fixture.root.join("oversized-manifest-restore"),
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::InvalidManifest { .. }));
}

#[test]
fn future_format_is_rejected_and_v0_is_migrated_in_memory() {
    let fixture = Fixture::new();
    let future = fixture.export("future.lorepia-backup");
    rewrite_manifest_version(&future, 2);
    let error = restore_backup(
        RestoreRequest {
            package: &future,
            destination: &fixture.root.join("future-restore"),
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::FutureVersion { found: 2, .. }));

    let legacy = fixture.export("legacy.lorepia-backup");
    rewrite_manifest_version(&legacy, 0);
    restore(
        &legacy,
        &fixture.root.join("legacy-restored"),
        RestorePolicy::FailIfPresent,
    );
}

#[test]
fn existing_data_fails_by_default_and_replace_uses_atomic_old_directory() {
    let fixture = Fixture::new();
    let package = fixture.export("replace.lorepia-backup");
    let target = fixture.root.join("existing");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("old.txt"), b"old").unwrap();
    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::ExistingData(_)));
    assert_eq!(fs::read(target.join("old.txt")).unwrap(), b"old");

    restore(&package, &target, RestorePolicy::Replace);
    assert!(target.join("product.sqlite3").is_file());
    assert!(!target.join("old.txt").exists());
}

#[test]
fn restore_cancel_at_every_publish_phase_resumes_or_rolls_forward() {
    for (index, phase) in [
        Phase::Prepared,
        Phase::Copied,
        Phase::Verified,
        Phase::OldMoved,
        Phase::NewPublished,
        Phase::Complete,
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = Fixture::new();
        fixture.add_asset(index);
        let package = fixture.export("phase.lorepia-backup");
        let target = fixture.root.join("phase-target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("old"), b"rollback source").unwrap();
        let mut stopped = false;
        let error = restore_backup(
            RestoreRequest {
                package: &package,
                destination: &target,
                space_probe: &FixedSpace(Some(u64::MAX)),
                secret_sentinels: &[],
            },
            RestoreOptions {
                policy: RestorePolicy::Replace,
                unknown_space_policy: UnknownSpacePolicy::FailClosed,
            },
            |progress| {
                if !stopped && progress.phase == phase {
                    stopped = true;
                    Control::Cancel
                } else {
                    Control::Continue
                }
            },
        )
        .unwrap_err();
        assert!(matches!(error, BackupError::Cancelled));
        let report = restore_backup(
            RestoreRequest {
                package: &package,
                destination: &target,
                space_probe: &FixedSpace(Some(u64::MAX)),
                secret_sentinels: &[],
            },
            RestoreOptions {
                policy: RestorePolicy::Replace,
                unknown_space_policy: UnknownSpacePolicy::FailClosed,
            },
            |_| Control::Continue,
        )
        .unwrap();
        assert!(report.resumed);
        assert!(report.replaced_existing);
        assert!(target.join("product.sqlite3").is_file());
    }
}

#[test]
fn restore_recovers_rename_before_journal_crash_windows() {
    let fixture = Fixture::new();
    let package = fixture.export("rename-window.lorepia-backup");
    let target = fixture.root.join("rename-target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("old"), b"old").unwrap();

    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions {
            policy: RestorePolicy::Replace,
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        },
        |progress| {
            if progress.phase == Phase::Verified {
                Control::Cancel
            } else {
                Control::Continue
            }
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Cancelled));
    let old = fixture.root.join(".rename-target.restore-old");
    fs::rename(&target, &old).unwrap();
    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions {
            policy: RestorePolicy::Replace,
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        },
        |progress| {
            if progress.phase == Phase::OldMoved {
                Control::Cancel
            } else {
                Control::Continue
            }
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Cancelled));
    let staging = fixture.root.join(".rename-target.restore-partial");
    fs::rename(&staging, &target).unwrap();
    let report = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions {
            policy: RestorePolicy::Replace,
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        },
        |_| Control::Continue,
    )
    .unwrap();
    assert!(report.resumed);
    assert!(target.join("product.sqlite3").is_file());
}

#[test]
fn fresh_restore_recovers_on_both_sides_of_the_publish_rename() {
    let fixture = Fixture::new();
    let package = fixture.export("fresh-rename-window.lorepia-backup");

    let before_target = fixture.root.join("fresh-before-target");
    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &before_target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |progress| {
            if progress.phase == Phase::OldMoved {
                Control::Cancel
            } else {
                Control::Continue
            }
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Cancelled));
    assert!(!before_target.exists());
    assert!(
        fixture
            .root
            .join(".fresh-before-target.restore-partial")
            .is_dir()
    );
    assert!(
        !fixture
            .root
            .join(".fresh-before-target.restore-old")
            .exists()
    );
    let report = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &before_target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(report.resumed);
    assert!(!report.replaced_existing);
    assert!(before_target.join("product.sqlite3").is_file());

    let after_target = fixture.root.join("fresh-after-target");
    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &after_target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |progress| {
            if progress.phase == Phase::OldMoved {
                Control::Cancel
            } else {
                Control::Continue
            }
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Cancelled));
    let staging = fixture.root.join(".fresh-after-target.restore-partial");
    assert!(staging.is_dir());
    assert!(!after_target.exists());
    assert!(
        !fixture
            .root
            .join(".fresh-after-target.restore-old")
            .exists()
    );
    fs::rename(&staging, &after_target).unwrap();
    assert!(after_target.is_dir());
    assert!(!staging.exists());
    assert!(
        !fixture
            .root
            .join(".fresh-after-target.restore-old")
            .exists()
    );
    let report = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &after_target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(report.resumed);
    assert!(!report.replaced_existing);
    assert!(after_target.join("product.sqlite3").is_file());
    assert!(
        !fixture
            .root
            .join(".fresh-after-target.restore-old")
            .exists()
    );
}

#[test]
fn cancellation_during_post_publish_hash_rolls_back_to_resumable_staging() {
    let fixture = Fixture::new();
    fixture.add_asset(128 * 1024);
    let package = fixture.export("post-publish-cancel.lorepia-backup");
    let target = fixture.root.join("post-publish-target");
    let mut published_callbacks = 0usize;
    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |progress| {
            if progress.phase == Phase::NewPublished {
                published_callbacks += 1;
                if published_callbacks >= 2 {
                    return Control::Cancel;
                }
            }
            Control::Continue
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Cancelled));
    assert!(!target.exists());
    assert!(
        fixture
            .root
            .join(".post-publish-target.restore-partial")
            .is_dir()
    );

    let report = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions::default(),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(report.resumed);
    assert!(target.join("product.sqlite3").is_file());
}

#[test]
fn missing_package_rolls_old_data_back_before_returning_error() {
    let fixture = Fixture::new();
    let package = fixture.export("detachable.lorepia-backup");
    let target = fixture.root.join("detachable-target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("old.txt"), b"old-data").unwrap();

    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions {
            policy: RestorePolicy::Replace,
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        },
        |progress| {
            if progress.phase == Phase::OldMoved {
                Control::Cancel
            } else {
                Control::Continue
            }
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Cancelled));
    assert!(!target.exists());
    assert!(fixture.root.join(".detachable-target.restore-old").is_dir());

    let detached = fixture.root.join("detached-package");
    fs::rename(&package, &detached).unwrap();
    let error = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions {
            policy: RestorePolicy::Replace,
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        },
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::Io(_)));
    assert_eq!(fs::read(target.join("old.txt")).unwrap(), b"old-data");
    assert!(!fixture.root.join(".detachable-target.restore-old").exists());

    fs::rename(&detached, &package).unwrap();
    let report = restore_backup(
        RestoreRequest {
            package: &package,
            destination: &target,
            space_probe: &FixedSpace(Some(u64::MAX)),
            secret_sentinels: &[],
        },
        RestoreOptions {
            policy: RestorePolicy::Replace,
            unknown_space_policy: UnknownSpacePolicy::FailClosed,
        },
        |_| Control::Continue,
    )
    .unwrap();
    assert!(report.resumed);
    assert!(report.replaced_existing);
    assert!(target.join("product.sqlite3").is_file());
}

#[test]
#[ignore = "provision a real 10 GiB SQLite database and 100 GiB asset store in CI load infrastructure"]
fn real_10gib_database_100gib_assets_load_gate() {
    panic!("external load fixture is intentionally not synthesized in a unit test");
}

#[test]
fn secret_sentinel_blocks_export_without_logging_secret() {
    let fixture = Fixture::new();
    let secret = "sk-test-super-secret";
    fixture.add_chat(secret, 1);
    let sentinel = SecretSentinel::new(secret.as_bytes().to_vec()).unwrap();
    let destination = fixture.root.join("secret.lorepia-backup");
    let error = export_backup(
        ExportRequest {
            product: &fixture.product,
            assets: &fixture.assets,
            destination: &destination,
            secret_sentinels: &[sentinel],
            space_probe: &FixedSpace(Some(u64::MAX)),
        },
        ExportOptions::default(),
        |_| Control::Continue,
    )
    .unwrap_err();
    assert!(matches!(error, BackupError::SecretFound { .. }));
    assert!(!error.to_string().contains(secret));
    assert!(!partial_path_for(&destination).unwrap().exists());
    assert_eq!(fixture.assets.stats().unwrap().reference_count, 0);
}

fn rewrite_manifest_version(package: &Path, version: u32) {
    let path = package.join("manifest.json");
    let mut manifest: BackupManifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest.format_version = version;
    write_manifest(package, &manifest);
}

fn write_manifest(package: &Path, manifest: &BackupManifest) {
    let path = package.join("manifest.json");
    let mut bytes = serde_json::to_vec(&manifest).unwrap();
    bytes.push(b'\n');
    fs::write(&path, &bytes).unwrap();
    let hash = hex::encode(Sha256::digest(&bytes));
    fs::write(package.join("manifest.sha256"), format!("{hash}\n")).unwrap();
}
