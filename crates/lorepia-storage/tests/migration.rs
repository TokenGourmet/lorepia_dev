mod common;

use lorepia_storage::{
    BUSY_TIMEOUT_MS, CURRENT_SCHEMA_VERSION, DefaultMode, ProviderId, RequestStatus, StorageError,
    Store, Theme,
};
use rusqlite::Connection;

use common::{begin_turn, checkpoint, create_chat, database, deliver_through, timestamp};

const MIGRATION_V1: &str = include_str!("../migrations/0001_chat_persistence.sql");
const MIGRATION_V2: &str = include_str!("../migrations/0002_stream_journal.sql");
const LEGACY_CHAT_ID: &str = "11111111111111111111111111111111";
const LEGACY_USER_MESSAGE_ID: &str = "22222222222222222222222222222222";
const LEGACY_ASSISTANT_MESSAGE_ID: &str = "33333333333333333333333333333333";
const LEGACY_REQUEST_ID: &str = "44444444444444444444444444444444";

#[test]
fn initializes_v3_with_wal_and_idempotently_reopens() {
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
fn rejects_v3_database_with_changed_index_or_trigger_definition() {
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
fn migrates_exact_v1_through_v3_without_losing_messages_or_request_metadata() {
    let (_directory, path) = database();
    seed_legacy_v1(&path);

    let store = Store::open_at(&path, timestamp(200)).expect("migrate v1 database");
    assert_eq!(store.startup_report().schema_version, 3);
    assert_eq!(store.startup_report().recovered_request_count, 1);
    let request_id =
        lorepia_storage::RequestStateId::parse(LEGACY_REQUEST_ID).expect("legacy request ID");
    let state = store
        .get_request_state(&request_id)
        .expect("migrated request state");
    assert_eq!(state.owner_label.as_str(), "legacy-v1");
    assert_eq!(state.stream_generation.as_str(), LEGACY_REQUEST_ID);
    assert_eq!(state.last_delivered_seq, 7);
    assert_eq!(state.last_durable_seq, 7);
    assert_eq!(state.last_acked_seq, None);
    assert_eq!(
        state.provider_response_id.as_deref(),
        Some("legacy-response")
    );
    assert_eq!(state.usage.expect("legacy usage").output_tokens, 7);
    assert_eq!(state.status, RequestStatus::Interrupted);
    let chat_id = lorepia_storage::ChatId::parse(LEGACY_CHAT_ID).expect("legacy chat ID");
    let messages = store
        .load_messages(&chat_id, None, 10)
        .expect("migrated messages");
    assert_eq!(messages.messages.len(), 2);
    assert_eq!(messages.messages[0].text, "legacy user");
    assert_eq!(messages.messages[1].text, "legacy partial");
    assert_eq!(messages.messages[0].parent_id, None);
    assert_eq!(messages.messages[0].depth, 0);
    assert_eq!(messages.messages[0].completed_at_ms, Some(timestamp(110)));
    assert_eq!(
        messages.messages[1]
            .parent_id
            .as_ref()
            .map(|id| id.as_str()),
        Some(LEGACY_USER_MESSAGE_ID)
    );
    assert_eq!(messages.messages[1].depth, 1);
    assert_eq!(messages.messages[1].completed_at_ms, None);
    let active = store
        .load_active_path(&chat_id, None, 10)
        .expect("migrated active path");
    assert_eq!(active.entries.len(), 2);
    assert_eq!(
        active.entries[0].message.id.as_str(),
        LEGACY_USER_MESSAGE_ID
    );
    assert_eq!(
        active.entries[1].message.id.as_str(),
        LEGACY_ASSISTANT_MESSAGE_ID
    );
    drop(store);

    let connection = Connection::open(&path).expect("inspect migrated database");
    let columns = table_columns(&connection, "request_state");
    assert!(columns.iter().any(|column| column == "owner_label"));
    assert!(columns.iter().any(|column| column == "stream_generation"));
    assert!(columns.iter().any(|column| column == "last_delivered_seq"));
    assert!(columns.iter().any(|column| column == "last_durable_seq"));
    assert!(columns.iter().any(|column| column == "last_acked_seq"));
    assert!(!columns.iter().any(|column| column == "last_seq"));
    let migrated_at_ms: i64 = connection
        .query_row(
            "SELECT migrated_at_ms FROM schema_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .expect("migration timestamp");
    assert_eq!(migrated_at_ms, 200);
}

#[test]
fn migrates_exact_v2_to_v3_and_keeps_request_journal_contract() {
    let (_directory, path) = database();
    seed_legacy_v2(&path);

    let store = Store::open_at(&path, timestamp(250)).expect("migrate exact v2 database");
    assert_eq!(store.startup_report().schema_version, 3);
    assert_eq!(store.startup_report().recovered_request_count, 1);
    let request_id = lorepia_storage::RequestStateId::parse(LEGACY_REQUEST_ID).expect("request ID");
    let request = store
        .get_request_state(&request_id)
        .expect("request journal after v3 migration");
    assert_eq!(request.owner_label.as_str(), "legacy-v1");
    assert_eq!(request.stream_generation.as_str(), LEGACY_REQUEST_ID);
    assert_eq!(request.last_delivered_seq, 7);
    assert_eq!(request.last_durable_seq, 7);
    assert_eq!(request.last_acked_seq, None);

    let chat_id = lorepia_storage::ChatId::parse(LEGACY_CHAT_ID).expect("chat ID");
    assert!(
        store
            .search_messages(&chat_id, "legacy user", 10)
            .expect("completed FTS hit")
            .iter()
            .any(|hit| hit.message.id.as_str() == LEGACY_USER_MESSAGE_ID)
    );
    assert!(
        store
            .search_messages(&chat_id, "legacy partial", 10)
            .expect("partial FTS exclusion")
            .is_empty()
    );
}

#[test]
fn rejects_tampered_v1_before_migration_and_leaves_it_at_v1() {
    let (_directory, path) = database();
    seed_legacy_v1(&path);
    {
        let connection = Connection::open(&path).expect("tamper v1 database");
        connection
            .execute_batch("DROP INDEX request_state_chat_started;")
            .expect("remove required v1 index");
    }

    let error = Store::open_at(&path, timestamp(200)).expect_err("reject tampered v1");
    assert!(matches!(error, StorageError::IncompatibleSchema { .. }));
    let connection = Connection::open(&path).expect("inspect rejected v1 database");
    let user_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("v1 user version");
    assert_eq!(user_version, 1);
    let columns = table_columns(&connection, "request_state");
    assert!(columns.iter().any(|column| column == "last_seq"));
    assert!(!columns.iter().any(|column| column == "last_durable_seq"));
    let preserved: String = connection
        .query_row(
            "SELECT text FROM messages WHERE id = ?1",
            [LEGACY_ASSISTANT_MESSAGE_ID],
            |row| row.get(0),
        )
        .expect("preserved v1 message");
    assert_eq!(preserved, "legacy partial");
}

#[test]
fn rejects_rows_that_violate_stream_sequence_invariant_even_if_checks_were_bypassed() {
    let (_directory, path) = database();
    let store = Store::open_at(&path, timestamp(10)).expect("initialize database");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);
    deliver_through(&store, &started, 0, 1, 35);
    store
        .checkpoint_response(checkpoint(&started, 0, 1, "durable", 40))
        .expect("checkpoint response");
    {
        let connection = Connection::open(&path).expect("tamper sequence");
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .expect("allow corruption fixture");
        connection
            .execute(
                "UPDATE request_state SET last_delivered_seq = 0 WHERE id = ?1",
                [started.request_state_id.as_str()],
            )
            .expect("break durable <= delivered invariant");
    }

    let decode_error = store
        .get_request_state(&started.request_state_id)
        .expect_err("Rust decoder rejects corrupt journal row");
    assert!(matches!(
        decode_error,
        StorageError::IncompatibleSchema {
            reason: "stream journal sequence invariant failed"
        }
    ));
    drop(store);

    // Normal startup deliberately performs only fixed-cost schema identity and
    // interrupted-request recovery. Full row scans belong to the explicit
    // diagnostic/restore boundary so a multi-gigabyte database cannot make
    // every app launch O(total rows).
    let reopened = Store::open_at(&path, timestamp(100))
        .expect("normal startup does not scan every journal row");
    drop(reopened);
    let validation_error = Store::validate_snapshot_file(&path)
        .expect_err("explicit validation rejects corrupt journal row");
    assert!(matches!(
        validation_error,
        StorageError::IncompatibleSchema { .. }
    ));
}

#[test]
fn explicit_validation_rejects_tampered_active_path_after_bounded_startup() {
    let (_directory, path) = database();
    let store = Store::open_at(&path, timestamp(10)).expect("initialize database");
    let chat = create_chat(&store, 20);
    let _started = begin_turn(&store, &chat, "hello", 30);
    drop(store);
    {
        let connection = Connection::open(&path).expect("tamper active path");
        connection
            .execute(
                "DELETE FROM active_path WHERE chat_id = ?1 AND position = 0",
                [chat.id.as_str()],
            )
            .expect("create active path gap");
    }

    let reopened = Store::open_at(&path, timestamp(100))
        .expect("normal startup does not scan the complete active path");
    drop(reopened);
    let error = Store::validate_snapshot_file(&path).expect_err("reject active path gap");
    assert!(matches!(
        error,
        StorageError::IncompatibleSchema {
            reason: "active path invariant failed"
        }
    ));
}

#[test]
fn explicit_validation_rejects_foreign_key_violation_after_bounded_startup() {
    let (_directory, path) = database();
    let store = Store::open_at(&path, timestamp(100)).expect("initialize database");
    drop(store);
    {
        let connection = Connection::open(&path).expect("tamper database");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 INSERT INTO messages(
                    id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 ) VALUES (
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    NULL, 1, 0, 1, 'user', 'complete', 'orphan', 100, 100, 100
                 );",
            )
            .expect("insert orphan with foreign keys disabled");
    }

    let reopened = Store::open_at(&path, timestamp(200))
        .expect("normal startup does not scan every foreign key");
    drop(reopened);
    let error = Store::validate_snapshot_file(&path).expect_err("reject orphan row");
    assert!(matches!(
        error,
        StorageError::IncompatibleSchema {
            reason: "foreign key check failed"
        }
    ));
}

fn seed_legacy_v1(path: &std::path::Path) {
    let connection = Connection::open(path).expect("create v1 database");
    connection
        .execute_batch(MIGRATION_V1)
        .expect("apply v1 schema");
    connection
        .execute(
            "INSERT INTO schema_meta(singleton, schema_version, migrated_at_ms)
             VALUES (1, 1, 90)",
            [],
        )
        .expect("insert v1 metadata");
    connection
        .execute(
            "INSERT INTO chats(
                id, character_id, title, revision, created_at_ms, updated_at_ms
             ) VALUES (?1, 'legacy-character', 'legacy chat', 1, 100, 130)",
            [LEGACY_CHAT_ID],
        )
        .expect("insert legacy chat");
    connection
        .execute(
            "INSERT INTO messages(
                id, chat_id, ordinal, role, status, text, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, 1, 'user', 'complete', 'legacy user', 110, 110)",
            [LEGACY_USER_MESSAGE_ID, LEGACY_CHAT_ID],
        )
        .expect("insert legacy user message");
    connection
        .execute(
            "INSERT INTO messages(
                id, chat_id, ordinal, role, status, text, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, 2, 'assistant', 'partial', 'legacy partial', 110, 130)",
            [LEGACY_ASSISTANT_MESSAGE_ID, LEGACY_CHAT_ID],
        )
        .expect("insert legacy assistant message");
    connection
        .execute(
            "INSERT INTO request_state(
                id, chat_id, user_message_id, assistant_message_id,
                provider_id, model_id, status, last_seq, provider_response_id,
                input_tokens, output_tokens, cached_input_tokens, reasoning_tokens,
                started_at_ms, updated_at_ms
             ) VALUES (
                ?1, ?2, ?3, ?4,
                'openai', 'legacy-model', 'running', 7, 'legacy-response',
                9, 7, 2, 1,
                110, 130
             )",
            [
                LEGACY_REQUEST_ID,
                LEGACY_CHAT_ID,
                LEGACY_USER_MESSAGE_ID,
                LEGACY_ASSISTANT_MESSAGE_ID,
            ],
        )
        .expect("insert legacy request state");
}

fn seed_legacy_v2(path: &std::path::Path) {
    seed_legacy_v1(path);
    let connection = Connection::open(path).expect("open v1 for v2 migration fixture");
    connection
        .execute_batch(MIGRATION_V2)
        .expect("apply exact v2 migration");
    connection
        .execute(
            "INSERT INTO schema_meta(singleton, schema_version, migrated_at_ms)
             VALUES (1, 2, 150)",
            [],
        )
        .expect("insert v2 metadata");
}

fn table_columns(connection: &Connection, table: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare table info");
    statement
        .query_map([], |row| row.get::<_, String>(1))
        .expect("read table info")
        .map(|row| row.expect("column name"))
        .collect()
}
