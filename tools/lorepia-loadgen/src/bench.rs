use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use rusqlite::{Connection, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    util::{
        DeterministicRng, Result, canonical_existing_dir, canonical_existing_file,
        checked_sum_file_sizes, invalid, sqlite_sidecars, write_json_atomic,
    },
    verify::{asset_stats, verify_database},
};

#[derive(Debug)]
pub struct BenchOptions {
    pub database: PathBuf,
    pub objects: Option<PathBuf>,
    pub seed: u64,
    pub warmup: usize,
    pub iterations: usize,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatencySummary {
    samples: usize,
    unit: &'static str,
    p50: u64,
    p95: u64,
    p99: u64,
    max: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkCase {
    name: &'static str,
    latency: LatencySummary,
    query_plan: Vec<String>,
    uses_offset: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceState {
    commit: Option<String>,
    dirty: Option<bool>,
    cargo_lock_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Environment {
    os: String,
    architecture: &'static str,
    os_version: Option<String>,
    rustc: Option<String>,
    peak_sampled_rss_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SizeEvidence {
    database_bytes: u64,
    wal_bytes: u64,
    shm_bytes: u64,
    total_database_and_sidecars_bytes: u64,
    fts_page_bytes: Option<u64>,
    object_active_bytes: Option<u64>,
    object_count: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchReceipt {
    artifact_kind: &'static str,
    tool_version: &'static str,
    schema_version: i64,
    seed: u64,
    warmup: usize,
    iterations: usize,
    database: String,
    objects: Option<String>,
    cases: Vec<BenchmarkCase>,
    sizes: SizeEvidence,
    environment: Environment,
    source: SourceState,
    release_evidence_eligible: bool,
    release_evidence_blockers: Vec<String>,
    raw_prompt_or_secret_included: bool,
}

pub fn run(options: BenchOptions) -> Result<()> {
    if options.warmup > 10_000 {
        return Err(invalid("--warmup must not exceed 10000"));
    }
    if !(1..=100_000).contains(&options.iterations) {
        return Err(invalid("--iterations must be between 1 and 100000"));
    }
    let database = canonical_existing_file(&options.database)?;
    let objects = options
        .objects
        .as_deref()
        .map(canonical_existing_dir)
        .transpose()?;
    if let Some(root) = objects.as_deref()
        && !root.join("assets.sqlite3").is_file()
    {
        return Err(invalid(
            "--objects must name an existing LorePia asset-store root",
        ));
    }
    let (verification, issues) = verify_database(&database, false)?;
    if !issues.is_empty() {
        return Err(invalid(format!(
            "benchmark input failed verification: {}",
            issues.join("; ")
        )));
    }

    let connection = Connection::open(&database)?;
    connection.pragma_update(None, "query_only", "ON")?;
    let chat_id = connection
        .query_row("SELECT id FROM chats ORDER BY id LIMIT 1", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap_or_default();

    for _ in 0..options.warmup {
        run_recent(&connection, &chat_id)?;
        run_search(&connection, &chat_id)?;
        run_active_path(&connection, &chat_id)?;
    }

    let mut recent = Vec::with_capacity(options.iterations);
    let mut search = Vec::with_capacity(options.iterations);
    let mut active_path = Vec::with_capacity(options.iterations);
    let mut peak_rss = sampled_rss_bytes();
    let mut rng = DeterministicRng::new(options.seed);
    for _ in 0..options.iterations {
        let rotation = rng.next_u64() % 3;
        for operation in 0..3 {
            match (rotation + operation) % 3 {
                0 => recent.push(measure(|| run_recent(&connection, &chat_id))?),
                1 => search.push(measure(|| run_search(&connection, &chat_id))?),
                _ => active_path.push(measure(|| run_active_path(&connection, &chat_id))?),
            }
        }
        if let Some(rss) = sampled_rss_bytes() {
            peak_rss = Some(peak_rss.unwrap_or(0).max(rss));
        }
    }

    let recent_plan = explain_recent(&connection, &chat_id)?;
    let search_plan = explain_search(&connection, &chat_id)?;
    let active_plan = explain_active_path(&connection, &chat_id)?;
    let fts_page_bytes = connection
        .query_row(
            "SELECT coalesce(sum(pgsize), 0) FROM dbstat WHERE name LIKE 'messages_fts%'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .and_then(|value| u64::try_from(value).ok());
    drop(connection);

    let sidecars = sqlite_sidecars(&database);
    let database_bytes = fs::metadata(&sidecars[0])?.len();
    let wal_bytes = sidecars
        .get(1)
        .filter(|path| path.exists())
        .map(fs::metadata)
        .transpose()?
        .map_or(0, |metadata| metadata.len());
    let shm_bytes = sidecars
        .get(2)
        .filter(|path| path.exists())
        .map(fs::metadata)
        .transpose()?
        .map_or(0, |metadata| metadata.len());
    let total_database_and_sidecars_bytes = checked_sum_file_sizes(&sidecars)?;
    let object_stats = objects.as_deref().map(asset_stats).transpose()?;

    let source = source_state();
    let blockers = release_evidence_blockers(&source);

    let receipt = BenchReceipt {
        artifact_kind: "LOREPIA_DETERMINISTIC_BENCHMARK_RECEIPT",
        tool_version: env!("CARGO_PKG_VERSION"),
        schema_version: verification.schema_version,
        seed: options.seed,
        warmup: options.warmup,
        iterations: options.iterations,
        database: database.display().to_string(),
        objects: objects.as_ref().map(|path| path.display().to_string()),
        cases: vec![
            BenchmarkCase {
                name: "recent_messages_keyset",
                latency: summarize(recent),
                query_plan: recent_plan,
                uses_offset: false,
            },
            BenchmarkCase {
                name: "fts_chat_search",
                latency: summarize(search),
                query_plan: search_plan,
                uses_offset: false,
            },
            BenchmarkCase {
                name: "active_path_keyset",
                latency: summarize(active_path),
                query_plan: active_plan,
                uses_offset: false,
            },
        ],
        sizes: SizeEvidence {
            database_bytes,
            wal_bytes,
            shm_bytes,
            total_database_and_sidecars_bytes,
            fts_page_bytes,
            object_active_bytes: object_stats.as_ref().map(|stats| stats.active_bytes),
            object_count: object_stats.as_ref().map(|stats| stats.object_count),
        },
        environment: Environment {
            os: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH,
            os_version: command_output("uname", &["-srvmo"]),
            rustc: command_output("rustc", &["--version"]),
            peak_sampled_rss_bytes: peak_rss,
        },
        release_evidence_eligible: blockers.is_empty(),
        release_evidence_blockers: blockers,
        source,
        raw_prompt_or_secret_included: false,
    };
    write_json_atomic(&options.output, &receipt)
}

fn release_evidence_blockers(source: &SourceState) -> Vec<String> {
    let mut blockers = Vec::new();
    match source.dirty {
        Some(true) => blockers.push("DIRTY_WORKTREE".to_owned()),
        Some(false) => {}
        None => blockers.push("WORKTREE_STATE_UNKNOWN".to_owned()),
    }
    if source
        .commit
        .as_ref()
        .is_none_or(|commit| commit.len() != 40)
    {
        blockers.push("FULL_COMMIT_UNKNOWN".to_owned());
    }
    if source.cargo_lock_sha256.is_none() {
        blockers.push("CARGO_LOCK_DIGEST_UNKNOWN".to_owned());
    }
    blockers
}

fn run_recent(connection: &Connection, chat_id: &str) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT id, ordinal, length(CAST(text AS BLOB))
         FROM messages
         WHERE chat_id = ?1 AND (ordinal < ?2 OR (ordinal = ?2 AND id < ?3))
         ORDER BY ordinal DESC, id DESC
         LIMIT 50",
    )?;
    let rows = statement.query_map(
        params![
            chat_id,
            9_007_199_254_740_991_i64,
            "ffffffffffffffffffffffffffffffff"
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )?;
    let mut count = 0_u64;
    for row in rows {
        black_box(row?);
        count += 1;
    }
    black_box(count);
    Ok(())
}

fn run_search(connection: &Connection, chat_id: &str) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT message.id, bm25(messages_fts)
         FROM messages_fts
         JOIN messages AS message ON message.row_id = messages_fts.rowid
         WHERE messages_fts MATCH 'LOREPIA' AND message.chat_id = ?1
         ORDER BY bm25(messages_fts), message.ordinal DESC, message.id DESC
         LIMIT 50",
    )?;
    let rows = statement.query_map([chat_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    let mut count = 0_u64;
    for row in rows {
        black_box(row?);
        count += 1;
    }
    black_box(count);
    Ok(())
}

fn run_active_path(connection: &Connection, chat_id: &str) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT path.position, message.id, length(CAST(message.text AS BLOB))
         FROM active_path AS path
         JOIN messages AS message
           ON message.chat_id = path.chat_id AND message.id = path.message_id
         WHERE path.chat_id = ?1 AND path.position > ?2
         ORDER BY path.position
         LIMIT 200",
    )?;
    let rows = statement.query_map(params![chat_id, -1_i64], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut count = 0_u64;
    for row in rows {
        black_box(row?);
        count += 1;
    }
    black_box(count);
    Ok(())
}

fn explain_recent(connection: &Connection, chat_id: &str) -> Result<Vec<String>> {
    explain(
        connection,
        "EXPLAIN QUERY PLAN
         SELECT id, ordinal FROM messages
         WHERE chat_id = ?1 AND (ordinal < ?2 OR (ordinal = ?2 AND id < ?3))
         ORDER BY ordinal DESC, id DESC LIMIT 50",
        params![
            chat_id,
            9_007_199_254_740_991_i64,
            "ffffffffffffffffffffffffffffffff"
        ],
    )
}

fn explain_search(connection: &Connection, chat_id: &str) -> Result<Vec<String>> {
    explain(
        connection,
        "EXPLAIN QUERY PLAN
         SELECT message.id, bm25(messages_fts)
         FROM messages_fts
         JOIN messages AS message ON message.row_id = messages_fts.rowid
         WHERE messages_fts MATCH 'LOREPIA' AND message.chat_id = ?1
         ORDER BY bm25(messages_fts), message.ordinal DESC, message.id DESC LIMIT 50",
        [chat_id],
    )
}

fn explain_active_path(connection: &Connection, chat_id: &str) -> Result<Vec<String>> {
    explain(
        connection,
        "EXPLAIN QUERY PLAN
         SELECT path.position, message.id
         FROM active_path AS path
         JOIN messages AS message
           ON message.chat_id = path.chat_id AND message.id = path.message_id
         WHERE path.chat_id = ?1 AND path.position > ?2
         ORDER BY path.position LIMIT 200",
        params![chat_id, -1_i64],
    )
}

fn explain<P: rusqlite::Params>(
    connection: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<String>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params, |row| row.get::<_, String>(3))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn measure<F>(operation: F) -> Result<u64>
where
    F: FnOnce() -> Result<()>,
{
    let started = Instant::now();
    operation()?;
    Ok(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX))
}

fn summarize(mut samples: Vec<u64>) -> LatencySummary {
    samples.sort_unstable();
    LatencySummary {
        samples: samples.len(),
        unit: "nanoseconds",
        p50: percentile(&samples, 50),
        p95: percentile(&samples, 95),
        p99: percentile(&samples, 99),
        max: samples.last().copied().unwrap_or(0),
    }
}

fn percentile(samples: &[u64], percentile: usize) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let rank = (samples.len() * percentile).div_ceil(100).max(1);
    samples[rank - 1]
}

fn sampled_rss_bytes() -> Option<u64> {
    if let Ok(status) = fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(value) = line.strip_prefix("VmRSS:") {
                let kib = value.split_whitespace().next()?.parse::<u64>().ok()?;
                return kib.checked_mul(1024);
            }
        }
    }
    let pid = std::process::id().to_string();
    command_output("ps", &["-o", "rss=", "-p", &pid])?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

fn source_state() -> SourceState {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let commit = git_output(&workspace, &["rev-parse", "HEAD"]);
    let dirty = git_output(
        &workspace,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .map(|status| !status.is_empty());
    let cargo_lock_sha256 = fs::read(workspace.join("Cargo.lock")).ok().map(|bytes| {
        let digest = Sha256::digest(bytes);
        format!("{digest:x}")
    });
    SourceState {
        commit,
        dirty,
        cargo_lock_sha256,
    }
}

fn git_output(workspace: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(arguments)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn command_output(command: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(command).args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{BranchProfile, DbOptions, generate as generate_db};

    #[test]
    fn percentile_uses_nearest_rank() {
        let samples = (1..=100).collect::<Vec<_>>();
        assert_eq!(percentile(&samples, 50), 50);
        assert_eq!(percentile(&samples, 95), 95);
        assert_eq!(percentile(&samples, 99), 99);
    }

    #[test]
    fn benchmark_emits_plans_sizes_environment_and_source_gate() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("fixture.sqlite3");
        let output = directory.path().join("bench.json");
        let database_receipt = directory.path().join("db-receipt.json");
        generate_db(DbOptions {
            messages: 30,
            target_text_bytes: 30_000,
            branch_profile: BranchProfile::Linear,
            seed: 42,
            output: database.clone(),
            receipt: Some(database_receipt),
        })
        .unwrap();
        run(BenchOptions {
            database,
            objects: None,
            seed: 42,
            warmup: 1,
            iterations: 3,
            output: output.clone(),
        })
        .unwrap();
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(receipt["cases"].as_array().unwrap().len(), 3);
        assert_eq!(receipt["cases"][0]["usesOffset"], false);
        assert!(receipt["environment"]["rustc"].is_string());
        assert_eq!(receipt["rawPromptOrSecretIncluded"], false);
        let blockers = receipt["releaseEvidenceBlockers"].as_array().unwrap();
        let dirty_blocked = blockers.iter().any(|blocker| blocker == "DIRTY_WORKTREE");
        match receipt["source"]["dirty"].as_bool() {
            Some(dirty) => assert_eq!(dirty_blocked, dirty),
            None => assert!(
                blockers
                    .iter()
                    .any(|blocker| blocker == "WORKTREE_STATE_UNKNOWN")
            ),
        }
        assert_eq!(receipt["releaseEvidenceEligible"], blockers.is_empty());
    }

    #[test]
    fn source_gate_is_deterministic_for_clean_dirty_and_unknown_states() {
        let clean = SourceState {
            commit: Some("a".repeat(40)),
            dirty: Some(false),
            cargo_lock_sha256: Some("b".repeat(64)),
        };
        assert!(release_evidence_blockers(&clean).is_empty());

        let dirty = SourceState {
            dirty: Some(true),
            ..clean.clone()
        };
        assert_eq!(release_evidence_blockers(&dirty), ["DIRTY_WORKTREE"]);

        let unknown = SourceState {
            commit: None,
            dirty: None,
            cargo_lock_sha256: None,
        };
        assert_eq!(
            release_evidence_blockers(&unknown),
            [
                "WORKTREE_STATE_UNKNOWN",
                "FULL_COMMIT_UNKNOWN",
                "CARGO_LOCK_DIGEST_UNKNOWN",
            ]
        );
    }
}
