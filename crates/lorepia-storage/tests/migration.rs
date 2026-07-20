mod common;

use lorepia_storage::{
    BUSY_TIMEOUT_MS, CURRENT_SCHEMA_VERSION, DefaultMode, ProviderId, StorageError, Store, Theme,
};
use rusqlite::Connection;

use common::{database, timestamp};

#[test]
fn initializes_v1_with_wal_and_idempotently_reopens() {
    let (_directory, path) = database();
    let first = Store::open_at(&path, timestamp(100)).expect("initialize database");
    assert_eq!(
        first.startup_report().schema_version,
        CURRENT_SCHEMA_VERSION
    );
    assert_eq!(first.startup_report().journal_mode, "wal");
    assert_eq!(first.startup_report().busy_timeout_ms, BUSY_TIMEOUT_MS);
    assert_eq!(first.startup_report().recovered_request_count, 0);

    let defaults = first.load_preferences().expect("default preferences");
    assert_eq!(defaults.selected_provider_id, ProviderId::OpenAi);
    assert_eq!(defaults.theme, Theme::System);
    assert_eq!(defaults.default_mode, DefaultMode::Chat);
    assert_eq!(defaults.revision, 0);
    assert_eq!(defaults.updated_at_ms, timestamp(0));
    assert_eq!(defaults.model_ids, Default::default());
    drop(first);

    let reopened = Store::open_at(&path, timestamp(200)).expect("reopen database");
    assert_eq!(
        reopened.startup_report().schema_version,
        CURRENT_SCHEMA_VERSION
    );
    assert_eq!(reopened.startup_report().recovered_request_count, 0);

    let connection = Connection::open(&path).expect("inspect database");
    let user_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read schema version");
    assert_eq!(user_version, CURRENT_SCHEMA_VERSION);
    let meta_version: i64 = connection
        .query_row(
            "SELECT schema_version FROM schema_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .expect("read schema metadata");
    assert_eq!(meta_version, CURRENT_SCHEMA_VERSION);
}

#[test]
fn rejects_future_schema_without_changing_its_data_or_version() {
    let (_directory, path) = database();
    {
        let connection = Connection::open(&path).expect("create future database");
        connection
            .execute_batch(
                "CREATE TABLE future_marker(value TEXT NOT NULL);
                 INSERT INTO future_marker(value) VALUES ('preserve-me');
                 PRAGMA user_version = 99;",
            )
            .expect("seed future database");
    }

    let error = Store::open_at(&path, timestamp(100)).expect_err("reject future schema");
    assert!(matches!(
        error,
        StorageError::FutureSchema {
            found: 99,
            supported: CURRENT_SCHEMA_VERSION
        }
    ));

    let connection = Connection::open(&path).expect("reopen future database");
    let user_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read schema version");
    let marker: String = connection
        .query_row("SELECT value FROM future_marker", [], |row| row.get(0))
        .expect("read marker");
    assert_eq!(user_version, 99);
    assert_eq!(marker, "preserve-me");
}

#[test]
fn rejects_nonempty_unversioned_database_without_adopting_it() {
    let (_directory, path) = database();
    {
        let connection = Connection::open(&path).expect("create unknown database");
        connection
            .execute("CREATE TABLE unrelated(value TEXT)", [])
            .expect("seed unknown table");
    }

    let error = Store::open_at(&path, timestamp(100)).expect_err("reject unknown database");
    assert!(matches!(
        error,
        StorageError::IncompatibleSchema {
            reason: "unversioned database is not empty"
        }
    ));
    let connection = Connection::open(&path).expect("reopen unknown database");
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'unrelated')",
            [],
            |row| row.get(0),
        )
        .expect("read unknown table");
    assert!(exists);
}

#[test]
fn persisted_schema_has_no_credential_or_raw_provider_error_columns() {
    let (_directory, path) = database();
    let store = Store::open_at(&path, timestamp(100)).expect("initialize database");
    drop(store);
    let connection = Connection::open(&path).expect("inspect database");

    let mut names = Vec::new();
    for table in ["settings", "request_state"] {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare schema query");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query columns");
        for row in rows {
            names.push(row.expect("column name"));
        }
    }
    let joined = names.join(" ").to_ascii_lowercase();
    for forbidden in [
        "api_key",
        "credential",
        "secret",
        "control_token",
        "raw_error",
        "provider_body",
    ] {
        assert!(!joined.contains(forbidden), "forbidden column {forbidden}");
    }
}

#[test]
fn rejects_v1_database_with_changed_index_or_trigger_definition() {
    let (_directory, path) = database();
    let store = Store::open_at(&path, timestamp(100)).expect("initialize database");
    drop(store);
    {
        let connection = Connection::open(&path).expect("tamper database");
        connection
            .execute_batch(
                "DROP INDEX request_state_one_running_per_chat;
                 DROP TRIGGER messages_fts_ai;
                 CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
                     SELECT 1;
                 END;",
            )
            .expect("replace schema definitions");
    }

    let error = Store::open_at(&path, timestamp(200)).expect_err("reject changed schema");
    assert!(matches!(error, StorageError::IncompatibleSchema { .. }));
}

#[test]
fn rejects_foreign_key_violation_before_startup_recovery() {
    let (_directory, path) = database();
    let store = Store::open_at(&path, timestamp(100)).expect("initialize database");
    drop(store);
    {
        let connection = Connection::open(&path).expect("tamper database");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 INSERT INTO messages(
                    id, chat_id, ordinal, role, status, text, created_at_ms, updated_at_ms
                 ) VALUES (
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    1, 'user', 'complete', 'orphan', 100, 100
                 );",
            )
            .expect("insert orphan with foreign keys disabled");
    }

    let error = Store::open_at(&path, timestamp(200)).expect_err("reject orphan row");
    assert!(matches!(
        error,
        StorageError::IncompatibleSchema {
            reason: "foreign key check failed"
        }
    ));
}
