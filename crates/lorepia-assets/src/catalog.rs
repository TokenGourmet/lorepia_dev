use std::{fs, io::ErrorKind, path::Path, str::FromStr, time::Duration};

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::{AssetError, AssetHash, AssetMime, AssetObject, AssetState, AssetStats, Result};

pub(crate) const CATALOG_FILE_NAME: &str = "assets.sqlite3";
const LEGACY_SCHEMA_VERSION: i64 = 1;
const QUARANTINE_INTENT_SCHEMA_VERSION: i64 = 2;
const BACKUP_SESSION_SCHEMA_VERSION: i64 = 3;
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 4;

pub(crate) fn initialize(path: &Path) -> Result<()> {
    let mut connection = connect(path)?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match version {
        0 => create_schema(&mut connection)?,
        LEGACY_SCHEMA_VERSION => {
            verify_schema(&connection, LEGACY_SCHEMA_VERSION)?;
            verify_derived_totals(&connection)?;
            migrate_v1_to_v2(&mut connection)?;
            migrate_v2_to_v3(&mut connection)?;
            migrate_v3_to_v4(&mut connection)?;
        }
        QUARANTINE_INTENT_SCHEMA_VERSION => {
            verify_schema(&connection, QUARANTINE_INTENT_SCHEMA_VERSION)?;
            verify_derived_totals(&connection)?;
            migrate_v2_to_v3(&mut connection)?;
            migrate_v3_to_v4(&mut connection)?;
        }
        BACKUP_SESSION_SCHEMA_VERSION => {
            verify_schema(&connection, BACKUP_SESSION_SCHEMA_VERSION)?;
            verify_derived_totals(&connection)?;
            migrate_v3_to_v4(&mut connection)?;
        }
        CURRENT_SCHEMA_VERSION => {}
        found => Err(AssetError::SchemaVersion {
            found,
            supported: CURRENT_SCHEMA_VERSION,
        })?,
    };
    verify_schema(&connection, CURRENT_SCHEMA_VERSION)?;
    verify_derived_totals(&connection)?;
    Ok(())
}

pub(crate) fn connect(path: &Path) -> Result<Connection> {
    reject_reparse_if_present(path)?;
    reject_auxiliary_reparse_points(path)?;
    let open_path = normalized_nofollow_path(path)?;
    let connection = Connection::open_with_flags(
        open_path,
        rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(connection)
}

pub(crate) fn validate_snapshot_file(path: &Path) -> Result<i64> {
    reject_reparse_if_present(path)?;
    reject_auxiliary_reparse_points(path)?;
    let open_path = normalized_nofollow_path(path)?;
    let connection = Connection::open_with_flags(
        open_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if !matches!(
        version,
        LEGACY_SCHEMA_VERSION
            | QUARANTINE_INTENT_SCHEMA_VERSION
            | BACKUP_SESSION_SCHEMA_VERSION
            | CURRENT_SCHEMA_VERSION
    ) {
        return Err(AssetError::SchemaVersion {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    let quick: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if quick != "ok" {
        return Err(AssetError::IncompatibleCatalog {
            reason: "SQLite quick_check failed",
        });
    }
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(AssetError::IncompatibleCatalog {
            reason: "SQLite integrity_check failed",
        });
    }
    verify_schema(&connection, version)?;
    verify_derived_totals(&connection)?;
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if rows.next()?.is_some() {
        return Err(AssetError::IncompatibleCatalog {
            reason: "foreign key check failed",
        });
    }
    Ok(version)
}

fn normalized_nofollow_path(path: &Path) -> Result<std::path::PathBuf> {
    let parent = path.parent().ok_or(AssetError::InvalidInput {
        field: "catalog path",
        reason: "must have a parent directory",
    })?;
    let name = path.file_name().ok_or(AssetError::InvalidInput {
        field: "catalog path",
        reason: "must end in a filename",
    })?;
    Ok(fs::canonicalize(parent)?.join(name))
}

fn reject_reparse_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) =>
        {
            Err(AssetError::UnsafeFilesystem {
                path: path.display().to_string(),
                reason: "catalog path is a symlink or reparse point".to_owned(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reject_auxiliary_reparse_points(path: &Path) -> Result<()> {
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        reject_reparse_if_present(Path::new(&name))?;
    }
    Ok(())
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

fn create_schema(connection: &mut Connection) -> Result<()> {
    create_schema_v1(connection)?;
    migrate_v1_to_v2(connection)?;
    migrate_v2_to_v3(connection)?;
    migrate_v3_to_v4(connection)
}

fn create_schema_v1(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE asset_objects (
            hash TEXT PRIMARY KEY NOT NULL
                CHECK(length(hash) = 64 AND hash NOT GLOB '*[^0-9a-f]*'),
            size INTEGER NOT NULL CHECK(size >= 0),
            mime TEXT NOT NULL CHECK(mime IN (
                'image/png', 'image/jpeg', 'image/webp', 'image/gif',
                'audio/wav', 'audio/mpeg', 'audio/ogg', 'audio/flac'
            )),
            relative_path TEXT NOT NULL UNIQUE,
            state TEXT NOT NULL CHECK(state IN ('active', 'missing', 'quarantined')),
            quarantine_name TEXT,
            verified_at_ms INTEGER NOT NULL CHECK(verified_at_ms >= 0),
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
            CHECK(
                (state = 'quarantined' AND quarantine_name IS NOT NULL)
                OR (state != 'quarantined' AND quarantine_name IS NULL)
            )
        ) STRICT;

        CREATE TABLE asset_refs (
            owner_type TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            hash TEXT NOT NULL REFERENCES asset_objects(hash) ON DELETE CASCADE,
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
            PRIMARY KEY(owner_type, owner_id, hash)
        ) WITHOUT ROWID, STRICT;

        CREATE INDEX idx_asset_refs_hash ON asset_refs(hash);

        CREATE TABLE asset_staging (
            name TEXT PRIMARY KEY NOT NULL,
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
        ) WITHOUT ROWID, STRICT;

        CREATE TABLE asset_quarantine (
            name TEXT PRIMARY KEY NOT NULL,
            original_hash TEXT,
            reason TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
        ) WITHOUT ROWID, STRICT;

        CREATE TABLE asset_totals (
            id INTEGER PRIMARY KEY NOT NULL CHECK(id = 1),
            object_count INTEGER NOT NULL CHECK(object_count >= 0),
            active_bytes INTEGER NOT NULL CHECK(active_bytes >= 0),
            reference_count INTEGER NOT NULL CHECK(reference_count >= 0),
            missing_count INTEGER NOT NULL CHECK(missing_count >= 0),
            quarantine_count INTEGER NOT NULL CHECK(quarantine_count >= 0),
            staging_count INTEGER NOT NULL CHECK(staging_count >= 0)
        ) STRICT;

        INSERT INTO asset_totals(
            id, object_count, active_bytes, reference_count,
            missing_count, quarantine_count, staging_count
        ) VALUES (1, 0, 0, 0, 0, 0, 0);

        CREATE TRIGGER asset_objects_totals_insert
        AFTER INSERT ON asset_objects BEGIN
            UPDATE asset_totals SET
                object_count = object_count + 1,
                active_bytes = active_bytes
                    + CASE WHEN NEW.state = 'active' THEN NEW.size ELSE 0 END,
                missing_count = missing_count
                    + CASE WHEN NEW.state = 'missing' THEN 1 ELSE 0 END
            WHERE id = 1;
        END;

        CREATE TRIGGER asset_objects_totals_update
        AFTER UPDATE OF size, state ON asset_objects BEGIN
            UPDATE asset_totals SET
                active_bytes = active_bytes
                    - CASE WHEN OLD.state = 'active' THEN OLD.size ELSE 0 END
                    + CASE WHEN NEW.state = 'active' THEN NEW.size ELSE 0 END,
                missing_count = missing_count
                    - CASE WHEN OLD.state = 'missing' THEN 1 ELSE 0 END
                    + CASE WHEN NEW.state = 'missing' THEN 1 ELSE 0 END
            WHERE id = 1;
        END;

        CREATE TRIGGER asset_objects_totals_delete
        AFTER DELETE ON asset_objects BEGIN
            UPDATE asset_totals SET
                object_count = object_count - 1,
                active_bytes = active_bytes
                    - CASE WHEN OLD.state = 'active' THEN OLD.size ELSE 0 END,
                missing_count = missing_count
                    - CASE WHEN OLD.state = 'missing' THEN 1 ELSE 0 END
            WHERE id = 1;
        END;

        CREATE TRIGGER asset_refs_totals_insert
        AFTER INSERT ON asset_refs BEGIN
            UPDATE asset_totals SET reference_count = reference_count + 1 WHERE id = 1;
        END;

        CREATE TRIGGER asset_refs_totals_delete
        AFTER DELETE ON asset_refs BEGIN
            UPDATE asset_totals SET reference_count = reference_count - 1 WHERE id = 1;
        END;

        CREATE TRIGGER asset_staging_totals_insert
        AFTER INSERT ON asset_staging BEGIN
            UPDATE asset_totals SET staging_count = staging_count + 1 WHERE id = 1;
        END;

        CREATE TRIGGER asset_staging_totals_delete
        AFTER DELETE ON asset_staging BEGIN
            UPDATE asset_totals SET staging_count = staging_count - 1 WHERE id = 1;
        END;

        CREATE TRIGGER asset_quarantine_totals_insert
        AFTER INSERT ON asset_quarantine BEGIN
            UPDATE asset_totals SET quarantine_count = quarantine_count + 1 WHERE id = 1;
        END;

        CREATE TRIGGER asset_quarantine_totals_delete
        AFTER DELETE ON asset_quarantine BEGIN
            UPDATE asset_totals SET quarantine_count = quarantine_count - 1 WHERE id = 1;
        END;

        PRAGMA user_version = 1;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn migrate_v1_to_v2(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE asset_quarantine_intents (
            name TEXT PRIMARY KEY NOT NULL
                CHECK(length(name) BETWEEN 1 AND 160)
                CHECK(name NOT GLOB '*[^a-z0-9.-]*'),
            operation TEXT NOT NULL CHECK(operation IN ('move', 'purge')),
            phase TEXT NOT NULL CHECK(phase IN ('prepared', 'moved')),
            source_relative_path TEXT,
            original_hash TEXT
                CHECK(original_hash IS NULL OR (
                    length(original_hash) = 64
                    AND original_hash NOT GLOB '*[^0-9a-f]*'
                )),
            reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 256),
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
            CHECK(
                (operation = 'move' AND source_relative_path IS NOT NULL
                    AND length(source_relative_path) BETWEEN 1 AND 256)
                OR (operation = 'purge' AND source_relative_path IS NULL)
            )
        ) WITHOUT ROWID, STRICT;

        PRAGMA user_version = 2;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn migrate_v2_to_v3(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        // v1/v2 represented export pins only as generic references. There is no durable lease
        // proving those legacy sessions are still resumable, so migration must fail closed by
        // releasing them. The existing delete trigger keeps asset_totals authoritative.
        "DELETE FROM asset_refs WHERE owner_type = 'lorepia-backup';

        CREATE TABLE asset_backup_sessions (
            session_id TEXT PRIMARY KEY NOT NULL
                CHECK(length(session_id) = 32)
                CHECK(session_id NOT GLOB '*[^0-9a-f]*'),
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
            lease_updated_at_ms INTEGER NOT NULL CHECK(lease_updated_at_ms >= created_at_ms)
        ) WITHOUT ROWID, STRICT;

        CREATE INDEX idx_asset_backup_sessions_lease
            ON asset_backup_sessions(lease_updated_at_ms, session_id);

        PRAGMA user_version = 3;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn migrate_v3_to_v4(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE asset_temporary_owner_sessions (
            owner_type TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
            lease_updated_at_ms INTEGER NOT NULL CHECK(lease_updated_at_ms >= created_at_ms),
            PRIMARY KEY(owner_type, owner_id)
        ) WITHOUT ROWID, STRICT;

        CREATE INDEX idx_asset_temporary_owner_sessions_lease
            ON asset_temporary_owner_sessions(owner_type, lease_updated_at_ms, owner_id);

        -- v3 import references had no durable liveness record. Converting them to an already
        -- expired lease lets AssetStore::open remove both the catalog rows and object files.
        INSERT INTO asset_temporary_owner_sessions(
            owner_type, owner_id, created_at_ms, lease_updated_at_ms
        )
        SELECT DISTINCT owner_type, owner_id, 0, 0
        FROM asset_refs WHERE owner_type = 'lorepia-import-session';

        PRAGMA user_version = 4;",
    )?;
    transaction.commit()?;
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct SchemaEntry {
    kind: String,
    name: String,
    table_name: String,
    sql: String,
}

fn verify_schema(connection: &Connection, version: i64) -> Result<()> {
    let actual = schema_entries(connection)?;
    let mut reference = Connection::open_in_memory()?;
    create_schema_v1(&mut reference)?;
    if version >= QUARANTINE_INTENT_SCHEMA_VERSION {
        migrate_v1_to_v2(&mut reference)?;
    }
    if version >= BACKUP_SESSION_SCHEMA_VERSION {
        migrate_v2_to_v3(&mut reference)?;
    }
    if version >= CURRENT_SCHEMA_VERSION {
        migrate_v3_to_v4(&mut reference)?;
    }
    let expected = schema_entries(&reference)?;
    if actual != expected {
        return Err(AssetError::IncompatibleCatalog {
            reason: "catalog schema fingerprint does not match the supported schema",
        });
    }
    Ok(())
}

fn schema_entries(connection: &Connection) -> Result<Vec<SchemaEntry>> {
    let mut statement = connection.prepare(
        "SELECT type, name, tbl_name, coalesce(sql, '')
         FROM sqlite_schema
         ORDER BY type, name, tbl_name, coalesce(sql, '')",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(SchemaEntry {
            kind: row.get(0)?,
            name: row.get(1)?,
            table_name: row.get(2)?,
            sql: row.get(3)?,
        })
    })?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

pub(crate) fn verify_derived_totals(connection: &Connection) -> Result<AssetStats> {
    let recorded = read_recorded_totals(connection)?;
    let computed = connection.query_row(
        "SELECT
            (SELECT count(*) FROM asset_objects),
            (SELECT coalesce(sum(size), 0) FROM asset_objects WHERE state = 'active'),
            (SELECT count(*) FROM asset_refs),
            (SELECT count(*) FROM asset_objects WHERE state = 'missing'),
            (SELECT count(*) FROM asset_quarantine),
            (SELECT count(*) FROM asset_staging)",
        [],
        decode_totals,
    )?;
    if recorded != computed {
        return Err(AssetError::IncompatibleCatalog {
            reason: "quota ledger does not match catalog contents",
        });
    }
    Ok(recorded)
}

fn read_recorded_totals(connection: &Connection) -> Result<AssetStats> {
    let mut statement = connection.prepare(
        "SELECT object_count, active_bytes, reference_count,
                missing_count, quarantine_count, staging_count
         FROM asset_totals WHERE id = 1",
    )?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Err(AssetError::IncompatibleCatalog {
            reason: "quota ledger singleton is missing or invalid",
        });
    };
    let totals = decode_totals(row)?;
    if rows.next()?.is_some() {
        return Err(AssetError::IncompatibleCatalog {
            reason: "quota ledger singleton is missing or invalid",
        });
    }
    Ok(totals)
}

fn decode_totals(row: &Row<'_>) -> rusqlite::Result<AssetStats> {
    let values = [
        row.get::<_, i64>(0)?,
        row.get::<_, i64>(1)?,
        row.get::<_, i64>(2)?,
        row.get::<_, i64>(3)?,
        row.get::<_, i64>(4)?,
        row.get::<_, i64>(5)?,
    ];
    if values.iter().any(|value| *value < 0) {
        return Err(rusqlite::Error::IntegralValueOutOfRange(0, values[0]));
    }
    Ok(AssetStats {
        object_count: values[0] as u64,
        active_bytes: values[1] as u64,
        reference_count: values[2] as u64,
        missing_count: values[3] as u64,
        quarantined_count: values[4] as u64,
        staging_count: values[5] as u64,
    })
}

pub(crate) fn get_object(connection: &Connection, hash: &AssetHash) -> Result<Option<AssetObject>> {
    connection
        .query_row(
            "SELECT hash, size, mime, relative_path, state, verified_at_ms, created_at_ms
             FROM asset_objects WHERE hash = ?1",
            params![hash.as_str()],
            decode_object,
        )
        .optional()?
        .map(Ok)
        .transpose()
}

pub(crate) fn decode_object(row: &Row<'_>) -> rusqlite::Result<AssetObject> {
    let hash: String = row.get(0)?;
    let size: i64 = row.get(1)?;
    let mime: String = row.get(2)?;
    let relative_path: String = row.get(3)?;
    let state: String = row.get(4)?;
    let verified_at_ms: i64 = row.get(5)?;
    let created_at_ms: i64 = row.get(6)?;

    let decoded = (|| {
        Ok(AssetObject {
            hash: AssetHash::parse(hash)?,
            size: u64::try_from(size).map_err(|_| AssetError::IncompatibleCatalog {
                reason: "asset size is negative",
            })?,
            mime: AssetMime::from_str(&mime)?,
            relative_path,
            state: AssetState::parse(&state)?,
            verified_at_ms,
            created_at_ms,
        })
    })();

    decoded.map_err(|error: AssetError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}
