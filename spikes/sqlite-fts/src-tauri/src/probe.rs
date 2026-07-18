use rusqlite::{params, Connection, ErrorCode, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, TryLockError};
use std::time::Duration;

const PROTOCOL_VERSION: u8 = 1;
const CURRENT_SCHEMA_VERSION: i64 = 2;
const FUTURE_SCHEMA_VERSION: i64 = 99;
const BUSY_TIMEOUT_MS: u64 = 250;
const SHORT_QUERY_LIMIT: usize = 64;
const PROBE_FILENAME: &str = "lorepia-m1-sqlite-fts-probe.sqlite3";
const FIXTURE_JSON: &str = include_str!("../../fixtures/korean-fts-v1.json");
const FIXTURE_SHA256: &str = "b5e8b2f2fdcf40d33dbb5eca555c982700e3cc1559dfe3adc878d85e2380b674";
const INITIAL_MARKER: &str = "m1-initial-marker";
const WRITER_MARKER: &str = "m1-writer-committed";
const RETRY_MARKER: &str = "m1-retry-succeeded";

static PROBE_LOCK: Mutex<()> = Mutex::new(());

const MIGRATION_V1: &str = r#"
CREATE TABLE probe_marker (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    value TEXT NOT NULL
);
CREATE TABLE fixture_records (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    raw_text TEXT NOT NULL
);
"#;

const MIGRATION_V2: &str = r#"
CREATE VIRTUAL TABLE fixture_fts USING fts5(
    raw_text,
    content = 'fixture_records',
    content_rowid = 'id',
    tokenize = 'trigram'
);
CREATE TRIGGER fixture_records_ai AFTER INSERT ON fixture_records BEGIN
    INSERT INTO fixture_fts(rowid, raw_text) VALUES (new.id, new.raw_text);
END;
CREATE TRIGGER fixture_records_ad AFTER DELETE ON fixture_records BEGIN
    INSERT INTO fixture_fts(fixture_fts, rowid, raw_text)
    VALUES ('delete', old.id, old.raw_text);
END;
CREATE TRIGGER fixture_records_au AFTER UPDATE ON fixture_records BEGIN
    INSERT INTO fixture_fts(fixture_fts, rowid, raw_text)
    VALUES ('delete', old.id, old.raw_text);
    INSERT INTO fixture_fts(rowid, raw_text) VALUES (new.id, new.raw_text);
END;
INSERT INTO fixture_fts(fixture_fts) VALUES ('rebuild');
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProbeErrorCode {
    PathUnavailable,
    OpenFailure,
    MigrationFailure,
    PersistenceFailure,
    ConcurrencyFailure,
    FtsUnavailable,
    FtsGoldenMismatch,
    CleanupFailure,
    ProbeBusy,
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeError {
    code: ProbeErrorCode,
    cleanup_pending: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceEvidence {
    marker_reopened: bool,
    fixture_rows_reopened: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcurrencyEvidence {
    journal_mode: &'static str,
    busy_timeout_ms: u64,
    reader_writer_concurrent: bool,
    snapshot_isolated: bool,
    busy_observed: bool,
    retry_succeeded: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenResult {
    query_id: String,
    result_ids: Vec<i64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchEvidence {
    tokenizer: &'static str,
    short_query_policy: &'static str,
    short_query_limit: usize,
    golden: Vec<GoldenResult>,
    mutation_sync: bool,
    injection_safe: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReceipt {
    protocol_version: u8,
    schema_version: i64,
    applied_migrations: [i64; 2],
    migrations_idempotent: bool,
    future_schema_rejected: bool,
    persistence: PersistenceEvidence,
    concurrency: ConcurrencyEvidence,
    search: SearchEvidence,
    sqlite_version: String,
    compile_options: Vec<String>,
    fts5_enabled: bool,
    fixture_sha256: String,
    cleanup_pending: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    version: u8,
    license: String,
    records: Vec<FixtureRecord>,
    queries: Vec<FixtureQuery>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureRecord {
    id: i64,
    title: String,
    raw_text: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum QueryMode {
    Fts,
    Like,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureQuery {
    query_id: String,
    mode: QueryMode,
    term: String,
    expected_ids: Vec<i64>,
}

fn probe_error(code: ProbeErrorCode) -> ProbeError {
    ProbeError {
        code,
        cleanup_pending: false,
    }
}

pub fn path_unavailable_error() -> ProbeError {
    probe_error(ProbeErrorCode::PathUnavailable)
}

pub fn internal_state_error() -> ProbeError {
    probe_error(ProbeErrorCode::InternalState)
}

pub fn with_process_lock<T>(
    operation: impl FnOnce() -> Result<T, ProbeError>,
) -> Result<T, ProbeError> {
    let _guard = match PROBE_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Err(probe_error(ProbeErrorCode::ProbeBusy)),
        Err(TryLockError::Poisoned(_)) => {
            return Err(probe_error(ProbeErrorCode::InternalState));
        }
    };
    operation()
}

pub fn run_probe_in_directory(directory: &Path) -> Result<ProbeReceipt, ProbeError> {
    fs::create_dir_all(directory).map_err(|_| path_unavailable_error())?;
    let database_path = directory.join(PROBE_FILENAME);

    if cleanup_probe_files(&database_path).is_err() {
        return Err(ProbeError {
            code: ProbeErrorCode::CleanupFailure,
            cleanup_pending: true,
        });
    }

    let result = run_probe(&database_path);
    let cleanup_result = cleanup_probe_files(&database_path);

    match (result, cleanup_result) {
        (Ok(receipt), Ok(())) => Ok(receipt),
        (Ok(_), Err(())) => Err(ProbeError {
            code: ProbeErrorCode::CleanupFailure,
            cleanup_pending: true,
        }),
        (Err(mut error), Ok(())) => {
            error.cleanup_pending = false;
            Err(error)
        }
        (Err(mut error), Err(())) => {
            error.cleanup_pending = true;
            Err(error)
        }
    }
}

fn run_probe(database_path: &Path) -> Result<ProbeReceipt, ProbeError> {
    let fixture = parse_and_validate_fixture()?;
    let fixture_sha256 = sha256_hex(FIXTURE_JSON.as_bytes());
    if fixture_sha256 != FIXTURE_SHA256 {
        return Err(probe_error(ProbeErrorCode::InternalState));
    }
    let future_schema_rejected = verify_future_schema_rejection(database_path)?;
    let mut applied_migrations = Vec::new();

    {
        let mut connection = open_new_database(database_path)?;
        applied_migrations.extend(apply_migrations(&mut connection, 1)?);
        insert_fixture(&mut connection, &fixture)?;
    }

    let persistence = {
        let mut connection = open_existing_database(database_path)?;
        let marker_reopened = marker_value(&connection)? == INITIAL_MARKER;
        let fixture_rows_reopened = fixture_rows_match(&connection, &fixture)?;
        if !marker_reopened || !fixture_rows_reopened {
            return Err(probe_error(ProbeErrorCode::PersistenceFailure));
        }
        ensure_fts5_trigram_available(&connection)?;
        applied_migrations.extend(apply_migrations(&mut connection, CURRENT_SCHEMA_VERSION)?);
        PersistenceEvidence {
            marker_reopened,
            fixture_rows_reopened,
        }
    };

    {
        let mut connection = open_existing_database(database_path)?;
        let second_pass = apply_migrations(&mut connection, CURRENT_SCHEMA_VERSION)?;
        if !second_pass.is_empty() {
            return Err(probe_error(ProbeErrorCode::MigrationFailure));
        }
    }

    if applied_migrations != [1, 2] {
        return Err(probe_error(ProbeErrorCode::MigrationFailure));
    }

    let concurrency = verify_concurrency(database_path)?;

    let (search, sqlite_version, compile_options) = {
        let connection = open_existing_database(database_path)?;
        if schema_version(&connection)? != CURRENT_SCHEMA_VERSION
            || marker_value(&connection)? != RETRY_MARKER
            || !fixture_rows_match(&connection, &fixture)?
        {
            return Err(probe_error(ProbeErrorCode::PersistenceFailure));
        }
        let compile_options = sqlite_compile_options(&connection)?;
        if !compile_options.iter().any(|option| option == "ENABLE_FTS5") {
            return Err(probe_error(ProbeErrorCode::FtsUnavailable));
        }
        let sqlite_version = sqlite_version(&connection)?;
        let search = verify_search(&connection, &fixture)?;
        (search, sqlite_version, compile_options)
    };

    Ok(ProbeReceipt {
        protocol_version: PROTOCOL_VERSION,
        schema_version: CURRENT_SCHEMA_VERSION,
        applied_migrations: [1, 2],
        migrations_idempotent: true,
        future_schema_rejected,
        persistence,
        concurrency,
        search,
        sqlite_version,
        compile_options,
        fts5_enabled: true,
        fixture_sha256,
        cleanup_pending: false,
    })
}

fn parse_and_validate_fixture() -> Result<Fixture, ProbeError> {
    let fixture: Fixture = serde_json::from_str(FIXTURE_JSON)
        .map_err(|_| probe_error(ProbeErrorCode::InternalState))?;
    if fixture.version != 1
        || fixture.license != "CC0-1.0"
        || fixture.records.is_empty()
        || fixture.records.len() > SHORT_QUERY_LIMIT
        || fixture.queries.is_empty()
        || fixture.queries.len() > SHORT_QUERY_LIMIT
    {
        return Err(probe_error(ProbeErrorCode::InternalState));
    }
    if fixture
        .records
        .windows(2)
        .any(|pair| pair[0].id >= pair[1].id)
    {
        return Err(probe_error(ProbeErrorCode::InternalState));
    }

    let mut record_ids = HashSet::new();
    for record in &fixture.records {
        if record.id <= 0
            || record.id > i64::from(i32::MAX)
            || record.title.is_empty()
            || record.raw_text.is_empty()
            || !record_ids.insert(record.id)
        {
            return Err(probe_error(ProbeErrorCode::InternalState));
        }
    }

    let mut query_ids = HashSet::new();
    for query in &fixture.queries {
        let character_count = query.term.chars().count();
        let derived_mode = if character_count <= 2 {
            QueryMode::Like
        } else {
            QueryMode::Fts
        };
        let query_id_valid = !query.query_id.is_empty()
            && query.query_id.len() <= 64
            && query.query_id.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || (byte == b'-' && index > 0)
            });
        if !query_id_valid
            || !query_ids.insert(query.query_id.as_str())
            || character_count == 0
            || character_count > 64
            || query.mode != derived_mode
            || query.expected_ids.len() > SHORT_QUERY_LIMIT
            || query.expected_ids.windows(2).any(|pair| pair[0] >= pair[1])
            || query
                .expected_ids
                .iter()
                .any(|id| *id <= 0 || *id > i64::from(i32::MAX) || !record_ids.contains(id))
        {
            return Err(probe_error(ProbeErrorCode::InternalState));
        }
    }
    Ok(fixture)
}

fn open_new_database(path: &Path) -> Result<Connection, ProbeError> {
    let connection =
        Connection::open(path).map_err(|_| probe_error(ProbeErrorCode::OpenFailure))?;
    configure_connection(&connection)?;
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(probe_error(ProbeErrorCode::ConcurrencyFailure));
    }
    Ok(connection)
}

fn open_existing_database(path: &Path) -> Result<Connection, ProbeError> {
    let connection =
        Connection::open(path).map_err(|_| probe_error(ProbeErrorCode::OpenFailure))?;
    configure_connection(&connection)?;
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(probe_error(ProbeErrorCode::ConcurrencyFailure));
    }
    Ok(connection)
}

fn configure_connection(connection: &Connection) -> Result<(), ProbeError> {
    connection
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .map_err(|_| probe_error(ProbeErrorCode::OpenFailure))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|_| probe_error(ProbeErrorCode::OpenFailure))?;
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(|_| probe_error(ProbeErrorCode::OpenFailure))?;
    let busy_timeout: i64 = connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .map_err(|_| probe_error(ProbeErrorCode::OpenFailure))?;
    if foreign_keys != 1 || busy_timeout != BUSY_TIMEOUT_MS as i64 {
        return Err(probe_error(ProbeErrorCode::OpenFailure));
    }
    Ok(())
}

fn ensure_schema_meta(connection: &Connection) -> Result<(), ProbeError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (\
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
                version INTEGER NOT NULL\
            );\
            INSERT OR IGNORE INTO schema_meta(singleton, version) VALUES (1, 0);",
        )
        .map_err(|_| probe_error(ProbeErrorCode::MigrationFailure))?;
    Ok(())
}

fn schema_version(connection: &Connection) -> Result<i64, ProbeError> {
    ensure_schema_meta(connection)?;
    connection
        .query_row(
            "SELECT version FROM schema_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| probe_error(ProbeErrorCode::MigrationFailure))
}

fn apply_migrations(
    connection: &mut Connection,
    target_version: i64,
) -> Result<Vec<i64>, ProbeError> {
    ensure_schema_meta(connection)?;
    let mut current = schema_version(connection)?;
    if current > CURRENT_SCHEMA_VERSION || target_version > CURRENT_SCHEMA_VERSION {
        return Err(probe_error(ProbeErrorCode::MigrationFailure));
    }
    let mut applied = Vec::new();
    while current < target_version {
        let next = current + 1;
        let sql = match next {
            1 => MIGRATION_V1,
            2 => MIGRATION_V2,
            _ => return Err(probe_error(ProbeErrorCode::MigrationFailure)),
        };
        apply_single_migration(connection, next, sql)?;
        applied.push(next);
        current = next;
    }
    Ok(applied)
}

fn apply_single_migration(
    connection: &mut Connection,
    next_version: i64,
    sql: &str,
) -> Result<(), ProbeError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| probe_error(ProbeErrorCode::MigrationFailure))?;
    transaction
        .execute_batch(sql)
        .map_err(|_| probe_error(ProbeErrorCode::MigrationFailure))?;
    let changed = transaction
        .execute(
            "UPDATE schema_meta SET version = ?1 WHERE singleton = 1",
            [next_version],
        )
        .map_err(|_| probe_error(ProbeErrorCode::MigrationFailure))?;
    if changed != 1 {
        return Err(probe_error(ProbeErrorCode::MigrationFailure));
    }
    transaction
        .commit()
        .map_err(|_| probe_error(ProbeErrorCode::MigrationFailure))
}

fn verify_future_schema_rejection(database_path: &Path) -> Result<bool, ProbeError> {
    {
        let connection = open_new_database(database_path)?;
        ensure_schema_meta(&connection)?;
        let changed = connection
            .execute(
                "UPDATE schema_meta SET version = ?1 WHERE singleton = 1",
                [FUTURE_SCHEMA_VERSION],
            )
            .map_err(|_| probe_error(ProbeErrorCode::MigrationFailure))?;
        if changed != 1 {
            return Err(probe_error(ProbeErrorCode::MigrationFailure));
        }
    }

    {
        let mut connection = open_existing_database(database_path)?;
        let rejected = matches!(
            apply_migrations(&mut connection, CURRENT_SCHEMA_VERSION),
            Err(ProbeError {
                code: ProbeErrorCode::MigrationFailure,
                ..
            })
        );
        if !rejected || schema_version(&connection)? != FUTURE_SCHEMA_VERSION {
            return Err(probe_error(ProbeErrorCode::MigrationFailure));
        }
    }

    cleanup_probe_files(database_path).map_err(|_| ProbeError {
        code: ProbeErrorCode::CleanupFailure,
        cleanup_pending: true,
    })?;
    Ok(true)
}

fn ensure_fts5_trigram_available(connection: &Connection) -> Result<(), ProbeError> {
    let fts5_enabled: i64 = connection
        .query_row(
            "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
            [],
            |row| row.get(0),
        )
        .map_err(|_| probe_error(ProbeErrorCode::FtsUnavailable))?;
    if fts5_enabled != 1 {
        return Err(probe_error(ProbeErrorCode::FtsUnavailable));
    }
    connection
        .execute_batch(
            "CREATE VIRTUAL TABLE temp.lorepia_fts5_trigram_capability \
             USING fts5(raw_text, tokenize = 'trigram');\
             DROP TABLE temp.lorepia_fts5_trigram_capability;",
        )
        .map_err(|_| probe_error(ProbeErrorCode::FtsUnavailable))
}

fn insert_fixture(connection: &mut Connection, fixture: &Fixture) -> Result<(), ProbeError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))?;
    transaction
        .execute(
            "INSERT INTO probe_marker(singleton, value) VALUES (1, ?1)",
            [INITIAL_MARKER],
        )
        .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))?;
    {
        let mut statement = transaction
            .prepare("INSERT INTO fixture_records(id, title, raw_text) VALUES (?1, ?2, ?3)")
            .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))?;
        for record in &fixture.records {
            statement
                .execute(params![record.id, record.title, record.raw_text])
                .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))?;
        }
    }
    transaction
        .commit()
        .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))
}

fn marker_value(connection: &Connection) -> Result<String, ProbeError> {
    connection
        .query_row(
            "SELECT value FROM probe_marker WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))
}

fn fixture_rows_match(connection: &Connection, fixture: &Fixture) -> Result<bool, ProbeError> {
    let mut statement = connection
        .prepare("SELECT id, title, raw_text FROM fixture_records ORDER BY id")
        .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))?;
    let rows = statement
        .query_map([], |row| {
            Ok(FixtureRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                raw_text: row.get(2)?,
            })
        })
        .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))?;
    let actual = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error(ProbeErrorCode::PersistenceFailure))?;
    Ok(actual == fixture.records)
}

fn verify_concurrency(database_path: &Path) -> Result<ConcurrencyEvidence, ProbeError> {
    let reader = open_existing_database(database_path)?;
    let writer = open_existing_database(database_path)?;

    reader
        .execute_batch("BEGIN DEFERRED")
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    let snapshot_before =
        marker_value(&reader).map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    let changed = writer
        .execute(
            "UPDATE probe_marker SET value = ?1 WHERE singleton = 1",
            [WRITER_MARKER],
        )
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    let snapshot_during =
        marker_value(&reader).map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    reader
        .execute_batch("COMMIT")
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    let visible_after =
        marker_value(&reader).map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;

    if changed != 1
        || snapshot_before != INITIAL_MARKER
        || snapshot_during != INITIAL_MARKER
        || visible_after != WRITER_MARKER
    {
        return Err(probe_error(ProbeErrorCode::ConcurrencyFailure));
    }
    drop(writer);
    drop(reader);

    let locker = open_existing_database(database_path)?;
    let contender = open_existing_database(database_path)?;
    locker
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    locker
        .execute(
            "UPDATE probe_marker SET value = 'm1-lock-held' WHERE singleton = 1",
            [],
        )
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    contender
        .busy_timeout(Duration::ZERO)
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    let busy_observed = matches!(
        contender.execute(
            "UPDATE probe_marker SET value = ?1 WHERE singleton = 1",
            [RETRY_MARKER],
        ),
        Err(error) if error.sqlite_error_code() == Some(ErrorCode::DatabaseBusy)
    );
    locker
        .execute_batch("ROLLBACK")
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    contender
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;
    let retry_succeeded = contender
        .execute(
            "UPDATE probe_marker SET value = ?1 WHERE singleton = 1",
            [RETRY_MARKER],
        )
        .map(|count| count == 1)
        .map_err(|_| probe_error(ProbeErrorCode::ConcurrencyFailure))?;

    if !busy_observed || !retry_succeeded {
        return Err(probe_error(ProbeErrorCode::ConcurrencyFailure));
    }

    Ok(ConcurrencyEvidence {
        journal_mode: "wal",
        busy_timeout_ms: BUSY_TIMEOUT_MS,
        reader_writer_concurrent: true,
        snapshot_isolated: true,
        busy_observed,
        retry_succeeded,
    })
}

fn verify_search(connection: &Connection, fixture: &Fixture) -> Result<SearchEvidence, ProbeError> {
    let mut golden = Vec::with_capacity(fixture.queries.len());
    for query in &fixture.queries {
        let result_ids = search_ids(connection, &query.term)?;
        if result_ids != query.expected_ids {
            return Err(probe_error(ProbeErrorCode::FtsGoldenMismatch));
        }
        golden.push(GoldenResult {
            query_id: query.query_id.clone(),
            result_ids,
        });
    }

    let mutation_sync = verify_fts_mutation_sync(connection)?;
    if !mutation_sync {
        return Err(probe_error(ProbeErrorCode::FtsGoldenMismatch));
    }
    connection
        .execute(
            "INSERT INTO fixture_fts(fixture_fts, rank) VALUES ('integrity-check', 1)",
            [],
        )
        .map_err(|_| probe_error(ProbeErrorCode::FtsGoldenMismatch))?;

    let injection_safe = fixture.queries.iter().any(|query| {
        query.query_id == "q-fts-injection-literal"
            && query.mode == QueryMode::Fts
            && query.expected_ids.is_empty()
    });
    if !injection_safe {
        return Err(probe_error(ProbeErrorCode::InternalState));
    }

    Ok(SearchEvidence {
        tokenizer: "trigram",
        short_query_policy: "escaped-like-bounded",
        short_query_limit: SHORT_QUERY_LIMIT,
        golden,
        mutation_sync,
        injection_safe,
    })
}

fn search_ids(connection: &Connection, term: &str) -> Result<Vec<i64>, ProbeError> {
    if term.is_empty() || term.chars().count() > 64 {
        return Err(probe_error(ProbeErrorCode::InternalState));
    }
    if term.chars().count() <= 2 {
        let pattern = format!("%{}%", escape_like_literal(term));
        collect_ids(
            connection,
            "SELECT id FROM fixture_records \
             WHERE raw_text LIKE ?1 ESCAPE '\\' \
             ORDER BY id LIMIT 64",
            &pattern,
        )
    } else {
        let expression = format!("\"{}\"", term.replace('"', "\"\""));
        collect_ids(
            connection,
            "SELECT rowid FROM fixture_fts \
             WHERE fixture_fts MATCH ?1 \
             ORDER BY rowid LIMIT 64",
            &expression,
        )
    }
}

fn collect_ids(connection: &Connection, sql: &str, argument: &str) -> Result<Vec<i64>, ProbeError> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|_| probe_error(ProbeErrorCode::FtsGoldenMismatch))?;
    let rows = statement
        .query_map([argument], |row| row.get::<_, i64>(0))
        .map_err(|_| probe_error(ProbeErrorCode::FtsGoldenMismatch))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row.map_err(|_| probe_error(ProbeErrorCode::FtsGoldenMismatch))?);
    }
    if ids.len() > SHORT_QUERY_LIMIT || ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(probe_error(ProbeErrorCode::FtsGoldenMismatch));
    }
    Ok(ids)
}

fn escape_like_literal(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len());
    for character in term.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn verify_fts_mutation_sync(connection: &Connection) -> Result<bool, ProbeError> {
    connection
        .execute(
            "INSERT INTO fixture_records(id, title, raw_text) \
             VALUES (99, '동기화 검증', '동기화 삽입 검증 문장')",
            [],
        )
        .map_err(|_| probe_error(ProbeErrorCode::FtsGoldenMismatch))?;
    let inserted = search_ids(connection, "동기화")? == [99];
    connection
        .execute(
            "UPDATE fixture_records SET raw_text = '수정된 색인 검증 문장' WHERE id = 99",
            [],
        )
        .map_err(|_| probe_error(ProbeErrorCode::FtsGoldenMismatch))?;
    let old_removed = search_ids(connection, "동기화")?.is_empty();
    let updated = search_ids(connection, "수정된")? == [99];
    connection
        .execute("DELETE FROM fixture_records WHERE id = 99", [])
        .map_err(|_| probe_error(ProbeErrorCode::FtsGoldenMismatch))?;
    let deleted = search_ids(connection, "수정된")?.is_empty();
    Ok(inserted && old_removed && updated && deleted)
}

fn sqlite_version(connection: &Connection) -> Result<String, ProbeError> {
    connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .map_err(|_| probe_error(ProbeErrorCode::InternalState))
}

fn sqlite_compile_options(connection: &Connection) -> Result<Vec<String>, ProbeError> {
    let mut statement = connection
        .prepare("PRAGMA compile_options")
        .map_err(|_| probe_error(ProbeErrorCode::InternalState))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| probe_error(ProbeErrorCode::InternalState))?;
    let mut options = Vec::new();
    for row in rows {
        let option = row.map_err(|_| probe_error(ProbeErrorCode::InternalState))?;
        if !is_bounded_compile_option(&option) {
            return Err(probe_error(ProbeErrorCode::InternalState));
        }
        options.push(option);
    }
    options.sort();
    options.dedup();
    if options.is_empty() || options.len() > 128 {
        return Err(probe_error(ProbeErrorCode::InternalState));
    }
    Ok(options)
}

fn is_bounded_compile_option(option: &str) -> bool {
    if option.is_empty() || option.len() > 128 {
        return false;
    }
    let (name, value) = match option.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (option, None),
    };
    if !name
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_uppercase())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return false;
    }
    value.is_none_or(|value| {
        !value.is_empty()
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'_' | b'.' | b'+' | b':' | b'/' | b'-')
            })
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(database_path.as_os_str());
    value.push(suffix);
    PathBuf::from(value)
}

fn cleanup_probe_files(database_path: &Path) -> Result<(), ()> {
    let paths = [
        sidecar_path(database_path, "-shm"),
        sidecar_path(database_path, "-wal"),
        database_path.to_path_buf(),
    ];
    let mut failed = false;
    for path in &paths {
        match path.try_exists() {
            Ok(true) => {
                if fs::remove_file(path).is_err() {
                    failed = true;
                }
            }
            Ok(false) => {}
            Err(_) => failed = true,
        }
    }
    for path in &paths {
        if path.try_exists().unwrap_or(true) {
            failed = true;
        }
    }
    if failed {
        Err(())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn database_path(directory: &Path) -> PathBuf {
        directory.join(PROBE_FILENAME)
    }

    #[test]
    fn full_file_backed_probe_passes_and_removes_all_files() {
        let directory = tempfile::tempdir().expect("tempdir");
        let receipt = run_probe_in_directory(directory.path()).expect("probe must pass");
        assert_eq!(receipt.protocol_version, 1);
        assert_eq!(receipt.schema_version, 2);
        assert_eq!(receipt.applied_migrations, [1, 2]);
        assert!(receipt.migrations_idempotent);
        assert!(receipt.future_schema_rejected);
        assert!(receipt.persistence.marker_reopened);
        assert!(receipt.persistence.fixture_rows_reopened);
        assert_eq!(receipt.concurrency.journal_mode, "wal");
        assert_eq!(receipt.concurrency.busy_timeout_ms, 250);
        assert!(receipt.concurrency.reader_writer_concurrent);
        assert!(receipt.concurrency.snapshot_isolated);
        assert!(receipt.concurrency.busy_observed);
        assert!(receipt.concurrency.retry_succeeded);
        assert_eq!(receipt.search.tokenizer, "trigram");
        assert_eq!(receipt.search.short_query_limit, 64);
        assert_eq!(receipt.search.golden.len(), 7);
        assert!(receipt.search.mutation_sync);
        assert!(receipt.search.injection_safe);
        assert!(receipt
            .compile_options
            .iter()
            .any(|value| value == "ENABLE_FTS5"));
        assert_eq!(receipt.fixture_sha256, FIXTURE_SHA256);
        assert!(!receipt.cleanup_pending);

        let path = database_path(directory.path());
        assert!(!path.exists());
        assert!(!sidecar_path(&path, "-wal").exists());
        assert!(!sidecar_path(&path, "-shm").exists());
    }

    #[test]
    fn stale_owned_probe_files_are_replaced_and_cleaned() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = database_path(directory.path());
        fs::write(&path, b"stale").expect("stale db");
        fs::write(sidecar_path(&path, "-wal"), b"stale wal").expect("stale wal");
        fs::write(sidecar_path(&path, "-shm"), b"stale shm").expect("stale shm");

        run_probe_in_directory(directory.path()).expect("probe must recover owned files");
        assert!(!path.exists());
        assert!(!sidecar_path(&path, "-wal").exists());
        assert!(!sidecar_path(&path, "-shm").exists());
    }

    #[test]
    fn preflight_cleanup_failure_fails_closed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = database_path(directory.path());
        fs::create_dir(&path).expect("directory collision");

        let error = run_probe_in_directory(directory.path()).expect_err("must fail");
        assert_eq!(error.code, ProbeErrorCode::CleanupFailure);
        assert!(error.cleanup_pending);
    }

    #[test]
    fn process_lock_rejects_a_concurrent_probe() {
        let guard = PROBE_LOCK.lock().expect("lock");
        let error = with_process_lock(|| Ok(())).expect_err("must be busy");
        assert_eq!(error, probe_error(ProbeErrorCode::ProbeBusy));
        drop(guard);
    }

    #[test]
    fn migrations_are_ordered_and_idempotent() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = database_path(directory.path());
        let mut connection = open_new_database(&path).expect("open");
        assert_eq!(
            apply_migrations(&mut connection, 2).expect("migrate"),
            [1, 2]
        );
        drop(connection);
        let mut connection = open_existing_database(&path).expect("reopen");
        assert!(apply_migrations(&mut connection, 2)
            .expect("second migrate")
            .is_empty());
        assert_eq!(schema_version(&connection).expect("version"), 2);
    }

    #[test]
    fn exact_fixture_reopen_check_detects_same_count_corruption() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = database_path(directory.path());
        let fixture = parse_and_validate_fixture().expect("fixture");
        let mut connection = open_new_database(&path).expect("open");
        apply_migrations(&mut connection, 1).expect("v1");
        insert_fixture(&mut connection, &fixture).expect("fixture rows");
        assert!(fixture_rows_match(&connection, &fixture).expect("exact rows"));
        connection
            .execute(
                "UPDATE fixture_records SET title = '손상됨' WHERE id = 1",
                [],
            )
            .expect("same-count corruption");
        assert!(!fixture_rows_match(&connection, &fixture).expect("mismatch"));
    }

    #[test]
    fn future_schema_version_fails_closed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = database_path(directory.path());
        let mut connection = open_new_database(&path).expect("open");
        ensure_schema_meta(&connection).expect("meta");
        connection
            .execute("UPDATE schema_meta SET version = 99", [])
            .expect("future version");
        let error = apply_migrations(&mut connection, 2).expect_err("must fail");
        assert_eq!(error.code, ProbeErrorCode::MigrationFailure);
    }

    #[test]
    fn failed_migration_rolls_back_schema_and_version() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = database_path(directory.path());
        let mut connection = open_new_database(&path).expect("open");
        ensure_schema_meta(&connection).expect("meta");
        let error = apply_single_migration(
            &mut connection,
            1,
            "CREATE TABLE must_rollback(id INTEGER); THIS IS NOT SQL;",
        )
        .expect_err("must fail");
        assert_eq!(error.code, ProbeErrorCode::MigrationFailure);
        assert_eq!(schema_version(&connection).expect("version"), 0);
        let table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='must_rollback'",
                [],
                |row| row.get(0),
            )
            .expect("catalog");
        assert_eq!(table_count, 0);
    }

    #[test]
    fn search_policy_escapes_fts_and_like_literals() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = database_path(directory.path());
        let fixture = parse_and_validate_fixture().expect("fixture");
        let mut connection = open_new_database(&path).expect("open");
        apply_migrations(&mut connection, 1).expect("v1");
        insert_fixture(&mut connection, &fixture).expect("fixture rows");
        apply_migrations(&mut connection, 2).expect("v2");

        assert_eq!(search_ids(&connection, "%_").expect("like literal"), [5]);
        assert!(search_ids(&connection, "%' OR 1=1 --")
            .expect("fts literal")
            .is_empty());
        assert_eq!(
            search_ids(&connection, "빛").expect("short Korean"),
            [1, 2, 4]
        );
        assert_eq!(search_ids(&connection, "도서관").expect("trigram"), [1]);
    }

    #[test]
    fn serialized_receipt_and_errors_have_only_the_bounded_contract() {
        let directory = tempfile::tempdir().expect("tempdir");
        let receipt = run_probe_in_directory(directory.path()).expect("probe");
        let value = serde_json::to_value(receipt).expect("serialize receipt");
        let expected_keys = [
            "appliedMigrations",
            "cleanupPending",
            "compileOptions",
            "concurrency",
            "fixtureSha256",
            "fts5Enabled",
            "futureSchemaRejected",
            "migrationsIdempotent",
            "persistence",
            "protocolVersion",
            "schemaVersion",
            "search",
            "sqliteVersion",
        ];
        let mut actual_keys: Vec<_> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        actual_keys.sort_unstable();
        assert_eq!(actual_keys, expected_keys);

        assert_eq!(
            serde_json::to_value(probe_error(ProbeErrorCode::OpenFailure)).expect("error"),
            json!({"code": "OPEN_FAILURE", "cleanupPending": false})
        );
    }
}
