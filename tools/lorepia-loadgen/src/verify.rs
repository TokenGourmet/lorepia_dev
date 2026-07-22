use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use lorepia_assets::{AssetLimits, AssetStats, AssetStore, MAX_PAGE_SIZE};
use lorepia_storage::CURRENT_SCHEMA_VERSION;
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::util::{
    Result, canonical_existing_dir, canonical_existing_file, checked_sum_file_sizes, emit_receipt,
    invalid, sqlite_sidecars,
};

#[derive(Debug)]
pub struct VerifyOptions {
    pub database: PathBuf,
    pub objects: Option<PathBuf>,
    pub full: bool,
    pub output: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseVerification {
    pub path: String,
    pub schema_version: i64,
    pub expected_schema_version: i64,
    pub sqlite_check: &'static str,
    pub sqlite_check_result: String,
    pub foreign_key_violations: u64,
    pub message_count: u64,
    pub message_text_bytes: u64,
    pub complete_message_count: u64,
    pub fts_docsize_rows: u64,
    pub fts_missing_rows: u64,
    pub fts_orphan_rows: u64,
    pub fts_integrity_command_run: bool,
    pub active_path_rows: u64,
    pub active_path_invalid_rows: u64,
    pub active_path_gap_chats: u64,
    pub chats_missing_active_path: u64,
    pub invalid_stream_journal_rows: u64,
    pub database_and_sidecar_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetVerification {
    pub root: String,
    pub object_count: u64,
    pub active_bytes: u64,
    pub reference_count: u64,
    pub missing_count: u64,
    pub quarantined_count: u64,
    pub catalog_staging_count: u64,
    pub filesystem_staging_entries: u64,
    pub listed_objects: u64,
    pub full_hashes_checked: u64,
    pub filesystem_object_bytes: u64,
    pub untracked_object_files: u64,
    pub missing_object_files: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReceipt {
    pub artifact_kind: &'static str,
    pub tool_version: &'static str,
    pub full: bool,
    pub passed: bool,
    pub offset_queries_used: bool,
    pub database: DatabaseVerification,
    pub assets: Option<AssetVerification>,
    pub issues: Vec<String>,
}

pub fn run(options: VerifyOptions) -> Result<()> {
    let database_path = canonical_existing_file(&options.database)?;
    let (database, mut issues) = verify_database(&database_path, options.full)?;
    let assets = if let Some(objects) = options.objects.as_deref() {
        let root = canonical_existing_dir(objects)?;
        let (verification, asset_issues) = verify_assets(&root, options.full)?;
        issues.extend(asset_issues);
        Some(verification)
    } else {
        None
    };
    let passed = issues.is_empty();
    let receipt = VerifyReceipt {
        artifact_kind: "LOREPIA_STORAGE_VERIFICATION_RECEIPT",
        tool_version: env!("CARGO_PKG_VERSION"),
        full: options.full,
        passed,
        offset_queries_used: false,
        database,
        assets,
        issues,
    };
    emit_receipt(options.output.as_deref(), &receipt)?;
    if passed {
        Ok(())
    } else {
        Err(invalid("verification failed; see the emitted receipt"))
    }
}

pub fn verify_database(path: &Path, full: bool) -> Result<(DatabaseVerification, Vec<String>)> {
    let mut connection = Connection::open(path)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let schema_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema_version != CURRENT_SCHEMA_VERSION {
        return Err(invalid(format!(
            "database schema {schema_version} is not CURRENT_SCHEMA_VERSION {CURRENT_SCHEMA_VERSION}"
        )));
    }
    let meta_version: i64 = connection.query_row(
        "SELECT schema_version FROM schema_meta WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if meta_version != schema_version {
        return Err(invalid("schema_meta and PRAGMA user_version disagree"));
    }

    let check_name = if full {
        "integrity_check"
    } else {
        "quick_check"
    };
    let check_sql = if full {
        "PRAGMA integrity_check"
    } else {
        "PRAGMA quick_check"
    };
    let mut statement = connection.prepare(check_sql)?;
    let check_rows = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    let check_result = check_rows.join("; ");

    let foreign_key_violations =
        query_u64(&connection, "SELECT count(*) FROM pragma_foreign_key_check")?;
    let message_count = query_u64(&connection, "SELECT count(*) FROM messages")?;
    let message_text_bytes = query_u64(
        &connection,
        "SELECT coalesce(sum(length(CAST(text AS BLOB))), 0) FROM messages",
    )?;
    let complete_message_count = query_u64(
        &connection,
        "SELECT count(*) FROM messages WHERE status = 'complete'",
    )?;
    let fts_docsize_rows = query_u64(&connection, "SELECT count(*) FROM messages_fts_docsize")?;
    let fts_missing_rows = query_u64(
        &connection,
        "SELECT count(*)
         FROM messages AS message
         LEFT JOIN messages_fts_docsize AS indexed ON indexed.id = message.row_id
         WHERE message.status = 'complete' AND indexed.id IS NULL",
    )?;
    let fts_orphan_rows = query_u64(
        &connection,
        "SELECT count(*)
         FROM messages_fts_docsize AS indexed
         LEFT JOIN messages AS message ON message.row_id = indexed.id
         WHERE message.row_id IS NULL OR message.status != 'complete'",
    )?;
    let active_path_rows = query_u64(&connection, "SELECT count(*) FROM active_path")?;
    let active_path_invalid_rows = query_u64(
        &connection,
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
    )?;
    let active_path_gap_chats = query_u64(
        &connection,
        "SELECT count(*) FROM (
            SELECT chat_id
            FROM active_path
            GROUP BY chat_id
            HAVING min(position) != 0 OR count(*) != max(position) + 1
         )",
    )?;
    let chats_missing_active_path = query_u64(
        &connection,
        "SELECT count(*)
         FROM chats AS chat
         WHERE EXISTS(SELECT 1 FROM messages WHERE chat_id = chat.id)
           AND NOT EXISTS(SELECT 1 FROM active_path WHERE chat_id = chat.id)",
    )?;
    let invalid_stream_journal_rows = query_u64(
        &connection,
        "SELECT count(*)
         FROM request_state
         WHERE last_delivered_seq < last_durable_seq
            OR last_durable_seq < coalesce(last_acked_seq, 0)
            OR length(stream_generation) != 32
            OR stream_generation GLOB '*[^0-9a-f]*'
            OR length(CAST(owner_label AS BLOB)) NOT BETWEEN 1 AND 128",
    )?;

    let mut fts_integrity_command_run = false;
    if full {
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1)",
            [],
        )?;
        transaction.rollback()?;
        fts_integrity_command_run = true;
    }

    let database_and_sidecar_bytes = checked_sum_file_sizes(&sqlite_sidecars(path))?;
    let mut issues = Vec::new();
    if check_rows.as_slice() != ["ok"] {
        issues.push(format!("SQLite {check_name} failed: {check_result}"));
    }
    append_nonzero(
        &mut issues,
        foreign_key_violations,
        "foreign-key violations",
    );
    if fts_docsize_rows != complete_message_count {
        issues.push(format!(
            "FTS row count {fts_docsize_rows} differs from complete message count {complete_message_count}"
        ));
    }
    append_nonzero(
        &mut issues,
        fts_missing_rows,
        "complete messages missing from FTS",
    );
    append_nonzero(&mut issues, fts_orphan_rows, "orphan/non-terminal FTS rows");
    append_nonzero(
        &mut issues,
        active_path_invalid_rows,
        "invalid active_path rows",
    );
    append_nonzero(&mut issues, active_path_gap_chats, "active_path gap chats");
    append_nonzero(
        &mut issues,
        chats_missing_active_path,
        "chats with messages but no active_path",
    );
    append_nonzero(
        &mut issues,
        invalid_stream_journal_rows,
        "invalid stream journal rows",
    );

    Ok((
        DatabaseVerification {
            path: path.display().to_string(),
            schema_version,
            expected_schema_version: CURRENT_SCHEMA_VERSION,
            sqlite_check: check_name,
            sqlite_check_result: check_result,
            foreign_key_violations,
            message_count,
            message_text_bytes,
            complete_message_count,
            fts_docsize_rows,
            fts_missing_rows,
            fts_orphan_rows,
            fts_integrity_command_run,
            active_path_rows,
            active_path_invalid_rows,
            active_path_gap_chats,
            chats_missing_active_path,
            invalid_stream_journal_rows,
            database_and_sidecar_bytes,
        },
        issues,
    ))
}

fn verify_assets(root: &Path, full: bool) -> Result<(AssetVerification, Vec<String>)> {
    for required in ["assets.sqlite3", "objects", ".staging", "quarantine"] {
        if !root.join(required).exists() {
            return Err(invalid(format!(
                "asset root is missing required entry {required}"
            )));
        }
    }
    let limits = AssetLimits::new(i64::MAX as u64, i64::MAX as u64)?;
    let store = AssetStore::open(root, limits)?;
    let stats = store.verify_catalog_ledger()?;
    let filesystem_staging_entries = directory_entry_count(&root.join(".staging"))?;

    let mut catalog_paths = BTreeSet::new();
    let mut cursor = None;
    let mut listed_objects = 0_u64;
    let mut full_hashes_checked = 0_u64;
    let mut filesystem_object_bytes = 0_u64;
    let mut missing_object_files = 0_u64;
    loop {
        let page = store.list_objects(cursor.as_ref(), MAX_PAGE_SIZE)?;
        for object in &page.objects {
            listed_objects += 1;
            catalog_paths.insert(object.relative_path.clone());
            if object.state.as_str() == "active" {
                match store.open_object(&object.hash) {
                    Ok(mut reader) => {
                        filesystem_object_bytes = filesystem_object_bytes
                            .checked_add(object.size)
                            .ok_or_else(|| invalid("asset filesystem byte total overflowed"))?;
                        if full {
                            let mut digest = Sha256::new();
                            let mut buffer = [0_u8; 64 * 1024];
                            loop {
                                let read = reader.read(&mut buffer)?;
                                if read == 0 {
                                    break;
                                }
                                digest.update(&buffer[..read]);
                            }
                            let actual = format!("{:x}", digest.finalize());
                            if actual != object.hash.as_str() {
                                return Err(invalid(format!(
                                    "asset hash mismatch for {}",
                                    object.hash
                                )));
                            }
                            full_hashes_checked += 1;
                        }
                    }
                    Err(_) => missing_object_files += 1,
                }
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let filesystem_paths = collect_regular_files(root, &root.join("objects"))?;
    let untracked_object_files = filesystem_paths
        .iter()
        .filter(|path| !catalog_paths.contains(*path))
        .count() as u64;
    let mut issues = Vec::new();
    append_nonzero(&mut issues, stats.missing_count, "catalog missing assets");
    append_nonzero(&mut issues, stats.staging_count, "catalog staging rows");
    append_nonzero(
        &mut issues,
        filesystem_staging_entries,
        "filesystem staging entries",
    );
    append_nonzero(&mut issues, missing_object_files, "missing object files");
    append_nonzero(
        &mut issues,
        untracked_object_files,
        "untracked object files",
    );
    if listed_objects != stats.object_count {
        issues.push(format!(
            "listed object count {listed_objects} differs from ledger {}",
            stats.object_count
        ));
    }
    if filesystem_object_bytes != stats.active_bytes {
        issues.push(format!(
            "filesystem object bytes {filesystem_object_bytes} differ from active ledger bytes {}",
            stats.active_bytes
        ));
    }

    Ok((
        AssetVerification {
            root: root.display().to_string(),
            object_count: stats.object_count,
            active_bytes: stats.active_bytes,
            reference_count: stats.reference_count,
            missing_count: stats.missing_count,
            quarantined_count: stats.quarantined_count,
            catalog_staging_count: stats.staging_count,
            filesystem_staging_entries,
            listed_objects,
            full_hashes_checked,
            filesystem_object_bytes,
            untracked_object_files,
            missing_object_files,
        },
        issues,
    ))
}

fn directory_entry_count(path: &Path) -> Result<u64> {
    fs::read_dir(path)?.try_fold(0_u64, |count, entry| {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            return Err(invalid(format!(
                "unsafe symlink in asset storage: {}",
                entry.path().display()
            )));
        }
        count
            .checked_add(1)
            .ok_or_else(|| invalid("directory entry count overflowed"))
    })
}

fn collect_regular_files(root: &Path, directory: &Path) -> Result<BTreeSet<String>> {
    let mut pending = vec![directory.to_path_buf()];
    let mut paths = BTreeSet::new();
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(invalid(format!(
                    "unsafe symlink in object storage: {}",
                    entry.path().display()
                )));
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .into_owned();
                paths.insert(relative);
            } else {
                return Err(invalid(format!(
                    "unsafe non-regular object entry: {}",
                    entry.path().display()
                )));
            }
        }
    }
    Ok(paths)
}

fn query_u64(connection: &Connection, sql: &str) -> Result<u64> {
    let value: i64 = connection.query_row(sql, [], |row| row.get(0))?;
    u64::try_from(value).map_err(|_| invalid("database returned a negative count"))
}

fn append_nonzero(issues: &mut Vec<String>, count: u64, label: &str) {
    if count != 0 {
        issues.push(format!("{label}: {count}"));
    }
}

pub fn asset_stats(root: &Path) -> Result<AssetStats> {
    let limits = AssetLimits::new(i64::MAX as u64, i64::MAX as u64)?;
    Ok(AssetStore::open(root, limits)?.verify_catalog_ledger()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assets::{AssetOptions, generate as generate_assets},
        db::{BranchProfile, DbOptions, generate as generate_db},
    };

    #[test]
    fn verifies_db_fts_active_path_and_full_asset_hashes() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("fixture.sqlite3");
        let objects = directory.path().join("assets");
        let database_receipt = directory.path().join("db-receipt.json");
        let asset_receipt = directory.path().join("asset-receipt.json");
        generate_db(DbOptions {
            messages: 20,
            target_text_bytes: 20_000,
            branch_profile: BranchProfile::Fanout,
            seed: 7,
            output: database.clone(),
            receipt: Some(database_receipt),
        })
        .unwrap();
        generate_assets(AssetOptions {
            count: 5,
            target_active_bytes: 2_000,
            duplicate_rate: 0.4,
            seed: 7,
            output: objects.clone(),
            receipt: Some(asset_receipt),
        })
        .unwrap();
        let receipt = directory.path().join("verify.json");
        run(VerifyOptions {
            database,
            objects: Some(objects),
            full: true,
            output: Some(receipt.clone()),
        })
        .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_slice(&fs::read(receipt).unwrap()).unwrap();
        assert_eq!(parsed["passed"], true);
        assert_eq!(parsed["database"]["ftsIntegrityCommandRun"], true);
        assert_eq!(parsed["assets"]["fullHashesChecked"], 3);
    }
}
