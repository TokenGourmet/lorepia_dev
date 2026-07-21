use std::{fmt, path::PathBuf, str::FromStr};

use lorepia_storage::{CURRENT_SCHEMA_VERSION, MAX_MESSAGE_BYTES, Store, TimestampMillis};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::util::{
    MIB, Result, checked_sum_file_sizes, deterministic_id, emit_receipt, ensure_free_space,
    invalid, prepare_new_file, sqlite_sidecars,
};

const CHAT_NAMESPACE: u64 = 0x4348_4154_0000_0000;
const MESSAGE_NAMESPACE: u64 = 0x4d53_4700_0000_0000;
const CHARACTER_NAMESPACE: u64 = 0x4348_4152_0000_0000;
const INSERT_BATCH: usize = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchProfile {
    Linear,
    Comb,
    Fanout,
}

impl fmt::Display for BranchProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Linear => "linear",
            Self::Comb => "comb",
            Self::Fanout => "fanout",
        })
    }
}

impl FromStr for BranchProfile {
    type Err = crate::util::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "linear" => Ok(Self::Linear),
            "comb" => Ok(Self::Comb),
            "fanout" => Ok(Self::Fanout),
            _ => Err(invalid("--branch-profile must be linear, comb, or fanout")),
        }
    }
}

#[derive(Debug)]
pub struct DbOptions {
    pub messages: u64,
    pub target_text_bytes: u64,
    pub branch_profile: BranchProfile,
    pub seed: u64,
    pub output: PathBuf,
    pub receipt: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DbReceipt {
    artifact_kind: &'static str,
    tool_version: &'static str,
    schema_version: i64,
    seed: u64,
    branch_profile: BranchProfile,
    output: String,
    message_count: u64,
    target_text_bytes: u64,
    actual_text_bytes: u64,
    text_byte_tolerance: u64,
    maximum_message_bytes: u64,
    contains_cjk_fixture: bool,
    contains_long_fixture: bool,
    active_path_rows: u64,
    fts_rows: u64,
    database_and_sidecar_bytes: u64,
    preflight_required_bytes: u64,
    preflight_available_bytes: u64,
    bounded_insert_batch: usize,
    offset_queries_used: bool,
}

pub fn generate(options: DbOptions) -> Result<()> {
    let message_count = usize::try_from(options.messages)
        .map_err(|_| invalid("--messages exceeds this platform's addressable range"))?;
    if options.messages > 9_007_199_254_740_991 {
        return Err(invalid("--messages exceeds LorePia's safe integer range"));
    }
    if options.messages == 0 && options.target_text_bytes != 0 {
        return Err(invalid("--size must be zero when --messages is zero"));
    }
    let capacity = options
        .messages
        .checked_mul(MAX_MESSAGE_BYTES as u64)
        .ok_or_else(|| invalid("message text capacity overflowed"))?;
    if options.target_text_bytes > capacity {
        return Err(invalid(format!(
            "--size exceeds the schema limit: {} messages can hold at most {capacity} bytes",
            options.messages
        )));
    }

    let per_row_estimate = options
        .messages
        .checked_mul(2_048)
        .ok_or_else(|| invalid("database preflight estimate overflowed"))?;
    let preflight_required = options
        .target_text_bytes
        .checked_mul(5)
        .and_then(|bytes| bytes.checked_add(per_row_estimate))
        .and_then(|bytes| bytes.checked_add(64 * MIB))
        .ok_or_else(|| invalid("database preflight estimate overflowed"))?;
    let parent = options
        .output
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let preflight_available = ensure_free_space(parent, preflight_required)?;

    let (output, reservation) = prepare_new_file(&options.output)?;
    drop(reservation);

    let store = Store::open_at(&output, TimestampMillis::new(1)?)?;
    if store.startup_report().schema_version != CURRENT_SCHEMA_VERSION {
        return Err(invalid(
            "new database did not initialize at CURRENT_SCHEMA_VERSION",
        ));
    }

    let mut connection = Connection::open(&output)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    let schema_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema_version != CURRENT_SCHEMA_VERSION {
        return Err(invalid(format!(
            "database schema is {schema_version}, expected {CURRENT_SCHEMA_VERSION}"
        )));
    }

    let chat_id = deterministic_id(CHAT_NAMESPACE, options.seed, 0);
    let character_id = deterministic_id(CHARACTER_NAMESPACE, options.seed, 0);
    connection.execute(
        "INSERT INTO chats(id, character_id, title, revision, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, 'LorePia deterministic load fixture', 1, 1000, 1000)",
        params![chat_id, character_id],
    )?;

    let sizes = message_sizes(message_count, options.target_text_bytes)?;
    let mut maximum_message_bytes = 0_u64;
    for batch_start in (0..message_count).step_by(INSERT_BATCH) {
        let batch_end = (batch_start + INSERT_BATCH).min(message_count);
        let transaction = connection.transaction()?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO messages(
                    id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'complete', ?8, ?9, ?9, ?9)",
            )?;
            for (index, size) in sizes.iter().enumerate().take(batch_end).skip(batch_start) {
                let index_u64 = index as u64;
                let shape = message_shape(options.branch_profile, index_u64);
                let id = message_id(options.seed, index_u64);
                let parent_id = shape
                    .parent_index
                    .map(|parent| message_id(options.seed, parent));
                let text = synthetic_text(*size, options.seed, index_u64)?;
                maximum_message_bytes = maximum_message_bytes.max(text.len() as u64);
                let role = if index % 2 == 0 { "user" } else { "assistant" };
                let ordinal = index_u64 + 1;
                let timestamp = 1_000_u64
                    .checked_add(index_u64)
                    .ok_or_else(|| invalid("message timestamp overflowed"))?;
                insert.execute(params![
                    id,
                    chat_id,
                    parent_id,
                    to_sql_integer(shape.sibling_ordinal, "sibling ordinal")?,
                    to_sql_integer(shape.depth, "message depth")?,
                    to_sql_integer(ordinal, "message ordinal")?,
                    role,
                    text,
                    to_sql_integer(timestamp, "message timestamp")?,
                ])?;
            }
        }
        transaction.commit()?;
    }

    let active_indices = active_path_indices(options.branch_profile, options.messages);
    for batch in active_indices.chunks(INSERT_BATCH) {
        let transaction = connection.transaction()?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO active_path(chat_id, position, message_id) VALUES (?1, ?2, ?3)",
            )?;
            for (position, index) in batch.iter().enumerate() {
                let global_position = active_position(options.branch_profile, *index);
                debug_assert!(global_position >= position as u64 || batch.len() <= INSERT_BATCH);
                insert.execute(params![
                    chat_id,
                    to_sql_integer(global_position, "active path position")?,
                    message_id(options.seed, *index)
                ])?;
            }
        }
        transaction.commit()?;
    }

    let updated_at = 1_000_u64
        .checked_add(options.messages.saturating_sub(1))
        .ok_or_else(|| invalid("chat timestamp overflowed"))?;
    connection.execute(
        "UPDATE chats SET revision = 2, updated_at_ms = ?2 WHERE id = ?1",
        params![chat_id, to_sql_integer(updated_at, "chat timestamp")?],
    )?;

    validate_generated_database(&mut connection, options.messages, options.target_text_bytes)?;
    let active_path_rows = query_u64(
        &connection,
        "SELECT count(*) FROM active_path WHERE chat_id = ?1",
        [&chat_id],
    )?;
    let fts_rows = query_u64(&connection, "SELECT count(*) FROM messages_fts", [])?;
    connection.execute_batch("PRAGMA optimize; PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(connection);
    drop(store);

    let database_and_sidecar_bytes = checked_sum_file_sizes(&sqlite_sidecars(&output))?;
    let receipt = DbReceipt {
        artifact_kind: "LOREPIA_DETERMINISTIC_PRODUCT_SCHEMA_DATABASE",
        tool_version: env!("CARGO_PKG_VERSION"),
        schema_version: CURRENT_SCHEMA_VERSION,
        seed: options.seed,
        branch_profile: options.branch_profile,
        output: output.display().to_string(),
        message_count: options.messages,
        target_text_bytes: options.target_text_bytes,
        actual_text_bytes: options.target_text_bytes,
        text_byte_tolerance: 0,
        maximum_message_bytes,
        contains_cjk_fixture: maximum_message_bytes >= SYNTHETIC_PREFIX.len() as u64,
        contains_long_fixture: maximum_message_bytes >= 128 * 1024,
        active_path_rows,
        fts_rows,
        database_and_sidecar_bytes,
        preflight_required_bytes: preflight_required,
        preflight_available_bytes: preflight_available,
        bounded_insert_batch: INSERT_BATCH,
        offset_queries_used: false,
    };
    emit_receipt(options.receipt.as_deref(), &receipt)
}

const SYNTHETIC_PREFIX: &[u8] = "LOREPIA 합成中日 load fixture ".as_bytes();

fn synthetic_text(bytes: u64, seed: u64, index: u64) -> Result<String> {
    let length = usize::try_from(bytes)
        .map_err(|_| invalid("individual message size exceeds this platform's range"))?;
    if length > MAX_MESSAGE_BYTES {
        return Err(invalid("individual message exceeds MAX_MESSAGE_BYTES"));
    }
    let mut output = vec![b'x'; length];
    let prefix_length = output.len().min(SYNTHETIC_PREFIX.len());
    if prefix_length == SYNTHETIC_PREFIX.len() {
        output[..prefix_length].copy_from_slice(SYNTHETIC_PREFIX);
    } else {
        let ascii = b"LOREPIA-load-fixture-";
        let length = output.len().min(ascii.len());
        output[..length].copy_from_slice(&ascii[..length]);
    }
    for (offset, byte) in output[prefix_length..].iter_mut().enumerate() {
        let value = seed
            .wrapping_add(index.rotate_left(17))
            .wrapping_add(offset as u64);
        *byte = b'a' + (value % 26) as u8;
    }
    String::from_utf8(output).map_err(Into::into)
}

fn message_sizes(count: usize, target: u64) -> Result<Vec<u64>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut sizes = vec![0_u64; count];
    let max = MAX_MESSAGE_BYTES as u64;
    if target >= max {
        sizes[0] = max;
        distribute(&mut sizes[1..], target - max)?;
    } else {
        distribute(&mut sizes, target)?;
    }
    Ok(sizes)
}

fn distribute(slots: &mut [u64], total: u64) -> Result<()> {
    if slots.is_empty() {
        if total == 0 {
            return Ok(());
        }
        return Err(invalid(
            "text bytes cannot fit in the requested message count",
        ));
    }
    let count = slots.len() as u64;
    let quotient = total / count;
    let remainder = total % count;
    if quotient > MAX_MESSAGE_BYTES as u64
        || (quotient == MAX_MESSAGE_BYTES as u64 && remainder != 0)
    {
        return Err(invalid("text bytes exceed per-message schema limits"));
    }
    for (index, slot) in slots.iter_mut().enumerate() {
        *slot = quotient + u64::from((index as u64) < remainder);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MessageShape {
    parent_index: Option<u64>,
    sibling_ordinal: u64,
    depth: u64,
}

fn message_shape(profile: BranchProfile, index: u64) -> MessageShape {
    if index == 0 {
        return MessageShape {
            parent_index: None,
            sibling_ordinal: 1,
            depth: 0,
        };
    }
    match profile {
        BranchProfile::Linear => MessageShape {
            parent_index: Some(index - 1),
            sibling_ordinal: 1,
            depth: index,
        },
        BranchProfile::Fanout => MessageShape {
            parent_index: Some(0),
            sibling_ordinal: index,
            depth: 1,
        },
        BranchProfile::Comb => {
            let spine_parent = if index.is_multiple_of(2) {
                index - 2
            } else {
                index - 1
            };
            MessageShape {
                parent_index: Some(spine_parent),
                sibling_ordinal: if index.is_multiple_of(2) { 2 } else { 1 },
                depth: index.div_ceil(2),
            }
        }
    }
}

fn active_path_indices(profile: BranchProfile, count: u64) -> Vec<u64> {
    if count == 0 {
        return Vec::new();
    }
    match profile {
        BranchProfile::Linear => (0..count).collect(),
        BranchProfile::Fanout => {
            if count == 1 {
                vec![0]
            } else {
                vec![0, count - 1]
            }
        }
        BranchProfile::Comb => {
            let mut indices = (0..count).step_by(2).collect::<Vec<_>>();
            if (count - 1) % 2 == 1 {
                indices.push(count - 1);
            }
            indices
        }
    }
}

fn active_position(profile: BranchProfile, index: u64) -> u64 {
    match profile {
        BranchProfile::Linear => index,
        BranchProfile::Fanout => u64::from(index != 0),
        BranchProfile::Comb => index.div_ceil(2),
    }
}

fn message_id(seed: u64, index: u64) -> String {
    deterministic_id(MESSAGE_NAMESPACE, seed, index)
}

fn to_sql_integer(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| invalid(format!("{field} exceeds SQLite INTEGER")))
}

fn validate_generated_database(
    connection: &mut Connection,
    expected_messages: u64,
    expected_text_bytes: u64,
) -> Result<()> {
    let message_count = query_u64(connection, "SELECT count(*) FROM messages", [])?;
    let text_bytes = query_u64(
        connection,
        "SELECT coalesce(sum(length(CAST(text AS BLOB))), 0) FROM messages",
        [],
    )?;
    if message_count != expected_messages || text_bytes != expected_text_bytes {
        return Err(invalid("generated DB count/byte reconciliation failed"));
    }
    let foreign_key_failures = query_u64(
        connection,
        "SELECT count(*) FROM pragma_foreign_key_check",
        [],
    )?;
    if foreign_key_failures != 0 {
        return Err(invalid("generated DB contains foreign-key violations"));
    }
    let active_path_failures = query_u64(
        connection,
        "SELECT count(*)
         FROM active_path AS current
         JOIN messages AS message
           ON message.chat_id = current.chat_id AND message.id = current.message_id
         LEFT JOIN active_path AS previous
           ON previous.chat_id = current.chat_id AND previous.position = current.position - 1
         WHERE message.depth != current.position
            OR (current.position = 0 AND message.parent_id IS NOT NULL)
            OR (current.position > 0 AND (
                previous.message_id IS NULL OR message.parent_id != previous.message_id
            ))",
        [],
    )?;
    if active_path_failures != 0 {
        return Err(invalid("generated DB active_path reconciliation failed"));
    }
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1)",
        [],
    )?;
    transaction.rollback()?;
    Ok(())
}

fn query_u64<P>(connection: &Connection, sql: &str, params: P) -> Result<u64>
where
    P: rusqlite::Params,
{
    let value: i64 = connection.query_row(sql, params, |row| row.get(0))?;
    u64::try_from(value).map_err(|_| invalid("database returned a negative count"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_distribution_is_exact_and_bounded() {
        for count in [1, 2, 19] {
            let target = (count as u64 * 70_000).min(count as u64 * MAX_MESSAGE_BYTES as u64);
            let sizes = message_sizes(count, target).unwrap();
            assert_eq!(sizes.iter().sum::<u64>(), target);
            assert!(sizes.iter().all(|size| *size <= MAX_MESSAGE_BYTES as u64));
        }
        let long = message_sizes(3, MAX_MESSAGE_BYTES as u64 + 99).unwrap();
        assert_eq!(long[0], MAX_MESSAGE_BYTES as u64);
        assert_eq!(long.iter().sum::<u64>(), MAX_MESSAGE_BYTES as u64 + 99);
    }

    #[test]
    fn synthetic_text_has_exact_utf8_bytes_and_cjk() {
        let text = synthetic_text(1_000, 42, 7).unwrap();
        assert_eq!(text.len(), 1_000);
        assert!(text.contains('中'));
        assert!(text.contains('日'));
        assert_eq!(text, synthetic_text(1_000, 42, 7).unwrap());
    }

    #[test]
    fn branch_profiles_produce_contiguous_active_paths() {
        for profile in [
            BranchProfile::Linear,
            BranchProfile::Comb,
            BranchProfile::Fanout,
        ] {
            for count in 0..20 {
                let active = active_path_indices(profile, count);
                for (position, index) in active.iter().enumerate() {
                    assert_eq!(active_position(profile, *index), position as u64);
                    assert_eq!(message_shape(profile, *index).depth, position as u64);
                    if position > 0 {
                        assert_eq!(
                            message_shape(profile, *index).parent_index,
                            Some(active[position - 1])
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn generates_current_schema_database_and_refuses_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("fixture.sqlite3");
        let receipt = directory.path().join("db-receipt.json");
        generate(DbOptions {
            messages: 12,
            target_text_bytes: 7_000,
            branch_profile: BranchProfile::Comb,
            seed: 42,
            output: output.clone(),
            receipt: Some(receipt),
        })
        .unwrap();
        let connection = Connection::open(&output).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            connection
                .query_row::<i64, _, _>("SELECT count(*) FROM messages", [], |row| row.get(0))
                .unwrap(),
            12
        );
        assert!(
            generate(DbOptions {
                messages: 0,
                target_text_bytes: 0,
                branch_profile: BranchProfile::Linear,
                seed: 0,
                output,
                receipt: None,
            })
            .is_err()
        );
    }
}
