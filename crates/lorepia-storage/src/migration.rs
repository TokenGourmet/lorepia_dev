use std::{fs, path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::{
    BUSY_TIMEOUT_MS, CURRENT_SCHEMA_VERSION, Result, StartupReport, StorageError, TimestampMillis,
};

const MIGRATION_V1: &str = include_str!("../migrations/0001_chat_persistence.sql");

const REQUIRED_SCHEMA_OBJECTS: &[(&str, &str)] = &[
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

    verify_integrity(&connection)?;
    match version {
        0 => {
            if database_existed && has_application_objects(&connection)? {
                return Err(StorageError::IncompatibleSchema {
                    reason: "unversioned database is not empty",
                });
            }
            apply_v1_migration(&mut connection, recovery_time)?;
        }
        CURRENT_SCHEMA_VERSION => {}
        _ => {
            return Err(StorageError::IncompatibleSchema {
                reason: "no migration path exists for the database version",
            });
        }
    }

    verify_current_schema(&connection)?;

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

fn verify_current_schema(connection: &Connection) -> Result<()> {
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
    if meta_version != CURRENT_SCHEMA_VERSION || schema_version(connection)? != meta_version {
        return Err(StorageError::IncompatibleSchema {
            reason: "schema metadata disagrees with SQLite user_version",
        });
    }

    for &(object_type, name) in REQUIRED_SCHEMA_OBJECTS {
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
    verify_schema_definitions(connection)?;
    verify_foreign_keys(connection)?;
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

fn verify_schema_definitions(connection: &Connection) -> Result<()> {
    let expected_connection = Connection::open_in_memory()?;
    expected_connection.execute_batch(MIGRATION_V1)?;
    let expected = schema_definitions(&expected_connection)?;
    let actual = schema_definitions(connection)?;
    if actual != expected {
        return Err(StorageError::IncompatibleSchema {
            reason: "schema definitions differ from the compiled migration",
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
