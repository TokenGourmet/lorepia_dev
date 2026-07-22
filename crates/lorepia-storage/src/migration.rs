use std::{fs, path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::{
    BUSY_TIMEOUT_MS, CURRENT_SCHEMA_VERSION, Result, StartupReport, StorageError, TimestampMillis,
};

const MIGRATION_V1: &str = include_str!("../migrations/0001_chat_persistence.sql");
const MIGRATION_V2: &str = include_str!("../migrations/0002_stream_journal.sql");
const MIGRATION_V3: &str = include_str!("../migrations/0003_branching_cache_wal.sql");

const REQUIRED_SCHEMA_OBJECTS_V1: &[(&str, &str)] = &[
    ("table", "schema_meta"),
    ("table", "chats"),
    ("table", "messages"),
    ("table", "request_state"),
    ("table", "settings"),
    ("table", "messages_fts"),
    ("index", "request_state_one_running_per_chat"),
    ("index", "request_state_chat_started"),
    ("trigger", "messages_fts_ai"),
    ("trigger", "messages_fts_ad"),
    ("trigger", "messages_fts_au"),
];

const REQUIRED_SCHEMA_OBJECTS_V2: &[(&str, &str)] = &[
    ("table", "schema_meta"),
    ("table", "chats"),
    ("table", "messages"),
    ("table", "request_state"),
    ("table", "settings"),
    ("table", "messages_fts"),
    ("index", "request_state_one_running_per_chat"),
    ("index", "request_state_chat_started"),
    ("index", "request_state_owner_running"),
    ("trigger", "messages_fts_ai"),
    ("trigger", "messages_fts_ad"),
    ("trigger", "messages_fts_au"),
];

const REQUIRED_SCHEMA_OBJECTS_V3: &[(&str, &str)] = &[
    ("table", "schema_meta"),
    ("table", "chats"),
    ("table", "messages"),
    ("table", "request_state"),
    ("table", "settings"),
    ("table", "active_path"),
    ("table", "message_render_cache"),
    ("table", "messages_fts"),
    ("index", "request_state_one_running_per_chat"),
    ("index", "request_state_chat_started"),
    ("index", "request_state_owner_running"),
    ("index", "messages_unique_child_sibling"),
    ("index", "messages_unique_root_sibling"),
    ("index", "messages_chat_parent_sibling"),
    ("index", "messages_chat_created"),
    ("index", "message_render_cache_lru"),
    ("trigger", "messages_parent_bi"),
    ("trigger", "messages_parent_bu"),
    ("trigger", "messages_fts_ai"),
    ("trigger", "messages_fts_ad"),
    ("trigger", "messages_fts_au_delete"),
    ("trigger", "messages_fts_au_insert"),
];

pub(crate) fn initialize_database(
    path: &Path,
    recovery_time: TimestampMillis,
) -> Result<StartupReport> {
    ensure_parent_directory(path)?;
    let database_existed = path.exists();
    let mut connection = open_for_initialization(path, database_existed)?;
    connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;

    let version = schema_version(&connection)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(StorageError::FutureSchema {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    if version < 0 {
        return Err(StorageError::IncompatibleSchema {
            reason: "schema version is negative",
        });
    }

    let migrated = version != CURRENT_SCHEMA_VERSION;
    match version {
        0 => {
            if database_existed {
                verify_integrity(&connection)?;
            }
            if database_existed && has_application_objects(&connection)? {
                return Err(StorageError::IncompatibleSchema {
                    reason: "unversioned database is not empty",
                });
            }
            apply_v1_migration(&mut connection, recovery_time)?;
            apply_v2_migration(&mut connection, recovery_time)?;
            apply_v3_migration(&mut connection, recovery_time)?;
        }
        1 => {
            verify_integrity(&connection)?;
            verify_schema(&connection, 1)?;
            apply_v2_migration(&mut connection, recovery_time)?;
            apply_v3_migration(&mut connection, recovery_time)?;
        }
        2 => {
            verify_integrity(&connection)?;
            verify_schema(&connection, 2)?;
            apply_v3_migration(&mut connection, recovery_time)?;
        }
        CURRENT_SCHEMA_VERSION => verify_schema_identity(&connection, CURRENT_SCHEMA_VERSION)?,
        _ => {
            return Err(StorageError::IncompatibleSchema {
                reason: "no migration path exists for the database version",
            });
        }
    }

    if migrated {
        verify_schema(&connection, CURRENT_SCHEMA_VERSION)?;
    }

    connection.pragma_update(None, "foreign_keys", "ON")?;
    let foreign_keys: i64 =
        connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    if foreign_keys != 1 {
        return Err(StorageError::IncompatibleSchema {
            reason: "SQLite foreign keys could not be enabled",
        });
    }

    let current_journal_mode: String =
        connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if !current_journal_mode.eq_ignore_ascii_case("wal") {
        let requested: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !requested.eq_ignore_ascii_case("wal") {
            return Err(StorageError::IncompatibleSchema {
                reason: "WAL journal mode is unavailable",
            });
        }
    }
    let journal_mode = connection
        .pragma_query_value::<String, _>(None, "journal_mode", |row| row.get(0))?
        .to_ascii_lowercase();
    if journal_mode != "wal" {
        return Err(StorageError::IncompatibleSchema {
            reason: "WAL journal mode did not persist",
        });
    }

    let recovered_request_count = recover_interrupted_requests(&mut connection, recovery_time)?;

    Ok(StartupReport {
        schema_version: CURRENT_SCHEMA_VERSION,
        journal_mode,
        busy_timeout_ms: BUSY_TIMEOUT_MS,
        recovered_request_count,
    })
}

pub(crate) fn open_operational_connection(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;

    let version = schema_version(&connection)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(StorageError::FutureSchema {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    if version != CURRENT_SCHEMA_VERSION {
        return Err(StorageError::IncompatibleSchema {
            reason: "database schema changed after startup",
        });
    }

    connection.pragma_update(None, "foreign_keys", "ON")?;
    let journal_mode: String =
        connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::IncompatibleSchema {
            reason: "database is no longer using WAL journal mode",
        });
    }
    Ok(connection)
}

pub(crate) fn validate_snapshot_file(path: &Path) -> Result<i64> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let version = schema_version(&connection)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(StorageError::FutureSchema {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    if !(1..=CURRENT_SCHEMA_VERSION).contains(&version) {
        return Err(StorageError::IncompatibleSchema {
            reason: "snapshot has no supported migration path",
        });
    }
    verify_integrity(&connection)?;
    verify_schema(&connection, version)?;
    Ok(version)
}

fn ensure_parent_directory(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        return Err(StorageError::InvalidInput {
            field: "database path",
            reason: "must name a database file",
        });
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(StorageError::PathUnavailable)?;
    }
    Ok(())
}

fn open_for_initialization(path: &Path, existed: bool) -> Result<Connection> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if !existed {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    Connection::open_with_flags(path, flags).map_err(StorageError::Database)
}

fn schema_version(connection: &Connection) -> Result<i64> {
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn verify_integrity(connection: &Connection) -> Result<()> {
    let result: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if result != "ok" {
        return Err(StorageError::IncompatibleSchema {
            reason: "SQLite integrity check failed",
        });
    }
    Ok(())
}

fn has_application_objects(connection: &Connection) -> Result<bool> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    Ok(count != 0)
}

fn apply_v1_migration(connection: &mut Connection, migrated_at_ms: TimestampMillis) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(MIGRATION_V1)?;
    transaction.execute(
        "INSERT INTO schema_meta(singleton, schema_version, migrated_at_ms) VALUES (1, 1, ?1)",
        params![migrated_at_ms.get()],
    )?;
    transaction.commit()?;
    Ok(())
}

fn apply_v2_migration(connection: &mut Connection, migrated_at_ms: TimestampMillis) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(MIGRATION_V2)?;
    transaction.execute(
        "INSERT INTO schema_meta(singleton, schema_version, migrated_at_ms) VALUES (1, 2, ?1)",
        params![migrated_at_ms.get()],
    )?;
    verify_schema(&transaction, 2)?;
    transaction.commit()?;
    Ok(())
}

fn apply_v3_migration(connection: &mut Connection, migrated_at_ms: TimestampMillis) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(MIGRATION_V3)?;
    transaction.execute(
        "INSERT INTO schema_meta(singleton, schema_version, migrated_at_ms) VALUES (1, 3, ?1)",
        params![migrated_at_ms.get()],
    )?;
    verify_schema(&transaction, 3)?;
    transaction.commit()?;
    Ok(())
}

fn verify_schema(connection: &Connection, expected_version: i64) -> Result<()> {
    verify_schema_identity(connection, expected_version)?;
    verify_foreign_keys(connection)?;
    if expected_version == 2 {
        verify_stream_journal_rows(connection)?;
    } else if expected_version == 3 {
        verify_stream_journal_rows(connection)?;
        verify_branch_rows(connection)?;
    }
    Ok(())
}

fn verify_schema_identity(connection: &Connection, expected_version: i64) -> Result<()> {
    let meta_version = connection
        .query_row(
            "SELECT schema_version FROM schema_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(StorageError::IncompatibleSchema {
            reason: "schema metadata is missing",
        })?;
    if meta_version != expected_version || schema_version(connection)? != meta_version {
        return Err(StorageError::IncompatibleSchema {
            reason: "schema metadata disagrees with SQLite user_version",
        });
    }

    let required_objects = match expected_version {
        1 => REQUIRED_SCHEMA_OBJECTS_V1,
        2 => REQUIRED_SCHEMA_OBJECTS_V2,
        3 => REQUIRED_SCHEMA_OBJECTS_V3,
        _ => {
            return Err(StorageError::IncompatibleSchema {
                reason: "no schema verifier exists for the database version",
            });
        }
    };
    for &(object_type, name) in required_objects {
        let exists = connection
            .query_row(
                "SELECT 1 FROM sqlite_schema WHERE type = ?1 AND name = ?2",
                params![object_type, name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(StorageError::IncompatibleSchema {
                reason: "a required schema object is missing",
            });
        }
    }
    verify_schema_definitions(connection, expected_version)?;
    let settings_rows: i64 = connection.query_row(
        "SELECT count(*) FROM settings WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if settings_rows != 1 {
        return Err(StorageError::IncompatibleSchema {
            reason: "settings singleton is missing",
        });
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct SchemaDefinition {
    object_type: String,
    name: String,
    table_name: String,
    sql: String,
}

fn verify_schema_definitions(connection: &Connection, version: i64) -> Result<()> {
    let expected_connection = Connection::open_in_memory()?;
    expected_connection.execute_batch(MIGRATION_V1)?;
    if version == 2 {
        expected_connection.execute_batch(MIGRATION_V2)?;
    } else if version == 3 {
        expected_connection.execute_batch(MIGRATION_V2)?;
        expected_connection.execute_batch(MIGRATION_V3)?;
    }
    let expected = schema_definitions(&expected_connection)?;
    let actual = schema_definitions(connection)?;
    if actual != expected {
        return Err(StorageError::IncompatibleSchema {
            reason: "schema definitions differ from the compiled migration",
        });
    }
    Ok(())
}

fn verify_branch_rows(connection: &Connection) -> Result<()> {
    let invalid_messages: i64 = connection.query_row(
        "SELECT count(*)
         FROM messages AS child
         LEFT JOIN messages AS parent
           ON parent.chat_id = child.chat_id AND parent.id = child.parent_id
         WHERE (child.parent_id IS NULL AND child.depth != 0)
            OR (child.parent_id IS NOT NULL AND (
                parent.id IS NULL OR parent.depth + 1 != child.depth
            ))
            OR (child.status = 'complete' AND child.completed_at_ms IS NULL)
            OR (child.status != 'complete' AND child.completed_at_ms IS NOT NULL)",
        [],
        |row| row.get(0),
    )?;
    if invalid_messages != 0 {
        return Err(StorageError::IncompatibleSchema {
            reason: "message branch invariant failed",
        });
    }

    let invalid_path_rows: i64 = connection.query_row(
        "SELECT count(*)
         FROM active_path AS current
         JOIN messages AS message
           ON message.chat_id = current.chat_id AND message.id = current.message_id
         LEFT JOIN active_path AS previous
           ON previous.chat_id = current.chat_id
          AND previous.position = current.position - 1
         WHERE message.depth != current.position
            OR (current.position = 0 AND message.parent_id IS NOT NULL)
            OR (current.position > 0 AND (
                previous.message_id IS NULL OR message.parent_id != previous.message_id
            ))",
        [],
        |row| row.get(0),
    )?;
    let path_gaps: i64 = connection.query_row(
        "SELECT count(*) FROM (
            SELECT chat_id
            FROM active_path
            GROUP BY chat_id
            HAVING min(position) != 0 OR count(*) != max(position) + 1
         )",
        [],
        |row| row.get(0),
    )?;
    let missing_paths: i64 = connection.query_row(
        "SELECT count(*)
         FROM chats AS chat
         WHERE EXISTS(SELECT 1 FROM messages WHERE chat_id = chat.id)
           AND NOT EXISTS(SELECT 1 FROM active_path WHERE chat_id = chat.id)",
        [],
        |row| row.get(0),
    )?;
    if invalid_path_rows != 0 || path_gaps != 0 || missing_paths != 0 {
        return Err(StorageError::IncompatibleSchema {
            reason: "active path invariant failed",
        });
    }
    Ok(())
}

fn verify_stream_journal_rows(connection: &Connection) -> Result<()> {
    let invalid_rows: i64 = connection.query_row(
        "SELECT count(*)
         FROM request_state
         WHERE length(CAST(owner_label AS BLOB)) NOT BETWEEN 1 AND 128
            OR owner_label GLOB '*[^A-Za-z0-9_:/-]*'
            OR length(stream_generation) != 32
            OR stream_generation GLOB '*[^0-9a-f]*'
            OR last_delivered_seq NOT BETWEEN 0 AND 9007199254740991
            OR last_durable_seq NOT BETWEEN 0 AND 9007199254740991
            OR (last_acked_seq IS NOT NULL AND last_acked_seq NOT BETWEEN 1 AND 9007199254740991)
            OR (last_acked_seq IS NOT NULL AND last_acked_seq > last_durable_seq)
            OR last_durable_seq > last_delivered_seq",
        [],
        |row| row.get(0),
    )?;
    if invalid_rows != 0 {
        return Err(StorageError::IncompatibleSchema {
            reason: "stream journal sequence or identity invariant failed",
        });
    }
    Ok(())
}

fn schema_definitions(connection: &Connection) -> Result<Vec<SchemaDefinition>> {
    let mut statement = connection.prepare(
        "SELECT type, name, tbl_name, sql
         FROM sqlite_schema
         WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL
         ORDER BY type, name",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(SchemaDefinition {
            object_type: row.get(0)?,
            name: row.get(1)?,
            table_name: row.get(2)?,
            sql: row.get(3)?,
        })
    })?;
    let mut definitions = Vec::new();
    for row in rows {
        definitions.push(row?);
    }
    Ok(definitions)
}

fn verify_foreign_keys(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if rows.next()?.is_some() {
        return Err(StorageError::IncompatibleSchema {
            reason: "foreign key check failed",
        });
    }
    Ok(())
}

fn recover_interrupted_requests(
    connection: &mut Connection,
    recovered_at_ms: TimestampMillis,
) -> Result<u64> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "UPDATE chats
         SET updated_at_ms = max(updated_at_ms, ?1)
         WHERE id IN (SELECT chat_id FROM request_state WHERE status = 'running')",
        params![recovered_at_ms.get()],
    )?;
    transaction.execute(
        "UPDATE messages
         SET status = 'partial',
             updated_at_ms = max(updated_at_ms, ?1)
         WHERE id IN (
               SELECT assistant_message_id
               FROM request_state
               WHERE status = 'running'
           )",
        params![recovered_at_ms.get()],
    )?;
    let recovered = transaction.execute(
        "UPDATE request_state
         SET status = 'interrupted',
             failure_code = 'APP_RESTARTED',
             updated_at_ms = max(updated_at_ms, ?1),
             finished_at_ms = max(updated_at_ms, started_at_ms, ?1)
         WHERE status = 'running'",
        params![recovered_at_ms.get()],
    )?;
    transaction.commit()?;
    u64::try_from(recovered).map_err(|_| StorageError::IncompatibleSchema {
        reason: "recovery count exceeds the supported range",
    })
}
