use std::{fs, path::Path};

use lorepia_assets::{AssetHash, AssetStore, CURRENT_ASSET_SCHEMA_VERSION};
use lorepia_storage::{CURRENT_SCHEMA_VERSION, Store};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::{BackupError, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CatalogObject {
    pub hash: AssetHash,
    pub size: u64,
    pub relative_path: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CatalogSummary {
    pub object_count: u64,
    pub total_bytes: u64,
}

pub(crate) fn database_version(path: &Path) -> Result<i64> {
    let connection = read_only(path)?;
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

pub(crate) fn validate_product_database(path: &Path) -> Result<i64> {
    let version = validate_database(
        path,
        "product",
        CURRENT_SCHEMA_VERSION,
        &["chats", "messages", "request_state", "settings"],
    )?;
    let exact = Store::validate_snapshot_file(path)?;
    if exact != version {
        return Err(BackupError::InvalidDatabase {
            database: "product",
            reason: "exact schema verifier disagrees with user_version",
        });
    }
    Ok(version)
}

pub(crate) fn validate_asset_catalog(path: &Path) -> Result<i64> {
    // The asset verifier opens with SQLITE_OPEN_NOFOLLOW. Canonicalizing an already-created
    // snapshot preserves that contract while avoiding platform temp roots such as macOS `/var`
    // whose ancestor is itself a system symlink to `/private/var`.
    let canonical = fs::canonicalize(path)?;
    let version = validate_database(
        &canonical,
        "asset catalog",
        CURRENT_ASSET_SCHEMA_VERSION,
        &["asset_objects", "asset_refs", "asset_totals"],
    )?;
    let exact = AssetStore::validate_catalog_snapshot_file(&canonical)?;
    if exact != version {
        return Err(BackupError::InvalidDatabase {
            database: "asset catalog",
            reason: "catalog verifier disagrees with user_version",
        });
    }
    Ok(version)
}

fn validate_database(
    path: &Path,
    label: &'static str,
    supported: i64,
    required_tables: &[&str],
) -> Result<i64> {
    let connection = read_only(path)?;
    let quick: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick != "ok" {
        return Err(BackupError::InvalidDatabase {
            database: label,
            reason: "quick_check did not return ok",
        });
    }
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(BackupError::InvalidDatabase {
            database: label,
            reason: "integrity_check did not return ok",
        });
    }
    let foreign_key_violation: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if foreign_key_violation.is_some() {
        return Err(BackupError::InvalidDatabase {
            database: label,
            reason: "foreign_key_check reported a violation",
        });
    }
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > supported {
        return Err(BackupError::InvalidDatabase {
            database: label,
            reason: "schema version is newer than this build",
        });
    }
    if version <= 0 {
        return Err(BackupError::InvalidDatabase {
            database: label,
            reason: "schema version is missing",
        });
    }
    for table in required_tables {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
             )",
            params![table],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(BackupError::InvalidDatabase {
                database: label,
                reason: "a required schema table is missing",
            });
        }
    }
    Ok(version)
}

pub(crate) fn catalog_objects_after(
    path: &Path,
    after: Option<&str>,
    limit: u16,
) -> Result<Vec<CatalogObject>> {
    let connection = read_only(path)?;
    let mut objects = Vec::new();
    if let Some(after) = after {
        let mut statement = connection.prepare(
            "SELECT hash, size, relative_path FROM asset_objects
             WHERE state = 'active' AND hash > ?1 ORDER BY hash LIMIT ?2",
        )?;
        let rows = statement.query_map(params![after, i64::from(limit)], decode_catalog_object)?;
        for row in rows {
            objects.push(row?);
        }
    } else {
        let mut statement = connection.prepare(
            "SELECT hash, size, relative_path FROM asset_objects
             WHERE state = 'active' ORDER BY hash LIMIT ?1",
        )?;
        let rows = statement.query_map(params![i64::from(limit)], decode_catalog_object)?;
        for row in rows {
            objects.push(row?);
        }
    }
    Ok(objects)
}

pub(crate) fn catalog_summary(path: &Path) -> Result<CatalogSummary> {
    let connection = read_only(path)?;
    let (count, bytes): (i64, i64) = connection.query_row(
        "SELECT count(*), COALESCE(sum(size), 0)
         FROM asset_objects WHERE state = 'active'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(CatalogSummary {
        object_count: u64::try_from(count).map_err(|_| BackupError::InvalidDatabase {
            database: "asset catalog",
            reason: "active object count is negative",
        })?,
        total_bytes: u64::try_from(bytes).map_err(|_| BackupError::InvalidDatabase {
            database: "asset catalog",
            reason: "active object byte total is negative or overflowed",
        })?,
    })
}

fn decode_catalog_object(row: &rusqlite::Row<'_>) -> rusqlite::Result<CatalogObject> {
    let hash: String = row.get(0)?;
    let size: i64 = row.get(1)?;
    let relative_path: String = row.get(2)?;
    let decoded = (|| {
        Ok(CatalogObject {
            hash: AssetHash::parse(hash).map_err(|_| BackupError::InvalidDatabase {
                database: "asset catalog",
                reason: "object hash is invalid",
            })?,
            size: u64::try_from(size).map_err(|_| BackupError::InvalidDatabase {
                database: "asset catalog",
                reason: "object size is negative",
            })?,
            relative_path,
        })
    })();
    decoded.map_err(|error: BackupError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn read_only(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}
