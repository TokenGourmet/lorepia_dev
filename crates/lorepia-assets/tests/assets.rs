use std::{
    fs,
    io::{Cursor, Error, Read},
    path::Path,
    str::FromStr,
    sync::mpsc,
    thread,
    time::Instant,
};

use lorepia_assets::{
    AssetError, AssetHash, AssetLimits, AssetMime, AssetOwner, AssetReference, AssetState,
    AssetStore, IngestRequest, ReconcileFindingKind,
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn limits() -> AssetLimits {
    AssetLimits::new(2 * 1024 * 1024, 32 * 1024 * 1024).expect("valid limits")
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

fn jpeg() -> Vec<u8> {
    vec![
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x02, 0x00, 0x03, 0x01, 0x01, 0x11, 0x00,
        0xff, 0xd9,
    ]
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn open_store(temp: &TempDir) -> AssetStore {
    AssetStore::open(temp.path().join("assets"), limits()).expect("open asset store")
}

#[test]
fn duplicate_content_is_deduplicated_and_source_names_never_select_paths() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let bytes = png(2, 3, 0);
    let malicious_names = [
        "../../escape.png",
        "..\\..\\escape.png",
        "CON.png",
        "e\u{301}.png",
    ];

    let mut first = Cursor::new(bytes.clone());
    let first = store
        .ingest_uncancelled(
            &mut first,
            IngestRequest::new(AssetMime::Png)
                .with_source_name(malicious_names[0])
                .expect("source label"),
        )
        .expect("first ingest");
    assert!(!first.deduplicated);

    for name in &malicious_names[1..] {
        let mut duplicate = Cursor::new(bytes.clone());
        let outcome = store
            .ingest_uncancelled(
                &mut duplicate,
                IngestRequest::new(AssetMime::Png)
                    .with_source_name(*name)
                    .expect("source label"),
            )
            .expect("deduplicated ingest");
        assert!(outcome.deduplicated);
        assert_eq!(outcome.object.hash, first.object.hash);
    }

    let stats = store.stats().expect("stats");
    assert_eq!(stats.object_count, 1);
    assert_eq!(stats.active_bytes, bytes.len() as u64);
    assert_eq!(store.verify_catalog_ledger().expect("ledger"), stats);
    assert_eq!(
        first.object.relative_path,
        format!(
            "objects/{}/{}/{}",
            &first.object.hash.as_str()[..2],
            &first.object.hash.as_str()[2..4],
            first.object.hash
        )
    );
    assert!(!temp.path().join("escape.png").exists());
}

#[test]
fn declared_mime_must_match_valid_magic() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let mut forged = Cursor::new(jpeg());
    assert!(matches!(
        store.ingest_uncancelled(&mut forged, IngestRequest::new(AssetMime::Png)),
        Err(AssetError::MimeMismatch { .. })
    ));

    let mut malformed = Cursor::new(b"not an image despite .png".to_vec());
    assert!(matches!(
        store.ingest_uncancelled(
            &mut malformed,
            IngestRequest::new(AssetMime::Png)
                .with_source_name("looks-valid.png")
                .expect("source label")
        ),
        Err(AssetError::UnsupportedContent)
    ));
    assert_eq!(store.stats().expect("stats").staging_count, 0);
}

#[test]
fn per_object_and_total_limits_fail_closed_without_staging_leaks() {
    let temp = TempDir::new().expect("tempdir");
    let store = AssetStore::open(
        temp.path().join("assets"),
        AssetLimits::new(33, 50).expect("limits"),
    )
    .expect("store");
    let mut first = Cursor::new(png(1, 1, 0));
    store
        .ingest_uncancelled(&mut first, IngestRequest::new(AssetMime::Png))
        .expect("first object");

    let mut second = Cursor::new(png(2, 2, 0));
    assert!(matches!(
        store.ingest_uncancelled(&mut second, IngestRequest::new(AssetMime::Png)),
        Err(AssetError::LimitExceeded {
            limit_name: "total asset quota",
            limit: 50
        })
    ));
    assert_eq!(store.stats().expect("stats").object_count, 1);
    assert_eq!(store.stats().expect("stats").staging_count, 0);

    let other = TempDir::new().expect("tempdir");
    let tiny = AssetStore::open(
        other.path().join("assets"),
        AssetLimits::new(32, 64).expect("limits"),
    )
    .expect("store");
    let mut too_large = Cursor::new(png(1, 1, 0));
    assert!(matches!(
        tiny.ingest_uncancelled(&mut too_large, IngestRequest::new(AssetMime::Png)),
        Err(AssetError::LimitExceeded {
            limit_name: "per-object quota",
            limit: 32
        })
    ));
    assert_eq!(tiny.stats().expect("stats").staging_count, 0);
}

struct FailingReader {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
}

impl Read for FailingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.reads > 0 {
            return Err(Error::other("injected content URI revocation"));
        }
        self.reads += 1;
        let limit = buffer.len().min(20);
        self.bytes.read(&mut buffer[..limit])
    }
}

struct BlockingReader {
    bytes: Cursor<Vec<u8>>,
    entered: Option<mpsc::Sender<()>>,
    release: mpsc::Receiver<()>,
}

impl Read for BlockingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(entered) = self.entered.take() {
            entered.send(()).expect("admission observer is alive");
            self.release.recv().expect("blocking reader is released");
        }
        self.bytes.read(buffer)
    }
}

#[test]
fn a_hundred_concurrent_import_attempts_are_rejected_by_bounded_admission() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first_store = store.clone();
    let first = thread::spawn(move || {
        let mut reader = BlockingReader {
            bytes: Cursor::new(png(1, 1, 0)),
            entered: Some(entered_tx),
            release: release_rx,
        };
        first_store.ingest_uncancelled(&mut reader, IngestRequest::new(AssetMime::Png))
    });
    entered_rx.recv().expect("first import holds admission");

    for attempt in 0..100 {
        let mut reader = Cursor::new(png(2, 2, attempt));
        assert!(matches!(
            store.ingest_uncancelled(&mut reader, IngestRequest::new(AssetMime::Png)),
            Err(AssetError::MutationBusy)
        ));
    }

    release_tx.send(()).expect("release first import");
    first
        .join()
        .expect("first import thread")
        .expect("first import");
    assert_eq!(store.stats().expect("stats").object_count, 1);
    assert_eq!(store.stats().expect("stats").staging_count, 0);
}

#[test]
fn cancellation_and_truncated_sources_remove_staging() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let mut large = Cursor::new(png(1, 1, 128 * 1024));
    let mut polls = 0;
    let error = store
        .ingest(&mut large, IngestRequest::new(AssetMime::Png), || {
            polls += 1;
            polls >= 2
        })
        .expect_err("cancelled");
    assert!(matches!(error, AssetError::Cancelled));
    assert_eq!(store.stats().expect("stats").staging_count, 0);

    let mut failing = FailingReader {
        bytes: Cursor::new(png(1, 1, 0)),
        reads: 0,
    };
    assert!(matches!(
        store.ingest_uncancelled(&mut failing, IngestRequest::new(AssetMime::Png)),
        Err(AssetError::Io(_))
    ));
    assert_eq!(store.stats().expect("stats").staging_count, 0);
}

#[test]
fn ref_batches_are_transactional_and_mark_sweep_only_removes_orphans() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let mut bytes = Cursor::new(png(1, 1, 0));
    let object = store
        .ingest_uncancelled(&mut bytes, IngestRequest::new(AssetMime::Png))
        .expect("ingest")
        .object;
    let owner = AssetOwner::new("character", "alice").expect("owner");
    let valid = AssetReference {
        owner: owner.clone(),
        hash: object.hash.clone(),
    };
    let missing = AssetReference {
        owner: owner.clone(),
        hash: AssetHash::parse("f".repeat(64)).expect("hash"),
    };
    assert!(matches!(
        store.add_refs(&[valid.clone(), missing]),
        Err(AssetError::NotFound { .. })
    ));
    assert_eq!(store.stats().expect("stats").reference_count, 0);

    store
        .add_refs(std::slice::from_ref(&valid))
        .expect("add ref");
    assert_eq!(store.stats().expect("stats").reference_count, 1);
    let page = store
        .mark_sweep_page(None, 10, i64::MAX, || false)
        .expect("mark sweep");
    assert!(page.removed.is_empty());
    store
        .remove_refs(std::slice::from_ref(&valid))
        .expect("remove ref");
    let page = store
        .mark_sweep_page(None, 10, i64::MAX, || false)
        .expect("mark sweep");
    assert_eq!(page.removed, vec![object.hash.clone()]);
    assert!(store.get_object(&object.hash).expect("catalog").is_none());
    assert!(
        !temp
            .path()
            .join("assets")
            .join(object.relative_path)
            .exists()
    );
    let stats = store.verify_catalog_ledger().expect("ledger");
    assert_eq!(stats.object_count, 0);
    assert_eq!(stats.active_bytes, 0);
    assert_eq!(stats.reference_count, 0);
}

#[test]
fn restart_preserves_catalog_and_safe_export_reads_the_object() {
    let temp = TempDir::new().expect("tempdir");
    let bytes = png(4, 5, 64);
    let hash = {
        let store = open_store(&temp);
        let mut source = Cursor::new(bytes.clone());
        store
            .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
            .expect("ingest")
            .object
            .hash
    };
    let reopened = open_store(&temp);
    let mut exported = Vec::new();
    let report = reopened
        .export_object(&hash, &mut exported, || false)
        .expect("export");
    assert_eq!(report.bytes_written, bytes.len() as u64);
    assert_eq!(exported, bytes);
}

#[test]
fn corruption_is_quarantined_and_a_later_ingest_repairs_the_object() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let bytes = png(7, 8, 0);
    let mut source = Cursor::new(bytes.clone());
    let object = store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest")
        .object;
    let object_path = temp.path().join("assets").join(&object.relative_path);
    fs::write(&object_path, b"corrupt").expect("corrupt object");

    let report = store
        .reconcile_catalog_page(None, 10, || false)
        .expect("reconcile");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.kind == ReconcileFindingKind::Corrupt)
    );
    assert_eq!(
        store
            .get_object(&object.hash)
            .expect("catalog")
            .expect("object")
            .state,
        AssetState::Quarantined
    );
    assert!(!object_path.exists());
    let quarantined_stats = store.verify_catalog_ledger().expect("ledger");
    assert_eq!(quarantined_stats.active_bytes, 0);
    assert_eq!(quarantined_stats.quarantined_count, 1);

    let mut replacement = Cursor::new(bytes);
    let repaired = store
        .ingest_uncancelled(&mut replacement, IngestRequest::new(AssetMime::Png))
        .expect("repair ingest");
    assert!(!repaired.deduplicated);
    assert_eq!(repaired.object.state, AssetState::Active);
    assert_eq!(store.stats().expect("stats").quarantined_count, 1);
    assert_eq!(
        store.verify_catalog_ledger().expect("ledger").active_bytes,
        repaired.object.size
    );

    let purged = store
        .purge_quarantine_page(None, 10, i64::MAX)
        .expect("purge quarantine");
    assert_eq!(purged.removed_names.len(), 1);
    assert_eq!(store.stats().expect("stats").quarantined_count, 0);
}

#[test]
fn a_missing_referenced_object_is_retained_in_catalog_and_can_be_repaired() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let bytes = png(11, 12, 0);
    let owner = AssetOwner::new("character", "missing-fixture").expect("owner");
    let mut source = Cursor::new(bytes.clone());
    let object = store
        .ingest_uncancelled(
            &mut source,
            IngestRequest::new(AssetMime::Png).with_owner(owner),
        )
        .expect("ingest")
        .object;
    let path = temp.path().join("assets").join(&object.relative_path);
    fs::remove_file(&path).expect("simulate missing object");
    let report = store
        .reconcile_catalog_page(None, 10, || false)
        .expect("reconcile");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.kind == ReconcileFindingKind::Missing)
    );
    let stats = store.verify_catalog_ledger().expect("ledger");
    assert_eq!(stats.object_count, 1);
    assert_eq!(stats.active_bytes, 0);
    assert_eq!(stats.reference_count, 1);
    assert_eq!(stats.missing_count, 1);
    assert!(
        store
            .mark_sweep_page(None, 10, i64::MAX, || false)
            .expect("mark sweep")
            .removed
            .is_empty()
    );

    let mut replacement = Cursor::new(bytes);
    let repaired = store
        .ingest_uncancelled(&mut replacement, IngestRequest::new(AssetMime::Png))
        .expect("repair");
    assert_eq!(repaired.object.state, AssetState::Active);
    let stats = store.verify_catalog_ledger().expect("ledger");
    assert_eq!(stats.missing_count, 0);
    assert_eq!(stats.reference_count, 1);
    assert_eq!(stats.active_bytes, repaired.object.size);
}

#[test]
fn filesystem_orphans_are_quarantined_only_by_explicit_shard_reconciliation() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let bytes = png(9, 9, 0);
    let hash = sha256(&bytes);
    let shard = &hash[..4];
    let directory = temp
        .path()
        .join("assets/objects")
        .join(&shard[..2])
        .join(&shard[2..]);
    fs::create_dir_all(&directory).expect("shard");
    fs::write(directory.join(&hash), bytes).expect("orphan object");

    drop(store);
    let reopened = open_store(&temp);
    assert!(
        directory.join(&hash).exists(),
        "startup must not scan objects"
    );
    let mut reconciliation = reopened
        .begin_shard_reconciliation(shard)
        .expect("open reconciliation cursor");
    let report = reconciliation
        .next_page(10, || false)
        .expect("explicit reconciliation");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| { finding.kind == ReconcileFindingKind::FilesystemOrphanQuarantined })
    );
    assert!(!directory.join(&hash).exists());
    assert_eq!(reopened.stats().expect("stats").quarantined_count, 1);
}

#[test]
fn shard_reconciliation_is_a_bounded_stateful_cursor() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let directory = temp.path().join("assets/objects/aa/bb");
    fs::create_dir_all(&directory).expect("shard");
    for name in ["bad-a", "bad-b", "bad-c"] {
        fs::write(directory.join(name), b"orphan").expect("orphan fixture");
    }
    let mut cursor = store.begin_shard_reconciliation("aabb").expect("cursor");
    let first = cursor.next_page(1, || false).expect("first page");
    assert_eq!(first.examined, 1);
    assert!(first.has_more);
    assert!(first.next_cursor.is_some());
    let second = cursor.next_page(1, || false).expect("second page");
    assert_eq!(second.examined, 1);
    assert!(second.has_more);
    let third = cursor.next_page(1, || false).expect("third page");
    assert_eq!(third.examined, 1);
    assert!(!third.has_more);
    assert!(third.next_cursor.is_none());
    assert_eq!(store.stats().expect("stats").quarantined_count, 3);
}

#[test]
fn stale_staging_is_catalog_driven_and_pageable() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let root = temp.path().join("assets");
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute(
            "INSERT INTO asset_staging(name, created_at_ms) VALUES ('deadbeef.partial', 0)",
            [],
        )
        .expect("staging metadata");
    fs::write(root.join(".staging/deadbeef.partial"), b"partial").expect("staging file");
    let page = store
        .cleanup_staging_page(None, 1, 0)
        .expect("cleanup staging");
    assert_eq!(page.removed_names, vec!["deadbeef.partial"]);
    assert_eq!(store.stats().expect("stats").staging_count, 0);
}

#[test]
fn a_hundred_thousand_catalog_rows_do_not_trigger_an_object_tree_scan_on_startup() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    drop(AssetStore::open(&root, limits()).expect("initialize store"));
    let mut connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    let transaction = connection.transaction().expect("transaction");
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO asset_objects(
                    hash, size, mime, relative_path, state, quarantine_name,
                    verified_at_ms, created_at_ms
                 ) VALUES (?1, 1, 'image/png', ?2, 'missing', NULL, 0, 0)",
            )
            .expect("statement");
        for index in 0u64..100_000 {
            let hash = format!("{index:064x}");
            let relative = format!("objects/{}/{}/{}", &hash[..2], &hash[2..4], hash);
            insert.execute(params![hash, relative]).expect("insert row");
        }
    }
    transaction.commit().expect("commit rows");
    drop(connection);

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(temp.path().join("does-not-exist"), root.join("objects/aa"))
            .expect("nested trap symlink");
    }

    let opened_at = Instant::now();
    let reopened = AssetStore::open(&root, limits()).expect("startup ignores nested object tree");
    eprintln!(
        "100,000-row DB-only startup validation: {:?}",
        opened_at.elapsed()
    );
    assert_eq!(reopened.stats().expect("stats").object_count, 100_000);
    assert_eq!(reopened.stats().expect("stats").active_bytes, 0);
}

#[test]
fn catalog_schema_and_quota_ledger_tampering_fail_closed() {
    let missing_trigger = TempDir::new().expect("tempdir");
    let root = missing_trigger.path().join("assets");
    drop(AssetStore::open(&root, limits()).expect("initialize"));
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute_batch("DROP TRIGGER asset_objects_totals_insert;")
        .expect("drop trigger");
    drop(connection);
    assert!(matches!(
        AssetStore::open(&root, limits()),
        Err(AssetError::IncompatibleCatalog {
            reason: "catalog schema fingerprint does not match the supported schema"
        })
    ));

    let altered_trigger = TempDir::new().expect("tempdir");
    let root = altered_trigger.path().join("assets");
    drop(AssetStore::open(&root, limits()).expect("initialize"));
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute_batch(
            "DROP TRIGGER asset_refs_totals_insert;
             CREATE TRIGGER asset_refs_totals_insert
             AFTER INSERT ON asset_refs BEGIN
                 UPDATE asset_totals
                 SET reference_count = reference_count + 1 + 0
                 WHERE id = 1;
             END;",
        )
        .expect("replace trigger with semantically similar SQL");
    drop(connection);
    assert!(matches!(
        AssetStore::open(&root, limits()),
        Err(AssetError::IncompatibleCatalog {
            reason: "catalog schema fingerprint does not match the supported schema"
        })
    ));

    let bad_ledger = TempDir::new().expect("tempdir");
    let root = bad_ledger.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("initialize");
    let mut source = Cursor::new(png(1, 1, 0));
    store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest");
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute(
            "UPDATE asset_totals SET active_bytes = active_bytes + 1",
            [],
        )
        .expect("tamper ledger");
    drop(connection);
    assert!(matches!(
        store.verify_catalog_ledger(),
        Err(AssetError::IncompatibleCatalog {
            reason: "quota ledger does not match catalog contents"
        })
    ));
    drop(store);
    assert!(matches!(
        AssetStore::open(&root, limits()),
        Err(AssetError::IncompatibleCatalog {
            reason: "quota ledger does not match catalog contents"
        })
    ));

    let future = TempDir::new().expect("tempdir");
    let root = future.path().join("assets");
    drop(AssetStore::open(&root, limits()).expect("initialize"));
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .pragma_update(None, "user_version", 99)
        .expect("future version");
    drop(connection);
    assert!(matches!(
        AssetStore::open(&root, limits()),
        Err(AssetError::SchemaVersion {
            found: 99,
            supported: 4
        })
    ));
}

#[test]
fn exact_legacy_v1_catalog_is_migrated_before_use() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    drop(AssetStore::open(&root, limits()).expect("initialize"));
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute_batch(
            "DROP TABLE asset_temporary_owner_sessions;
             DROP TABLE asset_backup_sessions;
             DROP TABLE asset_quarantine_intents;
             PRAGMA user_version = 1;",
        )
        .expect("construct exact v1 catalog");
    drop(connection);

    drop(AssetStore::open(&root, limits()).expect("migrate v1"));
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 4);
    let intent_table: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema
                WHERE type = 'table' AND name = 'asset_quarantine_intents'
             )",
            [],
            |row| row.get(0),
        )
        .expect("intent table");
    assert!(intent_table);
    let lease_table: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema
                WHERE type = 'table' AND name = 'asset_backup_sessions'
             )",
            [],
            |row| row.get(0),
        )
        .expect("backup lease table");
    assert!(lease_table);
    let temporary_session_table: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema
                WHERE type = 'table' AND name = 'asset_temporary_owner_sessions'
             )",
            [],
            |row| row.get(0),
        )
        .expect("temporary session table");
    assert!(temporary_session_table);
}

#[test]
fn v2_migration_releases_unleased_legacy_backup_pins() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("initialize");
    let mut source = Cursor::new(png(1, 1, 0));
    let hash = store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest")
        .object
        .hash;
    drop(store);

    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute(
            "INSERT INTO asset_refs(owner_type, owner_id, hash, created_at_ms)
             VALUES ('lorepia-backup', '0123456789abcdef0123456789abcdef', ?1, 0)",
            [hash.as_str()],
        )
        .expect("legacy unleased pin");
    connection
        .execute_batch(
            "DROP TABLE asset_temporary_owner_sessions;
             DROP TABLE asset_backup_sessions;
             PRAGMA user_version = 2;",
        )
        .expect("construct exact v2 catalog");
    drop(connection);

    let reopened = AssetStore::open(&root, limits()).expect("migrate v2");
    assert_eq!(reopened.stats().expect("stats").reference_count, 0);
    let swept = reopened
        .mark_sweep_page(None, 10, i64::MAX, || false)
        .expect("sweep unpinned object");
    assert_eq!(swept.removed, vec![hash]);
}

#[test]
fn v3_migration_reclaims_unleased_legacy_import_objects() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("initialize");
    let legacy_owner =
        AssetOwner::new("lorepia-import-session", "legacy-session").expect("legacy owner");
    let mut source = Cursor::new(png(1, 1, 0));
    store
        .ingest_uncancelled(
            &mut source,
            IngestRequest::new(AssetMime::Png).with_owner(legacy_owner),
        )
        .expect("legacy interrupted import");
    drop(store);

    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute_batch(
            "DROP TABLE asset_temporary_owner_sessions;
             PRAGMA user_version = 3;",
        )
        .expect("construct exact v3 catalog");
    drop(connection);

    let reopened = AssetStore::open(&root, limits()).expect("migrate and recover v3 import");
    let stats = reopened.stats().expect("recovered stats");
    assert_eq!(stats.object_count, 0);
    assert_eq!(stats.active_bytes, 0);
    assert_eq!(stats.reference_count, 0);
}

#[test]
fn restore_snapshot_validation_recalculates_authoritative_totals() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("store");
    let mut source = Cursor::new(png(1, 1, 0));
    store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest");
    let import_session =
        AssetOwner::new("lorepia-import-session", "snapshot-live-session").expect("session");
    store
        .begin_temporary_owner_session(&import_session)
        .expect("live import session");
    let mut temporary_source = Cursor::new(png(2, 2, 0));
    store
        .ingest_uncancelled(
            &mut temporary_source,
            IngestRequest::new(AssetMime::Png).with_owner(import_session.clone()),
        )
        .expect("temporary import object");
    let snapshot = temp.path().join("assets.snapshot.sqlite3");
    store
        .begin_backup_snapshot("0123456789abcdef0123456789abcdef", &snapshot, |_, _| true)
        .expect("snapshot");
    store
        .release_backup_snapshot("0123456789abcdef0123456789abcdef")
        .expect("release pins");

    let connection = Connection::open(&snapshot).expect("snapshot catalog");
    let operational_rows: i64 = connection
        .query_row(
            "SELECT
                (SELECT count(*) FROM asset_backup_sessions)
                + (SELECT count(*) FROM asset_refs WHERE owner_type = 'lorepia-backup')
                + (SELECT count(*) FROM asset_temporary_owner_sessions)
                + (SELECT count(*) FROM asset_refs WHERE owner_type = 'lorepia-import-session')",
            [],
            |row| row.get(0),
        )
        .expect("operational backup rows");
    assert_eq!(operational_rows, 0);
    store
        .rollback_temporary_owner(&import_session)
        .expect("cleanup source import session");
    connection
        .execute(
            "UPDATE asset_totals SET active_bytes = active_bytes + 1 WHERE id = 1",
            [],
        )
        .expect("tamper snapshot totals");
    drop(connection);
    let validation = AssetStore::validate_catalog_snapshot_file(&snapshot);
    assert!(
        matches!(
            validation,
            Err(AssetError::IncompatibleCatalog {
                reason: "quota ledger does not match catalog contents"
            })
        ),
        "unexpected validation result: {validation:?}"
    );
}

#[test]
fn failed_snapshot_creation_immediately_releases_pins_and_lease() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("store");
    let mut source = Cursor::new(png(1, 1, 0));
    store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest");
    let session = "0123456789abcdef0123456789abcdef";
    let snapshot = temp.path().join("cancelled.sqlite3");

    assert!(matches!(
        store.begin_backup_snapshot(session, &snapshot, |_, _| false),
        Err(AssetError::SnapshotCancelled)
    ));
    assert!(!snapshot.exists());
    assert_eq!(
        store.renew_backup_snapshot(session).expect("lookup lease"),
        None
    );
    assert_eq!(store.stats().expect("stats").reference_count, 0);
}

#[test]
fn backup_snapshot_lease_is_renewable_abandonable_and_stale_cleanup_is_bounded() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("store");
    let mut source = Cursor::new(png(1, 1, 0));
    let object = store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest")
        .object;
    let first = "0123456789abcdef0123456789abcdef";
    let second = "fedcba9876543210fedcba9876543210";
    store
        .begin_backup_snapshot(first, temp.path().join("first.sqlite3"), |_, _| true)
        .expect("first snapshot");
    store
        .begin_backup_snapshot(second, temp.path().join("second.sqlite3"), |_, _| true)
        .expect("second snapshot");

    let first_lease = store
        .renew_backup_snapshot(first)
        .expect("renew")
        .expect("first lease");
    assert_eq!(first_lease.pinned_objects, 1);
    let active = store
        .cleanup_stale_backup_snapshots(0, 1)
        .expect("active cleanup");
    assert!(active.removed_sessions.is_empty());

    let first_page = store
        .cleanup_stale_backup_snapshots(i64::MAX, 1)
        .expect("bounded cleanup");
    assert_eq!(first_page.removed_sessions.len(), 1);
    assert_eq!(first_page.released_pins, 1);
    let removed = first_page.removed_sessions[0].as_str();
    let survivor = if removed == first { second } else { first };
    assert!(
        store
            .renew_backup_snapshot(removed)
            .expect("removed lookup")
            .is_none()
    );
    assert!(
        store
            .renew_backup_snapshot(survivor)
            .expect("survivor lookup")
            .is_some()
    );

    assert_eq!(store.abandon_backup_snapshot(survivor).expect("abandon"), 1);
    assert_eq!(
        store.abandon_backup_snapshot(survivor).expect("idempotent"),
        0
    );
    let swept = store
        .mark_sweep_page(None, 10, i64::MAX, || false)
        .expect("sweep");
    assert_eq!(swept.removed, vec![object.hash]);
}

#[test]
fn lease_renewal_never_moves_backwards_when_the_wall_clock_regresses() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("store");
    let session = "0123456789abcdef0123456789abcdef";
    store
        .begin_backup_snapshot(session, temp.path().join("snapshot.sqlite3"), |_, _| true)
        .expect("snapshot");
    let future = i64::MAX - 1;
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute(
            "UPDATE asset_backup_sessions SET lease_updated_at_ms = ?2 WHERE session_id = ?1",
            rusqlite::params![session, future],
        )
        .expect("simulate future lease");
    drop(connection);

    let renewed = store
        .renew_backup_snapshot(session)
        .expect("renew")
        .expect("lease");
    assert_eq!(renewed.lease_updated_at_ms, future);
    store.abandon_backup_snapshot(session).expect("cleanup");
}

#[test]
fn forged_quarantine_intent_cannot_select_a_parent_path() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    drop(AssetStore::open(&root, limits()).expect("initialize"));
    let protected = root.join("protected.txt");
    fs::write(&protected, b"must remain").expect("protected fixture");
    let connection = Connection::open(root.join("assets.sqlite3")).expect("catalog");
    connection
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             INSERT INTO asset_quarantine_intents(
                name, operation, phase, source_relative_path,
                original_hash, reason, created_at_ms
             ) VALUES (
                '../protected.txt', 'purge', 'prepared', NULL,
                NULL, 'forged traversal', 0
             );",
        )
        .expect("forge row as if catalog constraints were bypassed");
    drop(connection);

    assert!(AssetStore::open(&root, limits()).is_err());
    assert_eq!(
        fs::read(&protected).expect("protected file"),
        b"must remain"
    );
}

#[cfg(unix)]
#[test]
fn symlinks_are_never_followed_for_object_open_or_root_layout() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().expect("tempdir");
    let unsafe_root = temp.path().join("unsafe");
    fs::create_dir(&unsafe_root).expect("root");
    fs::create_dir(temp.path().join("outside-objects")).expect("outside");
    symlink(
        temp.path().join("outside-objects"),
        unsafe_root.join("objects"),
    )
    .expect("symlink");
    assert!(matches!(
        AssetStore::open(&unsafe_root, limits()),
        Err(AssetError::UnsafeFilesystem { .. })
    ));

    let safe_root = temp.path().join("safe");
    let store = AssetStore::open(&safe_root, limits()).expect("store");
    let bytes = png(2, 2, 0);
    let mut source = Cursor::new(bytes);
    let object = store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest")
        .object;
    let object_path = safe_root.join(&object.relative_path);
    let outside = temp.path().join("outside-file");
    fs::rename(&object_path, &outside).expect("move object");
    symlink(&outside, &object_path).expect("object symlink");
    assert!(matches!(
        store.open_object(&object.hash),
        Err(AssetError::UnsafeFilesystem { .. })
    ));

    let catalog_root = temp.path().join("catalog-link");
    drop(AssetStore::open(&catalog_root, limits()).expect("catalog fixture"));
    let moved_catalog = temp.path().join("moved-assets.sqlite3");
    fs::rename(catalog_root.join("assets.sqlite3"), &moved_catalog).expect("move catalog");
    symlink(&moved_catalog, catalog_root.join("assets.sqlite3")).expect("catalog symlink");
    assert!(matches!(
        AssetStore::open(&catalog_root, limits()),
        Err(AssetError::UnsafeFilesystem { .. })
    ));
}

#[cfg(unix)]
#[test]
fn opened_directory_boundaries_resist_post_open_path_swaps() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("store");
    let bytes = png(2, 2, 0);
    let mut source = Cursor::new(bytes.clone());
    let object = store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest")
        .object;

    let outside_staging = temp.path().join("outside-staging");
    fs::create_dir(&outside_staging).expect("outside staging");
    fs::rename(root.join(".staging"), root.join("staging-opened")).expect("swap staging");
    symlink(&outside_staging, root.join(".staging")).expect("replacement staging symlink");
    let mut second_source = Cursor::new(png(3, 3, 0));
    store
        .ingest_uncancelled(&mut second_source, IngestRequest::new(AssetMime::Png))
        .expect("staging remains on opened descriptor");
    assert_eq!(count_regular_files(&outside_staging), 0);

    let outside_quarantine = temp.path().join("outside-quarantine");
    fs::create_dir(&outside_quarantine).expect("outside quarantine");
    fs::rename(root.join("quarantine"), root.join("quarantine-opened")).expect("swap quarantine");
    symlink(&outside_quarantine, root.join("quarantine")).expect("replacement quarantine symlink");
    let corrupt_bytes = png(4, 4, 0);
    let corrupt_hash = sha256(&corrupt_bytes);
    let corrupt_path = root
        .join("objects")
        .join(&corrupt_hash[..2])
        .join(&corrupt_hash[2..4])
        .join(&corrupt_hash);
    fs::create_dir_all(corrupt_path.parent().expect("corrupt parent")).expect("corrupt shards");
    fs::write(&corrupt_path, b"wrong").expect("corrupt object");
    let mut corrupt_source = Cursor::new(corrupt_bytes);
    store
        .ingest_uncancelled(&mut corrupt_source, IngestRequest::new(AssetMime::Png))
        .expect("quarantine remains on opened descriptor");
    assert_eq!(store.stats().expect("stats").quarantined_count, 1);
    assert_eq!(count_regular_files(&outside_quarantine), 0);

    let outside_objects = temp.path().join("outside-objects");
    fs::create_dir(&outside_objects).expect("outside objects");
    fs::rename(root.join("objects"), root.join("objects-opened")).expect("swap objects");
    symlink(&outside_objects, root.join("objects")).expect("replacement objects symlink");
    let mut reader = store
        .open_object(&object.hash)
        .expect("opened descriptor remains inside original boundary");
    let mut observed = Vec::new();
    reader.read_to_end(&mut observed).expect("read object");
    assert_eq!(observed, bytes);
    assert_eq!(count_regular_files(&outside_objects), 0);

    let outside_root = temp.path().join("outside-root");
    fs::create_dir(&outside_root).expect("outside root");
    fs::rename(&root, temp.path().join("assets-opened")).expect("swap root");
    symlink(&outside_root, &root).expect("replacement root symlink");
    assert!(matches!(
        store.stats(),
        Err(AssetError::UnsafeFilesystem { .. })
    ));
    assert_eq!(count_regular_files(&outside_root), 0);
}

#[test]
fn image_dimension_bombs_are_rejected_before_decode() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let mut bomb = Cursor::new(png(100_000, 100_000, 0));
    assert!(matches!(
        store.ingest_uncancelled(&mut bomb, IngestRequest::new(AssetMime::Png)),
        Err(AssetError::InvalidInput {
            field: "image dimensions",
            ..
        })
    ));
}

#[test]
fn export_observes_cancellation_without_exposing_an_absolute_object_path() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let mut source = Cursor::new(png(1, 1, 128 * 1024));
    let object = store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest")
        .object;
    assert!(!Path::new(&object.relative_path).is_absolute());
    let mut destination = Vec::new();
    let mut polls = 0;
    assert!(matches!(
        store.export_object(&object.hash, &mut destination, || {
            polls += 1;
            polls >= 2
        }),
        Err(AssetError::Cancelled)
    ));
    assert!(!destination.is_empty());
    assert!(destination.len() < object.size as usize);
}

#[test]
fn invalid_declared_mime_is_rejected_by_the_allowlist() {
    assert!(AssetMime::from_str("image/svg+xml").is_err());
    assert!(AssetMime::from_str("text/html").is_err());
    assert!(AssetMime::from_str("image/png").is_ok());
}

#[test]
fn corrupted_hash_path_is_quarantined_before_correct_content_is_published() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("assets");
    let store = AssetStore::open(&root, limits()).expect("store");
    let bytes = png(3, 3, 0);
    let hash = sha256(&bytes);
    let final_path = root
        .join("objects")
        .join(&hash[..2])
        .join(&hash[2..4])
        .join(&hash);
    fs::create_dir_all(final_path.parent().expect("parent")).expect("shard");
    fs::write(&final_path, b"wrong bytes").expect("collision fixture");

    let mut source = Cursor::new(bytes.clone());
    let outcome = store
        .ingest_uncancelled(&mut source, IngestRequest::new(AssetMime::Png))
        .expect("ingest replaces corrupt hash path safely");
    assert_eq!(outcome.object.hash.as_str(), hash);
    assert_eq!(fs::read(final_path).expect("published object"), bytes);
    assert_eq!(store.stats().expect("stats").quarantined_count, 1);
}

#[test]
fn source_read_failure_does_not_publish_a_partial_object() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let mut failing = FailingReader {
        bytes: Cursor::new(png(1, 1, 0)),
        reads: 0,
    };
    let _ = store.ingest_uncancelled(&mut failing, IngestRequest::new(AssetMime::Png));
    let object_entries = count_regular_files(&temp.path().join("assets/objects"));
    assert_eq!(object_entries, 0);
}

#[test]
fn temporary_owner_commit_and_rollback_are_atomic_idempotent_and_dedup_safe() {
    let temp = TempDir::new().expect("tempdir");
    let store = open_store(&temp);
    let existing_owner = AssetOwner::new("character", "existing").expect("existing owner");
    let final_owner = AssetOwner::new("character", "imported").expect("final owner");
    let session = AssetOwner::new("lorepia-import-session", "session-a").expect("session");

    let existing_bytes = png(2, 3, 0);
    let mut existing_source = Cursor::new(existing_bytes.clone());
    let existing = store
        .ingest_uncancelled(
            &mut existing_source,
            IngestRequest::new(AssetMime::Png).with_owner(existing_owner.clone()),
        )
        .expect("existing referenced object");
    let mut duplicate = Cursor::new(existing_bytes);
    assert!(
        store
            .ingest_uncancelled(
                &mut duplicate,
                IngestRequest::new(AssetMime::Png).with_owner(session.clone()),
            )
            .expect("session duplicate")
            .deduplicated
    );
    let mut new_source = Cursor::new(png(4, 5, 0));
    let new_object = store
        .ingest_uncancelled(
            &mut new_source,
            IngestRequest::new(AssetMime::Png).with_owner(session.clone()),
        )
        .expect("session object")
        .object;

    assert_eq!(
        store.rollback_temporary_owner(&session).expect("rollback"),
        1
    );
    assert_eq!(
        store
            .rollback_temporary_owner(&session)
            .expect("idempotent rollback"),
        0
    );
    assert!(
        store
            .get_object(&existing.object.hash)
            .expect("lookup")
            .is_some()
    );
    assert!(
        store
            .get_object(&new_object.hash)
            .expect("lookup")
            .is_none()
    );
    let stats = store.stats().expect("stats");
    assert_eq!(stats.object_count, 1);
    assert_eq!(stats.reference_count, 1);

    let second_session =
        AssetOwner::new("lorepia-import-session", "session-b").expect("second session");
    store
        .begin_temporary_owner_session(&second_session)
        .expect("register second session");
    let mut promoted_source = Cursor::new(png(6, 7, 0));
    let promoted = store
        .ingest_uncancelled(
            &mut promoted_source,
            IngestRequest::new(AssetMime::Png).with_owner(second_session.clone()),
        )
        .expect("promoted object")
        .object;
    let reserved_final =
        AssetOwner::new("lorepia-import-session", "not-a-final-owner").expect("reserved final");
    assert!(matches!(
        store.commit_temporary_owner_refs(&second_session, &reserved_final, 1),
        Err(AssetError::InvalidInput {
            field: "temporary owner",
            ..
        })
    ));
    assert_eq!(store.stats().expect("stats").reference_count, 2);
    assert_eq!(
        store
            .commit_temporary_owner_refs(&second_session, &final_owner, 1)
            .expect("atomic promotion"),
        1
    );
    assert_eq!(
        store
            .rollback_temporary_owner(&second_session)
            .expect("committed session has nothing to roll back"),
        0
    );
    assert!(store.get_object(&promoted.hash).expect("lookup").is_some());
    assert_eq!(store.stats().expect("stats").reference_count, 2);

    let empty_session =
        AssetOwner::new("lorepia-import-session", "session-empty").expect("empty session");
    store
        .begin_temporary_owner_session(&empty_session)
        .expect("register empty session");
    assert!(matches!(
        store.commit_temporary_owner_refs(&empty_session, &final_owner, 1),
        Err(AssetError::InvalidInput {
            field: "temporary owner references",
            ..
        })
    ));
    assert!(
        store
            .temporary_owner_session_is_live(&empty_session)
            .expect("failed promotion preserves recoverable lease")
    );
    store
        .finish_empty_temporary_owner_session(&empty_session)
        .expect("close empty session");
}

fn count_regular_files(root: &Path) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(std::result::Result::ok)
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                count_regular_files(&path)
            } else {
                usize::from(path.is_file())
            }
        })
        .sum()
}
