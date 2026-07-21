mod common;

use lorepia_storage::{MAX_SAFE_INTEGER, Store, WalCheckpointPolicy};
use rusqlite::Connection;

use common::{create_chat, database, timestamp};

fn prepared_wal() -> (tempfile::TempDir, std::path::PathBuf, Store, Connection) {
    let (directory, path) = database();
    let store = Store::open_at(&path, timestamp(10)).expect("open store");
    let _chat = create_chat(&store, 20);
    let writer = Connection::open(&path).expect("open WAL writer");
    writer
        .execute_batch("PRAGMA wal_autocheckpoint = 0")
        .expect("disable writer auto-checkpoint");
    for revision in 1..=128_i64 {
        writer
            .execute(
                "UPDATE settings
                 SET revision = ?1, updated_at_ms = ?1
                 WHERE singleton = 1",
                [revision],
            )
            .expect("append WAL frame");
    }
    (directory, path, store, writer)
}

#[test]
fn passive_only_policy_never_escalates() {
    let (_directory, _path, store, _writer) = prepared_wal();
    let report = store
        .maintain_wal(WalCheckpointPolicy {
            restart_threshold_bytes: None,
            emergency_truncate_threshold_bytes: None,
        })
        .expect("passive checkpoint");

    assert!(report.passive.log_frames > 0);
    assert!(!report.threshold_exceeded);
    assert!(report.restart.is_none());
    assert!(!report.emergency_truncate_threshold_exceeded);
    assert!(report.truncate.is_none());
}

#[test]
fn restart_threshold_escalates_without_truncating_below_emergency_threshold() {
    let (_directory, _path, store, _writer) = prepared_wal();
    let report = store
        .maintain_wal(WalCheckpointPolicy {
            restart_threshold_bytes: Some(1),
            emergency_truncate_threshold_bytes: Some(MAX_SAFE_INTEGER),
        })
        .expect("restart checkpoint");

    assert!(report.threshold_exceeded);
    assert!(report.restart.is_some());
    assert!(!report.emergency_truncate_threshold_exceeded);
    assert!(report.truncate.is_none());
}

#[test]
fn healthy_restart_may_emergency_truncate_and_shrink_the_wal() {
    let (_directory, _path, store, _writer) = prepared_wal();
    let report = store
        .maintain_wal(WalCheckpointPolicy {
            restart_threshold_bytes: Some(1),
            emergency_truncate_threshold_bytes: Some(1),
        })
        .expect("emergency truncate checkpoint");

    let restart = report.restart.expect("restart telemetry");
    let truncate = report.truncate.expect("truncate telemetry");
    assert!(!restart.busy);
    assert_eq!(restart.remaining_frames, 0);
    assert!(report.emergency_truncate_threshold_exceeded);
    assert!(!truncate.busy);
    assert_eq!(truncate.remaining_frames, 0);
    assert!(truncate.wal_file_bytes < restart.wal_file_bytes);
}

#[test]
fn long_reader_is_observable_and_never_forces_emergency_truncate() {
    let (_directory, path) = database();
    let store = Store::open_at(&path, timestamp(10)).expect("open store");
    let _chat = create_chat(&store, 20);

    let reader = Connection::open(&path).expect("open long reader");
    reader
        .execute_batch("BEGIN DEFERRED")
        .expect("begin long read transaction");
    let _: i64 = reader
        .query_row(
            "SELECT revision FROM settings WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .expect("establish reader snapshot");

    let writer = Connection::open(&path).expect("open WAL writer");
    writer
        .execute_batch("PRAGMA wal_autocheckpoint = 0")
        .expect("disable writer auto-checkpoint");
    for revision in 1..=128_i64 {
        writer
            .execute(
                "UPDATE settings
                 SET revision = ?1, updated_at_ms = ?1
                 WHERE singleton = 1",
                [revision],
            )
            .expect("append WAL frame");
    }

    let blocked = store
        .maintain_wal(WalCheckpointPolicy {
            restart_threshold_bytes: Some(1),
            emergency_truncate_threshold_bytes: Some(1),
        })
        .expect("observe checkpoint starvation");
    assert!(blocked.threshold_exceeded);
    assert!(blocked.restart.is_some());
    assert!(blocked.starvation_observed);
    assert!(blocked.truncate.is_none());
    assert!(
        blocked.passive.remaining_frames > 0 || blocked.restart.expect("restart telemetry").busy
    );

    reader.execute_batch("COMMIT").expect("release long reader");
    let recovered = store
        .maintain_wal(WalCheckpointPolicy {
            restart_threshold_bytes: Some(1),
            emergency_truncate_threshold_bytes: Some(MAX_SAFE_INTEGER),
        })
        .expect("checkpoint after reader release");
    assert!(!recovered.passive.busy);
    assert_eq!(recovered.passive.remaining_frames, 0);
    if let Some(restart) = recovered.restart {
        assert!(!restart.busy);
        assert_eq!(restart.remaining_frames, 0);
    }
    assert!(recovered.truncate.is_none());
    assert!(!recovered.starvation_observed);
    assert!(recovered.passive.page_size_bytes >= 512);
}

#[test]
fn emergency_threshold_is_closed_and_requires_a_restart_policy() {
    let (_directory, _path, store, _writer) = prepared_wal();
    for policy in [
        WalCheckpointPolicy {
            restart_threshold_bytes: None,
            emergency_truncate_threshold_bytes: Some(1),
        },
        WalCheckpointPolicy {
            restart_threshold_bytes: Some(2),
            emergency_truncate_threshold_bytes: Some(1),
        },
        WalCheckpointPolicy {
            restart_threshold_bytes: Some(1),
            emergency_truncate_threshold_bytes: Some(0),
        },
        WalCheckpointPolicy {
            restart_threshold_bytes: Some(1),
            emergency_truncate_threshold_bytes: Some(MAX_SAFE_INTEGER + 1),
        },
    ] {
        assert!(store.maintain_wal(policy).is_err());
    }
}
