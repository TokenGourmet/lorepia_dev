use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use fs2::FileExt;
use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Row, Transaction, TransactionBehavior, backup, params,
};

use crate::{
    ActivePathEntry, ActivePathPage, ActivePathSelection, AppPreferences, AppendBranchMessage,
    AppendedBranchMessage, BeginTurn, BranchCursor, BranchPage, CachedRender, CharacterId, Chat,
    ChatCursor, ChatId, ChatPage, CreateChat, CumulativeAck, DefaultMode, DeliveryCheckpoint,
    EvictRenderCache, MAX_BRANCH_DEPTH, MAX_CHAT_TITLE_BYTES, MAX_CHECKPOINT_BYTES,
    MAX_MESSAGE_BYTES, MAX_MESSAGE_PAGE_BYTES, MAX_PAGE_SIZE, MAX_PROVIDER_RESPONSE_ID_BYTES,
    MAX_RENDER_CACHE_EVICTION, MAX_RENDERED_HTML_BYTES, MAX_SEARCH_QUERY_CHARS,
    MAX_SHORT_QUERY_SCAN_ROWS, MAX_USER_MESSAGE_BYTES, Message, MessageId, MessageOrdinalCursor,
    MessagePage, MessageRole, MessageSearchHit, MessageStatus, MessageTimelineCursor,
    MessageTimelinePage, ModelId, ProviderId, ProviderModelIds, ProviderSelection, PutRenderCache,
    RecentMessagePage, RenderCacheEviction, RendererVersion, RequestFailureCode, RequestState,
    RequestStateId, RequestStatus, ResponseCheckpoint, ResponseProgress, Result, SelectActivePath,
    StartedTurn, StartupReport, StorageError, StreamGeneration, StreamOwnerLabel,
    StreamSequenceProgress, TerminalCheckpoint, TerminalOutcome, Theme, TimestampMillis,
    TokenUsage, UpdatePreferences, WalCheckpointPolicy, WalCheckpointTelemetry,
    WalMaintenanceReport, migration,
};

#[derive(Clone, Debug)]
pub struct Store {
    database_path: PathBuf,
    startup_report: StartupReport,
    _lease: Arc<DatabaseLease>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_at(path, TimestampMillis::now()?)
    }

    pub fn open_at(path: impl AsRef<Path>, recovery_time: TimestampMillis) -> Result<Self> {
        let (lease, database_path) = DatabaseLease::acquire(path.as_ref())?;
        let startup_report = migration::initialize_database(&database_path, recovery_time)?;
        Ok(Self {
            database_path,
            startup_report,
            _lease: Arc::new(lease),
        })
    }

    pub fn startup_report(&self) -> &StartupReport {
        &self.startup_report
    }

    /// Creates a transactionally consistent SQLite snapshot while the operational database
    /// remains available to readers and writers.
    ///
    /// `target` must not already exist. The callback is invoked between bounded Online Backup
    /// API steps and may cancel the operation. A cancelled or failed target is removed.
    pub fn online_snapshot_to<C>(
        &self,
        target: impl AsRef<Path>,
        mut continue_copy: C,
    ) -> Result<u64>
    where
        C: FnMut(u64, u64) -> bool,
    {
        let target = target.as_ref();
        let parent = target.parent().ok_or(StorageError::InvalidInput {
            field: "snapshot path",
            reason: "must have a parent directory",
        })?;
        fs::create_dir_all(parent).map_err(StorageError::PathUnavailable)?;
        match fs::symlink_metadata(target) {
            Ok(_) => {
                return Err(StorageError::Conflict {
                    entity: "snapshot target",
                });
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(StorageError::PathUnavailable(error)),
        }

        let result = (|| {
            let source = self.connection()?;
            let mut destination = Connection::open(target)?;
            destination.pragma_update(None, "journal_mode", "DELETE")?;
            destination.pragma_update(None, "synchronous", "FULL")?;
            let snapshot = backup::Backup::new(&source, &mut destination)?;
            loop {
                let progress = snapshot.progress();
                let total = u64::try_from(progress.pagecount.max(0)).unwrap_or(0);
                let remaining = u64::try_from(progress.remaining.max(0)).unwrap_or(0);
                if !continue_copy(total.saturating_sub(remaining), total) {
                    return Err(StorageError::SnapshotCancelled);
                }
                match snapshot.step(128)? {
                    backup::StepResult::Done => break,
                    backup::StepResult::More => {}
                    backup::StepResult::Busy | backup::StepResult::Locked => {
                        std::thread::yield_now();
                    }
                    _ => std::thread::yield_now(),
                }
            }
            drop(snapshot);
            destination.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
            destination.pragma_update(None, "journal_mode", "DELETE")?;
            drop(destination);
            let file = File::open(target).map_err(StorageError::PathUnavailable)?;
            file.sync_all().map_err(StorageError::PathUnavailable)?;
            file.metadata()
                .map(|metadata| metadata.len())
                .map_err(StorageError::PathUnavailable)
        })();

        if result.is_err() {
            match fs::remove_file(target) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => {}
            }
        }
        result
    }

    /// Returns SQLite's current logical database size estimate (page count times page size).
    pub fn snapshot_size_estimate(&self) -> Result<u64> {
        let connection = self.connection()?;
        let page_count: i64 =
            connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
        let page_count =
            u64::try_from(page_count).map_err(|_| StorageError::IncompatibleSchema {
                reason: "SQLite page count is negative",
            })?;
        let page_size = u64::try_from(page_size).map_err(|_| StorageError::IncompatibleSchema {
            reason: "SQLite page size is negative",
        })?;
        page_count
            .checked_mul(page_size)
            .ok_or(StorageError::IncompatibleSchema {
                reason: "SQLite logical size overflowed",
            })
    }

    /// Performs the same exact schema-definition and row-invariant checks used before a
    /// migration, without modifying or recovering the snapshot.
    pub fn validate_snapshot_file(path: impl AsRef<Path>) -> Result<i64> {
        migration::validate_snapshot_file(path.as_ref())
    }

    pub fn create_chat(&self, input: CreateChat) -> Result<Chat> {
        validate_chat_title(&input.title)?;
        let chat = Chat {
            id: ChatId::new(),
            character_id: input.character_id,
            title: input.title,
            revision: 1,
            created_at_ms: input.at_ms,
            updated_at_ms: input.at_ms,
        };
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO chats(id, character_id, title, revision, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 1, ?4, ?4)",
                params![
                    chat.id.as_str(),
                    chat.character_id.as_str(),
                    chat.title,
                    chat.created_at_ms.get()
                ],
            )
            .map_err(|error| map_constraint(error, "chat"))?;
        Ok(chat)
    }

    pub fn get_chat(&self, chat_id: &ChatId) -> Result<Chat> {
        let connection = self.connection()?;
        let raw = connection
            .query_row(
                "SELECT id, character_id, title, revision, created_at_ms, updated_at_ms
                 FROM chats WHERE id = ?1",
                params![chat_id.as_str()],
                raw_chat,
            )
            .optional()?
            .ok_or(StorageError::NotFound { entity: "chat" })?;
        decode_chat(raw)
    }

    pub fn rename_chat(
        &self,
        chat_id: &ChatId,
        expected_revision: u64,
        title: impl Into<String>,
        at_ms: TimestampMillis,
    ) -> Result<Chat> {
        let title = title.into();
        validate_chat_title(&title)?;
        ensure_revision_can_advance(expected_revision, "chat revision")?;
        let expected_revision = encode_u64(expected_revision, "chat revision")?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE chats
             SET title = ?2, revision = revision + 1, updated_at_ms = max(updated_at_ms, ?4)
             WHERE id = ?1 AND revision = ?3",
            params![chat_id.as_str(), title, expected_revision, at_ms.get()],
        )?;
        if changed == 0 {
            ensure_chat_exists(&transaction, chat_id)?;
            return Err(StorageError::Conflict {
                entity: "chat revision",
            });
        }
        let raw = transaction.query_row(
            "SELECT id, character_id, title, revision, created_at_ms, updated_at_ms
             FROM chats WHERE id = ?1",
            params![chat_id.as_str()],
            raw_chat,
        )?;
        let chat = decode_chat(raw)?;
        transaction.commit()?;
        Ok(chat)
    }

    pub fn delete_chat(&self, chat_id: &ChatId) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_chat_exists(&transaction, chat_id)?;
        let running: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM request_state WHERE chat_id = ?1 AND status = 'running'
             )",
            params![chat_id.as_str()],
            |row| row.get(0),
        )?;
        if running {
            return Err(StorageError::Conflict {
                entity: "chat with an active request",
            });
        }
        let changed =
            transaction.execute("DELETE FROM chats WHERE id = ?1", params![chat_id.as_str()])?;
        if changed == 0 {
            return Err(StorageError::NotFound { entity: "chat" });
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_chats(&self, limit: u16, before: Option<&ChatCursor>) -> Result<ChatPage> {
        validate_page_size(limit)?;
        let connection = self.connection()?;
        let fetch_limit = i64::from(limit) + 1;
        let mut raw_chats = Vec::new();

        if let Some(cursor) = before {
            let mut statement = connection.prepare(
                "SELECT id, character_id, title, revision, created_at_ms, updated_at_ms
                 FROM chats
                 WHERE updated_at_ms < ?1 OR (updated_at_ms = ?1 AND id < ?2)
                 ORDER BY updated_at_ms DESC, id DESC
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(
                params![
                    cursor.updated_at_ms.get(),
                    cursor.chat_id.as_str(),
                    fetch_limit
                ],
                raw_chat,
            )?;
            for row in rows {
                raw_chats.push(row?);
            }
        } else {
            let mut statement = connection.prepare(
                "SELECT id, character_id, title, revision, created_at_ms, updated_at_ms
                 FROM chats
                 ORDER BY updated_at_ms DESC, id DESC
                 LIMIT ?1",
            )?;
            let rows = statement.query_map(params![fetch_limit], raw_chat)?;
            for row in rows {
                raw_chats.push(row?);
            }
        }

        let has_more = raw_chats.len() > usize::from(limit);
        raw_chats.truncate(usize::from(limit));
        let chats = raw_chats
            .into_iter()
            .map(decode_chat)
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if has_more {
            chats.last().map(|chat| ChatCursor {
                updated_at_ms: chat.updated_at_ms,
                chat_id: chat.id.clone(),
            })
        } else {
            None
        };
        Ok(ChatPage { chats, next_cursor })
    }

    pub fn begin_turn(&self, input: BeginTurn) -> Result<StartedTurn> {
        validate_user_message(&input.user_text)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let chat_created_at: i64 = transaction
            .query_row(
                "SELECT created_at_ms FROM chats WHERE id = ?1",
                params![input.chat_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::NotFound { entity: "chat" })?;
        if input.started_at_ms < TimestampMillis::new(chat_created_at)? {
            return Err(StorageError::InvalidInput {
                field: "turn timestamp",
                reason: "must not predate the chat",
            });
        }

        let last_ordinal: Option<i64> = transaction.query_row(
            "SELECT max(ordinal) FROM messages WHERE chat_id = ?1",
            params![input.chat_id.as_str()],
            |row| row.get(0),
        )?;
        let user_ordinal_i64 =
            last_ordinal
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(StorageError::IncompatibleSchema {
                    reason: "message ordinal overflowed",
                })?;
        let assistant_ordinal_i64 =
            user_ordinal_i64
                .checked_add(1)
                .ok_or(StorageError::IncompatibleSchema {
                    reason: "message ordinal overflowed",
                })?;
        let user_ordinal = decode_u64(user_ordinal_i64, "message ordinal")?;
        let assistant_ordinal = decode_u64(assistant_ordinal_i64, "message ordinal")?;

        let active_leaf = current_active_leaf(&transaction, &input.chat_id)?;
        if last_ordinal.is_some() != active_leaf.is_some() {
            return Err(StorageError::IncompatibleSchema {
                reason: "chat messages and active path disagree",
            });
        }
        let (user_parent_id, user_depth) = match active_leaf {
            Some((message_id, depth)) => {
                let next_depth = depth.checked_add(1).ok_or(StorageError::InvalidInput {
                    field: "message depth",
                    reason: "exceeds the branch depth limit",
                })?;
                (Some(message_id), next_depth)
            }
            None => (None, 0),
        };
        let assistant_depth = user_depth
            .checked_add(1)
            .ok_or(StorageError::InvalidInput {
                field: "message depth",
                reason: "exceeds the branch depth limit",
            })?;
        validate_branch_depth(assistant_depth)?;
        let user_sibling_ord =
            next_sibling_ordinal(&transaction, &input.chat_id, user_parent_id.as_ref())?;

        let request_state_id = RequestStateId::new();
        let user_message_id = MessageId::new();
        let assistant_message_id = MessageId::new();
        let at_ms = input.started_at_ms.get();

        transaction
            .execute(
                "INSERT INTO messages(
                    id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    'user', 'complete', ?7, ?8, ?8, ?8
                 )",
                params![
                    user_message_id.as_str(),
                    input.chat_id.as_str(),
                    user_parent_id.as_ref().map(MessageId::as_str),
                    encode_u64(user_sibling_ord, "message sibling ordinal")?,
                    encode_u64(user_depth, "message depth")?,
                    user_ordinal_i64,
                    input.user_text,
                    at_ms
                ],
            )
            .map_err(|error| map_constraint(error, "message"))?;
        transaction
            .execute(
                "INSERT INTO messages(
                    id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 ) VALUES (
                    ?1, ?2, ?3, 1, ?4, ?5,
                    'assistant', 'partial', '', ?6, ?6, NULL
                 )",
                params![
                    assistant_message_id.as_str(),
                    input.chat_id.as_str(),
                    user_message_id.as_str(),
                    encode_u64(assistant_depth, "message depth")?,
                    assistant_ordinal_i64,
                    at_ms
                ],
            )
            .map_err(|error| map_constraint(error, "message"))?;
        transaction.execute(
            "INSERT INTO active_path(chat_id, position, message_id)
             VALUES (?1, ?2, ?3), (?1, ?4, ?5)",
            params![
                input.chat_id.as_str(),
                encode_u64(user_depth, "active path position")?,
                user_message_id.as_str(),
                encode_u64(assistant_depth, "active path position")?,
                assistant_message_id.as_str()
            ],
        )?;
        transaction
            .execute(
                "INSERT INTO request_state(
                    id, chat_id, user_message_id, assistant_message_id,
                    provider_id, model_id, owner_label, stream_generation,
                    status, last_delivered_seq, last_durable_seq, last_acked_seq,
                    started_at_ms, updated_at_ms
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    'running', 0, 0, NULL, ?9, ?9
                 )",
                params![
                    request_state_id.as_str(),
                    input.chat_id.as_str(),
                    user_message_id.as_str(),
                    assistant_message_id.as_str(),
                    input.selection.provider_id.as_str(),
                    input.selection.model_id.as_str(),
                    input.owner_label.as_str(),
                    input.stream_generation.as_str(),
                    at_ms
                ],
            )
            .map_err(|error| map_constraint(error, "active request"))?;
        transaction.execute(
            "UPDATE chats SET updated_at_ms = max(updated_at_ms, ?2) WHERE id = ?1",
            params![input.chat_id.as_str(), at_ms],
        )?;
        transaction.commit()?;

        Ok(StartedTurn {
            request_state_id,
            user_message_id,
            assistant_message_id,
            user_ordinal,
            assistant_ordinal,
            owner_label: input.owner_label,
            stream_generation: input.stream_generation,
            last_delivered_seq: 0,
            last_durable_seq: 0,
            last_acked_seq: None,
        })
    }

    pub fn record_response_delivery(
        &self,
        checkpoint: DeliveryCheckpoint,
    ) -> Result<StreamSequenceProgress> {
        validate_delivery_checkpoint(&checkpoint)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load_request_state(&transaction, &checkpoint.request_state_id)?;
        ensure_running_and_owned(
            &state,
            &checkpoint.owner_label,
            &checkpoint.stream_generation,
        )?;
        let effective_at_ms = effective_stream_timestamp(&state, checkpoint.at_ms);
        if state.last_delivered_seq != checkpoint.expected_last_delivered_seq {
            return Err(StorageError::SequenceMismatch {
                expected: state.last_delivered_seq,
                actual: checkpoint.expected_last_delivered_seq,
            });
        }

        let changed = transaction.execute(
            "UPDATE request_state
             SET last_delivered_seq = ?2,
                 updated_at_ms = max(updated_at_ms, ?3)
             WHERE id = ?1
               AND owner_label = ?4
               AND stream_generation = ?5
               AND status = 'running'
               AND last_delivered_seq = ?6",
            params![
                checkpoint.request_state_id.as_str(),
                encode_u64(checkpoint.through_seq, "delivered sequence")?,
                effective_at_ms.get(),
                checkpoint.owner_label.as_str(),
                checkpoint.stream_generation.as_str(),
                encode_u64(checkpoint.expected_last_delivered_seq, "delivered sequence")?
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::SequenceMismatch {
                expected: state.last_delivered_seq,
                actual: checkpoint.expected_last_delivered_seq,
            });
        }
        let progress = StreamSequenceProgress {
            request_state_id: state.id,
            owner_label: state.owner_label,
            stream_generation: state.stream_generation,
            last_delivered_seq: checkpoint.through_seq,
            last_durable_seq: state.last_durable_seq,
            last_acked_seq: state.last_acked_seq,
            status: state.status,
            updated_at_ms: effective_at_ms,
        };
        transaction.commit()?;
        Ok(progress)
    }

    pub fn acknowledge_response(
        &self,
        acknowledgement: CumulativeAck,
    ) -> Result<StreamSequenceProgress> {
        validate_cumulative_ack(&acknowledgement)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load_request_state(&transaction, &acknowledgement.request_state_id)?;
        ensure_stream_owned(
            &state,
            &acknowledgement.owner_label,
            &acknowledgement.stream_generation,
        )?;
        let effective_at_ms = effective_stream_timestamp(&state, acknowledgement.at_ms);
        if state.last_acked_seq != acknowledgement.expected_last_acked_seq {
            return Err(StorageError::SequenceMismatch {
                expected: state.last_acked_seq.unwrap_or(0),
                actual: acknowledgement.expected_last_acked_seq.unwrap_or(0),
            });
        }
        if acknowledgement.through_seq > state.last_durable_seq {
            return Err(StorageError::InvalidInput {
                field: "acknowledged sequence",
                reason: "must not advance beyond the durable sequence",
            });
        }

        let changed = transaction.execute(
            "UPDATE request_state
             SET last_acked_seq = ?2,
                 updated_at_ms = max(updated_at_ms, ?3)
             WHERE id = ?1
               AND owner_label = ?4
               AND stream_generation = ?5
               AND last_acked_seq IS ?6",
            params![
                acknowledgement.request_state_id.as_str(),
                encode_u64(acknowledgement.through_seq, "acknowledged sequence")?,
                effective_at_ms.get(),
                acknowledgement.owner_label.as_str(),
                acknowledgement.stream_generation.as_str(),
                acknowledgement
                    .expected_last_acked_seq
                    .map(|sequence| encode_u64(sequence, "acknowledged sequence"))
                    .transpose()?
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::SequenceMismatch {
                expected: state.last_acked_seq.unwrap_or(0),
                actual: acknowledgement.expected_last_acked_seq.unwrap_or(0),
            });
        }
        let progress = StreamSequenceProgress {
            request_state_id: state.id,
            owner_label: state.owner_label,
            stream_generation: state.stream_generation,
            last_delivered_seq: state.last_delivered_seq,
            last_durable_seq: state.last_durable_seq,
            last_acked_seq: Some(acknowledgement.through_seq),
            status: state.status,
            updated_at_ms: effective_at_ms,
        };
        transaction.commit()?;
        Ok(progress)
    }

    pub fn checkpoint_response(&self, checkpoint: ResponseCheckpoint) -> Result<ResponseProgress> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let progress = apply_response_progress(&transaction, &checkpoint, None)?;
        transaction.commit()?;
        Ok(progress)
    }

    pub fn finish_turn(&self, terminal: TerminalCheckpoint) -> Result<ResponseProgress> {
        if terminal.outcome == TerminalOutcome::Failed(RequestFailureCode::AppRestarted) {
            return Err(StorageError::InvalidInput {
                field: "failure code",
                reason: "APP_RESTARTED is reserved for startup recovery",
            });
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let progress =
            apply_response_progress(&transaction, &terminal.checkpoint, Some(terminal.outcome))?;
        transaction.commit()?;
        Ok(progress)
    }

    pub fn complete_turn(&self, checkpoint: ResponseCheckpoint) -> Result<ResponseProgress> {
        self.finish_turn(TerminalCheckpoint {
            checkpoint,
            outcome: TerminalOutcome::Completed,
        })
    }

    pub fn cancel_turn(&self, checkpoint: ResponseCheckpoint) -> Result<ResponseProgress> {
        self.finish_turn(TerminalCheckpoint {
            checkpoint,
            outcome: TerminalOutcome::Cancelled,
        })
    }

    pub fn fail_turn(
        &self,
        checkpoint: ResponseCheckpoint,
        code: RequestFailureCode,
    ) -> Result<ResponseProgress> {
        self.finish_turn(TerminalCheckpoint {
            checkpoint,
            outcome: TerminalOutcome::Failed(code),
        })
    }

    pub fn get_request_state(&self, request_state_id: &RequestStateId) -> Result<RequestState> {
        let connection = self.connection()?;
        load_request_state(&connection, request_state_id)
    }

    pub fn load_messages(
        &self,
        chat_id: &ChatId,
        after_ordinal: Option<u64>,
        limit: u16,
    ) -> Result<MessagePage> {
        validate_page_size(limit)?;
        let connection = self.connection()?;
        ensure_chat_exists(&connection, chat_id)?;
        let after = after_ordinal
            .map(|value| encode_u64(value, "message ordinal"))
            .transpose()?
            .unwrap_or(0);
        let fetch_limit = i64::from(limit) + 1;
        let mut statement = connection.prepare(
            "SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
             FROM messages
             WHERE chat_id = ?1 AND ordinal > ?2
             ORDER BY ordinal ASC
             LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![chat_id.as_str(), after, fetch_limit], raw_message)?;
        let mut raw_messages = Vec::new();
        for row in rows {
            raw_messages.push(row?);
        }
        let has_more = raw_messages.len() > usize::from(limit);
        raw_messages.truncate(usize::from(limit));
        let messages = raw_messages
            .into_iter()
            .map(decode_message)
            .collect::<Result<Vec<_>>>()?;
        let next_ordinal = if has_more {
            messages.last().map(|message| message.ordinal)
        } else {
            None
        };
        Ok(MessagePage {
            messages,
            next_ordinal,
        })
    }

    /// Loads one bounded window starting at the newest eligible message.
    ///
    /// The external cursor is the stable `(chat_id, ordinal)` identity. SQLite
    /// resolves it to the current active-path position, walks the active-path
    /// primary key backwards, and never admits inactive branch siblings. The
    /// result is reversed in memory so callers always receive canonical
    /// ascending display order. `before` is exclusive and the returned cursor
    /// points at the oldest returned message when another older page exists.
    pub fn load_recent_messages(
        &self,
        chat_id: &ChatId,
        before: Option<&MessageOrdinalCursor>,
        limit: u16,
    ) -> Result<RecentMessagePage> {
        validate_page_size(limit)?;
        let connection = self.connection()?;
        ensure_chat_exists(&connection, chat_id)?;
        let fetch_limit = i64::from(limit) + 1;
        let before_position = if let Some(cursor) = before {
            if &cursor.chat_id != chat_id {
                return Err(StorageError::InvalidInput {
                    field: "message cursor chat ID",
                    reason: "must match the requested chat",
                });
            }
            let before_ordinal = encode_u64(cursor.ordinal, "message cursor ordinal")?;
            if before_ordinal == 0 {
                return Err(StorageError::InvalidInput {
                    field: "message cursor ordinal",
                    reason: "must be at least 1",
                });
            }
            let position = connection
                .query_row(
                    "SELECT p.position
                     FROM messages AS m
                     JOIN active_path AS p
                       ON p.chat_id = m.chat_id AND p.message_id = m.id
                     WHERE m.chat_id = ?1 AND m.ordinal = ?2",
                    params![chat_id.as_str(), before_ordinal],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .ok_or(StorageError::InvalidInput {
                    field: "message cursor ordinal",
                    reason: "must identify the current active path",
                })?;
            Some(position)
        } else {
            None
        };

        if let Some(before_position) = before_position {
            let mut statement = connection.prepare(
                "SELECT m.id, m.chat_id, m.parent_id, m.sibling_ord, m.depth, m.ordinal,
                        m.role, m.status, m.text,
                        m.created_at_ms, m.updated_at_ms, m.completed_at_ms
                 FROM active_path AS p
                 JOIN messages AS m ON m.chat_id = p.chat_id AND m.id = p.message_id
                 WHERE p.chat_id = ?1 AND p.position < ?2
                 ORDER BY p.position DESC
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(
                params![chat_id.as_str(), before_position, fetch_limit],
                raw_message,
            )?;
            let bounded = collect_bounded_message_rows(rows, limit)?;
            let raw_messages = bounded.0;
            let stopped_early = bounded.1;
            finish_recent_message_page(chat_id, raw_messages, stopped_early)
        } else {
            let mut statement = connection.prepare(
                "SELECT m.id, m.chat_id, m.parent_id, m.sibling_ord, m.depth, m.ordinal,
                        m.role, m.status, m.text,
                        m.created_at_ms, m.updated_at_ms, m.completed_at_ms
                 FROM active_path AS p
                 JOIN messages AS m ON m.chat_id = p.chat_id AND m.id = p.message_id
                 WHERE p.chat_id = ?1
                 ORDER BY p.position DESC
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(params![chat_id.as_str(), fetch_limit], raw_message)?;
            let bounded = collect_bounded_message_rows(rows, limit)?;
            let raw_messages = bounded.0;
            let stopped_early = bounded.1;
            finish_recent_message_page(chat_id, raw_messages, stopped_early)
        }
    }

    pub fn append_branch_message(
        &self,
        input: AppendBranchMessage,
    ) -> Result<AppendedBranchMessage> {
        validate_complete_message(input.role, &input.text)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let chat_created_at: i64 = transaction
            .query_row(
                "SELECT created_at_ms FROM chats WHERE id = ?1",
                params![input.chat_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::NotFound { entity: "chat" })?;
        if input.at_ms < TimestampMillis::new(chat_created_at)? {
            return Err(StorageError::InvalidInput {
                field: "message timestamp",
                reason: "must not predate the chat",
            });
        }
        ensure_expected_active_leaf(
            &transaction,
            &input.chat_id,
            input.expected_active_leaf_id.as_ref(),
        )?;

        let depth = match input.parent_id.as_ref() {
            Some(parent_id) => {
                let parent_depth = message_depth(&transaction, &input.chat_id, parent_id)?;
                parent_depth
                    .checked_add(1)
                    .ok_or(StorageError::InvalidInput {
                        field: "message depth",
                        reason: "exceeds the branch depth limit",
                    })?
            }
            None => 0,
        };
        validate_branch_depth(depth)?;
        let sibling_ord =
            next_sibling_ordinal(&transaction, &input.chat_id, input.parent_id.as_ref())?;
        let ordinal = next_message_ordinal(&transaction, &input.chat_id)?;
        let message_id = MessageId::new();
        transaction
            .execute(
                "INSERT INTO messages(
                    id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, 'complete', ?8, ?9, ?9, ?9
                 )",
                params![
                    message_id.as_str(),
                    input.chat_id.as_str(),
                    input.parent_id.as_ref().map(MessageId::as_str),
                    encode_u64(sibling_ord, "message sibling ordinal")?,
                    encode_u64(depth, "message depth")?,
                    encode_u64(ordinal, "message ordinal")?,
                    input.role.as_str(),
                    input.text,
                    input.at_ms.get()
                ],
            )
            .map_err(|error| map_constraint(error, "branch message"))?;
        let path_length = replace_active_path(&transaction, &input.chat_id, &message_id)?;
        transaction.execute(
            "UPDATE chats SET updated_at_ms = max(updated_at_ms, ?2) WHERE id = ?1",
            params![input.chat_id.as_str(), input.at_ms.get()],
        )?;
        let raw = query_message(&transaction, &message_id)?;
        let message = decode_message(raw)?;
        let active_path = ActivePathSelection {
            chat_id: input.chat_id,
            leaf_message_id: message_id,
            path_length,
        };
        transaction.commit()?;
        Ok(AppendedBranchMessage {
            message,
            active_path,
        })
    }

    pub fn select_active_path(&self, input: SelectActivePath) -> Result<ActivePathSelection> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_chat_exists(&transaction, &input.chat_id)?;
        ensure_expected_active_leaf(
            &transaction,
            &input.chat_id,
            input.expected_leaf_id.as_ref(),
        )?;
        let path_length =
            replace_active_path(&transaction, &input.chat_id, &input.leaf_message_id)?;
        transaction.execute(
            "UPDATE chats SET updated_at_ms = max(updated_at_ms, ?2) WHERE id = ?1",
            params![input.chat_id.as_str(), input.at_ms.get()],
        )?;
        let selection = ActivePathSelection {
            chat_id: input.chat_id,
            leaf_message_id: input.leaf_message_id,
            path_length,
        };
        transaction.commit()?;
        Ok(selection)
    }

    pub fn load_active_path(
        &self,
        chat_id: &ChatId,
        after_position: Option<u64>,
        limit: u16,
    ) -> Result<ActivePathPage> {
        validate_page_size(limit)?;
        let connection = self.connection()?;
        ensure_chat_exists(&connection, chat_id)?;
        let fetch_limit = i64::from(limit) + 1;
        let mut raw_entries = Vec::new();
        if let Some(after) = after_position {
            let mut statement = connection.prepare(
                "SELECT p.position,
                        m.id, m.chat_id, m.parent_id, m.sibling_ord, m.depth, m.ordinal,
                        m.role, m.status, m.text,
                        m.created_at_ms, m.updated_at_ms, m.completed_at_ms
                 FROM active_path AS p
                 JOIN messages AS m ON m.chat_id = p.chat_id AND m.id = p.message_id
                 WHERE p.chat_id = ?1 AND p.position > ?2
                 ORDER BY p.position ASC
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(
                params![
                    chat_id.as_str(),
                    encode_u64(after, "active path position")?,
                    fetch_limit
                ],
                raw_active_path_entry,
            )?;
            for row in rows {
                raw_entries.push(row?);
            }
        } else {
            let mut statement = connection.prepare(
                "SELECT p.position,
                        m.id, m.chat_id, m.parent_id, m.sibling_ord, m.depth, m.ordinal,
                        m.role, m.status, m.text,
                        m.created_at_ms, m.updated_at_ms, m.completed_at_ms
                 FROM active_path AS p
                 JOIN messages AS m ON m.chat_id = p.chat_id AND m.id = p.message_id
                 WHERE p.chat_id = ?1
                 ORDER BY p.position ASC
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(
                params![chat_id.as_str(), fetch_limit],
                raw_active_path_entry,
            )?;
            for row in rows {
                raw_entries.push(row?);
            }
        }
        let has_more = raw_entries.len() > usize::from(limit);
        raw_entries.truncate(usize::from(limit));
        let entries = raw_entries
            .into_iter()
            .map(decode_active_path_entry)
            .collect::<Result<Vec<_>>>()?;
        let next_position = has_more
            .then(|| entries.last().map(|entry| entry.position))
            .flatten();
        Ok(ActivePathPage {
            entries,
            next_position,
        })
    }

    pub fn load_branch_children(
        &self,
        chat_id: &ChatId,
        parent_id: Option<&MessageId>,
        after: Option<&BranchCursor>,
        limit: u16,
    ) -> Result<BranchPage> {
        validate_page_size(limit)?;
        let connection = self.connection()?;
        ensure_chat_exists(&connection, chat_id)?;
        if let Some(parent_id) = parent_id {
            message_depth(&connection, chat_id, parent_id)?;
        }
        let fetch_limit = i64::from(limit) + 1;
        let mut raw_messages = Vec::new();
        let parent = parent_id.map(MessageId::as_str);
        match (parent, after) {
            (Some(parent), Some(cursor)) => {
                let mut statement = connection.prepare(
                    "SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                            role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                     FROM messages
                     WHERE chat_id = ?1 AND parent_id = ?2
                       AND (sibling_ord > ?3 OR (sibling_ord = ?3 AND id > ?4))
                     ORDER BY sibling_ord ASC, id ASC
                     LIMIT ?5",
                )?;
                let rows = statement.query_map(
                    params![
                        chat_id.as_str(),
                        parent,
                        encode_u64(cursor.sibling_ord, "message sibling ordinal")?,
                        cursor.message_id.as_str(),
                        fetch_limit
                    ],
                    raw_message,
                )?;
                for row in rows {
                    raw_messages.push(row?);
                }
            }
            (Some(parent), None) => {
                let mut statement = connection.prepare(
                    "SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                            role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                     FROM messages
                     WHERE chat_id = ?1 AND parent_id = ?2
                     ORDER BY sibling_ord ASC, id ASC
                     LIMIT ?3",
                )?;
                let rows = statement
                    .query_map(params![chat_id.as_str(), parent, fetch_limit], raw_message)?;
                for row in rows {
                    raw_messages.push(row?);
                }
            }
            (None, Some(cursor)) => {
                let mut statement = connection.prepare(
                    "SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                            role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                     FROM messages
                     WHERE chat_id = ?1 AND parent_id IS NULL
                       AND (sibling_ord > ?2 OR (sibling_ord = ?2 AND id > ?3))
                     ORDER BY sibling_ord ASC, id ASC
                     LIMIT ?4",
                )?;
                let rows = statement.query_map(
                    params![
                        chat_id.as_str(),
                        encode_u64(cursor.sibling_ord, "message sibling ordinal")?,
                        cursor.message_id.as_str(),
                        fetch_limit
                    ],
                    raw_message,
                )?;
                for row in rows {
                    raw_messages.push(row?);
                }
            }
            (None, None) => {
                let mut statement = connection.prepare(
                    "SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                            role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                     FROM messages
                     WHERE chat_id = ?1 AND parent_id IS NULL
                     ORDER BY sibling_ord ASC, id ASC
                     LIMIT ?2",
                )?;
                let rows =
                    statement.query_map(params![chat_id.as_str(), fetch_limit], raw_message)?;
                for row in rows {
                    raw_messages.push(row?);
                }
            }
        }
        let has_more = raw_messages.len() > usize::from(limit);
        raw_messages.truncate(usize::from(limit));
        let messages = raw_messages
            .into_iter()
            .map(decode_message)
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = has_more.then(|| {
            messages.last().map(|message| BranchCursor {
                sibling_ord: message.sibling_ord,
                message_id: message.id.clone(),
            })
        });
        Ok(BranchPage {
            messages,
            next_cursor: next_cursor.flatten(),
        })
    }

    pub fn load_message_timeline(
        &self,
        chat_id: &ChatId,
        after: Option<&MessageTimelineCursor>,
        limit: u16,
    ) -> Result<MessageTimelinePage> {
        validate_page_size(limit)?;
        let connection = self.connection()?;
        ensure_chat_exists(&connection, chat_id)?;
        let fetch_limit = i64::from(limit) + 1;
        let mut raw_messages = Vec::new();
        if let Some(cursor) = after {
            let mut statement = connection.prepare(
                "SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                        role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 FROM messages
                 WHERE chat_id = ?1
                   AND (created_at_ms > ?2 OR (created_at_ms = ?2 AND id > ?3))
                 ORDER BY created_at_ms ASC, id ASC
                 LIMIT ?4",
            )?;
            let rows = statement.query_map(
                params![
                    chat_id.as_str(),
                    cursor.created_at_ms.get(),
                    cursor.message_id.as_str(),
                    fetch_limit
                ],
                raw_message,
            )?;
            for row in rows {
                raw_messages.push(row?);
            }
        } else {
            let mut statement = connection.prepare(
                "SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                        role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 FROM messages
                 WHERE chat_id = ?1
                 ORDER BY created_at_ms ASC, id ASC
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(params![chat_id.as_str(), fetch_limit], raw_message)?;
            for row in rows {
                raw_messages.push(row?);
            }
        }
        let has_more = raw_messages.len() > usize::from(limit);
        raw_messages.truncate(usize::from(limit));
        let messages = raw_messages
            .into_iter()
            .map(decode_message)
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = has_more.then(|| {
            messages.last().map(|message| MessageTimelineCursor {
                created_at_ms: message.created_at_ms,
                message_id: message.id.clone(),
            })
        });
        Ok(MessageTimelinePage {
            messages,
            next_cursor: next_cursor.flatten(),
        })
    }

    pub fn search_messages(
        &self,
        chat_id: &ChatId,
        query: &str,
        limit: u16,
    ) -> Result<Vec<MessageSearchHit>> {
        validate_page_size(limit)?;
        validate_search_query(query)?;
        let connection = self.connection()?;
        ensure_chat_exists(&connection, chat_id)?;
        let mut raw_messages = Vec::new();

        if query.chars().count() < 3 {
            let mut statement = connection.prepare(
                "WITH candidates AS (
                    SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                           role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                    FROM messages
                    WHERE chat_id = ?1 AND status = 'complete'
                    ORDER BY ordinal DESC
                    LIMIT ?4
                 )
                 SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                        role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 FROM candidates
                 WHERE instr(text, ?2) > 0
                 ORDER BY ordinal DESC
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(
                params![
                    chat_id.as_str(),
                    query,
                    i64::from(limit),
                    i64::from(MAX_SHORT_QUERY_SCAN_ROWS)
                ],
                raw_message,
            )?;
            for row in rows {
                raw_messages.push(row?);
            }
        } else {
            let phrase = format!("\"{}\"", query.replace('"', "\"\""));
            let mut statement = connection.prepare(
                "SELECT m.id, m.chat_id, m.parent_id, m.sibling_ord, m.depth, m.ordinal,
                        m.role, m.status, m.text,
                        m.created_at_ms, m.updated_at_ms, m.completed_at_ms
                 FROM messages_fts f
                 JOIN messages m ON m.row_id = f.rowid
                 WHERE messages_fts MATCH ?2 AND m.chat_id = ?1
                 ORDER BY bm25(messages_fts), m.ordinal DESC, m.id DESC
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(
                params![chat_id.as_str(), phrase, i64::from(limit)],
                raw_message,
            )?;
            for row in rows {
                raw_messages.push(row?);
            }
        }

        raw_messages
            .into_iter()
            .map(decode_message)
            .map(|result| result.map(|message| MessageSearchHit { message }))
            .collect()
    }

    pub fn put_render_cache(&self, input: PutRenderCache) -> Result<CachedRender> {
        validate_rendered_html(&input.html)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let message_created_at: i64 = transaction
            .query_row(
                "SELECT created_at_ms FROM messages WHERE id = ?1",
                params![input.message_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::NotFound { entity: "message" })?;
        if input.at_ms < TimestampMillis::new(message_created_at)? {
            return Err(StorageError::InvalidInput {
                field: "render cache timestamp",
                reason: "must not predate the message",
            });
        }

        match input.expected {
            None => {
                transaction
                    .execute(
                        "INSERT INTO message_render_cache(
                            message_id, renderer_ver, html, last_used_at_ms
                         ) VALUES (?1, ?2, ?3, ?4)",
                        params![
                            input.message_id.as_str(),
                            encode_u64(input.renderer_version.get(), "renderer version")?,
                            input.html,
                            input.at_ms.get()
                        ],
                    )
                    .map_err(|error| map_constraint(error, "render cache"))?;
            }
            Some(expected) => {
                if input.at_ms < expected.last_used_at_ms {
                    return Err(StorageError::InvalidInput {
                        field: "render cache timestamp",
                        reason: "must not move backwards",
                    });
                }
                let changed = transaction.execute(
                    "UPDATE message_render_cache
                     SET renderer_ver = ?2, html = ?3, last_used_at_ms = ?4
                     WHERE message_id = ?1
                       AND renderer_ver = ?5
                       AND last_used_at_ms = ?6",
                    params![
                        input.message_id.as_str(),
                        encode_u64(input.renderer_version.get(), "renderer version")?,
                        input.html,
                        input.at_ms.get(),
                        encode_u64(expected.renderer_version.get(), "renderer version")?,
                        expected.last_used_at_ms.get()
                    ],
                )?;
                if changed != 1 {
                    let exists: bool = transaction.query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM message_render_cache WHERE message_id = ?1
                         )",
                        params![input.message_id.as_str()],
                        |row| row.get(0),
                    )?;
                    return Err(if exists {
                        StorageError::Conflict {
                            entity: "render cache revision",
                        }
                    } else {
                        StorageError::NotFound {
                            entity: "render cache",
                        }
                    });
                }
            }
        }
        let cached = CachedRender {
            message_id: input.message_id,
            renderer_version: input.renderer_version,
            html: input.html,
            last_used_at_ms: input.at_ms,
        };
        transaction.commit()?;
        Ok(cached)
    }

    pub fn get_render_cache(
        &self,
        message_id: &MessageId,
        renderer_version: RendererVersion,
        touched_at_ms: TimestampMillis,
    ) -> Result<Option<CachedRender>> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let message_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?1)",
            params![message_id.as_str()],
            |row| row.get(0),
        )?;
        if !message_exists {
            return Err(StorageError::NotFound { entity: "message" });
        }
        let raw = transaction
            .query_row(
                "SELECT renderer_ver, html, last_used_at_ms
                 FROM message_render_cache
                 WHERE message_id = ?1 AND renderer_ver = ?2",
                params![
                    message_id.as_str(),
                    encode_u64(renderer_version.get(), "renderer version")?
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((stored_version, html, last_used_at_ms)) = raw else {
            transaction.commit()?;
            return Ok(None);
        };
        let stored_last_used = TimestampMillis::new(last_used_at_ms)?;
        if touched_at_ms < stored_last_used {
            return Err(StorageError::InvalidInput {
                field: "render cache timestamp",
                reason: "must not move backwards",
            });
        }
        let changed = transaction.execute(
            "UPDATE message_render_cache
             SET last_used_at_ms = ?3
             WHERE message_id = ?1 AND renderer_ver = ?2 AND last_used_at_ms = ?4",
            params![
                message_id.as_str(),
                stored_version,
                touched_at_ms.get(),
                last_used_at_ms
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::Conflict {
                entity: "render cache revision",
            });
        }
        let cached = CachedRender {
            message_id: message_id.clone(),
            renderer_version: RendererVersion::new(decode_u64(
                stored_version,
                "renderer version",
            )?)?,
            html,
            last_used_at_ms: touched_at_ms,
        };
        transaction.commit()?;
        Ok(Some(cached))
    }

    pub fn evict_render_cache(&self, input: EvictRenderCache) -> Result<RenderCacheEviction> {
        if input.limit == 0 || input.limit > MAX_RENDER_CACHE_EVICTION {
            return Err(StorageError::InvalidInput {
                field: "render cache eviction limit",
                reason: "must be between 1 and 1000",
            });
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (entries, bytes): (i64, i64) = transaction.query_row(
            "SELECT count(*), coalesce(sum(length(CAST(html AS BLOB))), 0)
             FROM (
                SELECT html
                FROM message_render_cache
                WHERE last_used_at_ms < ?1
                ORDER BY last_used_at_ms ASC, message_id ASC
                LIMIT ?2
             )",
            params![input.older_than_ms.get(), i64::from(input.limit)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        transaction.execute(
            "DELETE FROM message_render_cache
             WHERE message_id IN (
                SELECT message_id
                FROM message_render_cache
                WHERE last_used_at_ms < ?1
                ORDER BY last_used_at_ms ASC, message_id ASC
                LIMIT ?2
             )",
            params![input.older_than_ms.get(), i64::from(input.limit)],
        )?;
        let report = RenderCacheEviction {
            evicted_entries: decode_u64(entries, "render cache eviction count")?,
            evicted_html_bytes: decode_u64(bytes, "render cache eviction bytes")?,
        };
        transaction.commit()?;
        Ok(report)
    }

    pub fn maintain_wal(&self, policy: WalCheckpointPolicy) -> Result<WalMaintenanceReport> {
        validate_wal_threshold(policy.restart_threshold_bytes, "WAL restart threshold")?;
        validate_wal_threshold(
            policy.emergency_truncate_threshold_bytes,
            "WAL emergency truncate threshold",
        )?;
        if let Some(emergency_threshold) = policy.emergency_truncate_threshold_bytes {
            let Some(restart_threshold) = policy.restart_threshold_bytes else {
                return Err(StorageError::InvalidInput {
                    field: "WAL emergency truncate threshold",
                    reason: "requires a restart threshold",
                });
            };
            if emergency_threshold < restart_threshold {
                return Err(StorageError::InvalidInput {
                    field: "WAL emergency truncate threshold",
                    reason: "must not be below the restart threshold",
                });
            }
        }
        let connection = self.connection()?;
        let passive = wal_checkpoint_telemetry(&connection, "PASSIVE", &self.database_path)?;
        let threshold_exceeded = policy
            .restart_threshold_bytes
            .is_some_and(|threshold| passive.frame_payload_bytes >= threshold);
        let restart = if threshold_exceeded {
            Some(wal_checkpoint_telemetry(
                &connection,
                "RESTART",
                &self.database_path,
            )?)
        } else {
            None
        };
        let emergency_truncate_threshold_exceeded = restart
            .zip(policy.emergency_truncate_threshold_bytes)
            .is_some_and(|(telemetry, threshold)| telemetry.wal_file_bytes >= threshold);
        // TRUNCATE is deliberately a second-stage emergency action. Never
        // request it directly from PASSIVE telemetry: RESTART must first prove
        // that no reader is blocking and that every frame was checkpointed.
        let truncate = if emergency_truncate_threshold_exceeded
            && restart.is_some_and(|telemetry| !telemetry.busy && telemetry.remaining_frames == 0)
        {
            Some(wal_checkpoint_telemetry(
                &connection,
                "TRUNCATE",
                &self.database_path,
            )?)
        } else {
            None
        };
        let starvation_observed = passive.busy
            || passive.remaining_frames != 0
            || restart.is_some_and(|telemetry| telemetry.busy || telemetry.remaining_frames != 0)
            || truncate.is_some_and(|telemetry| telemetry.busy || telemetry.remaining_frames != 0);
        Ok(WalMaintenanceReport {
            passive,
            restart,
            truncate,
            restart_threshold_bytes: policy.restart_threshold_bytes,
            emergency_truncate_threshold_bytes: policy.emergency_truncate_threshold_bytes,
            threshold_exceeded,
            emergency_truncate_threshold_exceeded,
            starvation_observed,
        })
    }

    pub fn load_preferences(&self) -> Result<AppPreferences> {
        let connection = self.connection()?;
        let raw = connection.query_row(
            "SELECT
                selected_provider_id,
                openai_model_id, anthropic_model_id, deepseek_model_id,
                ollama_cloud_model_id, gemini_model_id,
                theme, default_mode, revision, updated_at_ms
             FROM settings WHERE singleton = 1",
            [],
            raw_preferences,
        )?;
        decode_preferences(raw)
    }

    pub fn save_preferences(&self, update: UpdatePreferences) -> Result<AppPreferences> {
        validate_model_preferences(&update.model_ids)?;
        ensure_revision_can_advance(update.expected_revision, "settings revision")?;
        let expected_revision = encode_u64(update.expected_revision, "settings revision")?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE settings
             SET selected_provider_id = ?1,
                 openai_model_id = ?2,
                 anthropic_model_id = ?3,
                 deepseek_model_id = ?4,
                 ollama_cloud_model_id = ?5,
                 gemini_model_id = ?6,
                 theme = ?7,
                 default_mode = ?8,
                 revision = revision + 1,
                 updated_at_ms = max(updated_at_ms, ?9)
             WHERE singleton = 1 AND revision = ?10",
            params![
                update.selected_provider_id.as_str(),
                update.model_ids.openai,
                update.model_ids.anthropic,
                update.model_ids.deepseek,
                update.model_ids.ollama_cloud,
                update.model_ids.gemini,
                update.theme.as_str(),
                update.default_mode.as_str(),
                update.at_ms.get(),
                expected_revision
            ],
        )?;
        if changed == 0 {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM settings WHERE singleton = 1)",
                [],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(StorageError::IncompatibleSchema {
                    reason: "settings singleton is missing",
                });
            }
            return Err(StorageError::Conflict {
                entity: "settings revision",
            });
        }
        let raw = transaction.query_row(
            "SELECT
                selected_provider_id,
                openai_model_id, anthropic_model_id, deepseek_model_id,
                ollama_cloud_model_id, gemini_model_id,
                theme, default_mode, revision, updated_at_ms
             FROM settings WHERE singleton = 1",
            [],
            raw_preferences,
        )?;
        let preferences = decode_preferences(raw)?;
        transaction.commit()?;
        Ok(preferences)
    }

    fn connection(&self) -> Result<Connection> {
        migration::open_operational_connection(&self.database_path)
    }
}

#[derive(Debug)]
struct DatabaseLease {
    _file: File,
}

impl DatabaseLease {
    fn acquire(path: &Path) -> Result<(Self, PathBuf)> {
        let file_name = path.file_name().ok_or(StorageError::InvalidInput {
            field: "database path",
            reason: "must name a database file",
        })?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(StorageError::PathUnavailable)?;
        let parent = fs::canonicalize(parent).map_err(StorageError::PathUnavailable)?;
        let database_path = parent.join(file_name);
        let mut lock_name = file_name.to_os_string();
        lock_name.push(".lock");
        let lock_path = parent.join(lock_name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(StorageError::PathUnavailable)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok((Self { _file: file }, database_path)),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Err(StorageError::Conflict {
                entity: "database lease",
            }),
            Err(error) => Err(StorageError::PathUnavailable(error)),
        }
    }
}

#[derive(Debug)]
struct RawPreferences {
    selected_provider_id: String,
    openai_model_id: String,
    anthropic_model_id: String,
    deepseek_model_id: String,
    ollama_cloud_model_id: String,
    gemini_model_id: String,
    theme: String,
    default_mode: String,
    revision: i64,
    updated_at_ms: i64,
}

fn raw_preferences(row: &Row<'_>) -> rusqlite::Result<RawPreferences> {
    Ok(RawPreferences {
        selected_provider_id: row.get(0)?,
        openai_model_id: row.get(1)?,
        anthropic_model_id: row.get(2)?,
        deepseek_model_id: row.get(3)?,
        ollama_cloud_model_id: row.get(4)?,
        gemini_model_id: row.get(5)?,
        theme: row.get(6)?,
        default_mode: row.get(7)?,
        revision: row.get(8)?,
        updated_at_ms: row.get(9)?,
    })
}

fn decode_preferences(raw: RawPreferences) -> Result<AppPreferences> {
    let model_ids = ProviderModelIds {
        openai: raw.openai_model_id,
        anthropic: raw.anthropic_model_id,
        deepseek: raw.deepseek_model_id,
        ollama_cloud: raw.ollama_cloud_model_id,
        gemini: raw.gemini_model_id,
    };
    validate_model_preferences(&model_ids)?;
    Ok(AppPreferences {
        selected_provider_id: ProviderId::from_str(&raw.selected_provider_id)?,
        model_ids,
        theme: Theme::from_str(&raw.theme)?,
        default_mode: DefaultMode::from_str(&raw.default_mode)?,
        revision: decode_u64(raw.revision, "settings revision")?,
        updated_at_ms: TimestampMillis::new(raw.updated_at_ms)?,
    })
}

#[derive(Debug)]
struct RawChat {
    id: String,
    character_id: String,
    title: String,
    revision: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

fn raw_chat(row: &Row<'_>) -> rusqlite::Result<RawChat> {
    Ok(RawChat {
        id: row.get(0)?,
        character_id: row.get(1)?,
        title: row.get(2)?,
        revision: row.get(3)?,
        created_at_ms: row.get(4)?,
        updated_at_ms: row.get(5)?,
    })
}

fn decode_chat(raw: RawChat) -> Result<Chat> {
    validate_persisted_text(&raw.title, MAX_CHAT_TITLE_BYTES, "chat title")?;
    Ok(Chat {
        id: ChatId::parse(raw.id)?,
        character_id: CharacterId::parse(raw.character_id)?,
        title: raw.title,
        revision: decode_u64(raw.revision, "chat revision")?,
        created_at_ms: TimestampMillis::new(raw.created_at_ms)?,
        updated_at_ms: TimestampMillis::new(raw.updated_at_ms)?,
    })
}

#[derive(Debug)]
struct RawMessage {
    id: String,
    chat_id: String,
    parent_id: Option<String>,
    sibling_ord: i64,
    depth: i64,
    ordinal: i64,
    role: String,
    status: String,
    text: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    completed_at_ms: Option<i64>,
}

const MESSAGE_PAGE_ROW_ENVELOPE_BYTES: usize = 1_024;

fn collect_bounded_message_rows<I>(rows: I, limit: u16) -> Result<(Vec<RawMessage>, bool)>
where
    I: Iterator<Item = rusqlite::Result<RawMessage>>,
{
    let mut messages = Vec::new();
    let mut aggregate_bytes = 0usize;
    for row in rows {
        let message = row?;
        if messages.len() >= usize::from(limit) {
            return Ok((messages, true));
        }
        let row_bytes = message
            .text
            .len()
            .checked_add(MESSAGE_PAGE_ROW_ENVELOPE_BYTES)
            .ok_or(StorageError::IncompatibleSchema {
                reason: "message page byte count overflowed",
            })?;
        let projected =
            aggregate_bytes
                .checked_add(row_bytes)
                .ok_or(StorageError::IncompatibleSchema {
                    reason: "message page byte count overflowed",
                })?;
        if !messages.is_empty() && projected > MAX_MESSAGE_PAGE_BYTES {
            return Ok((messages, true));
        }
        if projected > MAX_MESSAGE_PAGE_BYTES {
            return Err(StorageError::IncompatibleSchema {
                reason: "one message exceeds the native page byte budget",
            });
        }
        aggregate_bytes = projected;
        messages.push(message);
    }
    Ok((messages, false))
}

fn finish_recent_message_page(
    chat_id: &ChatId,
    raw_messages: Vec<RawMessage>,
    has_more: bool,
) -> Result<RecentMessagePage> {
    let mut messages = raw_messages
        .into_iter()
        .map(decode_message)
        .collect::<Result<Vec<_>>>()?;
    messages.reverse();
    let older_cursor = if has_more {
        messages
            .first()
            .map(|message| MessageOrdinalCursor::new(chat_id.clone(), message.ordinal))
            .transpose()?
    } else {
        None
    };
    Ok(RecentMessagePage {
        messages,
        older_cursor,
    })
}

fn raw_message(row: &Row<'_>) -> rusqlite::Result<RawMessage> {
    raw_message_from(row, 0)
}

fn raw_message_from(row: &Row<'_>, start: usize) -> rusqlite::Result<RawMessage> {
    Ok(RawMessage {
        id: row.get(start)?,
        chat_id: row.get(start + 1)?,
        parent_id: row.get(start + 2)?,
        sibling_ord: row.get(start + 3)?,
        depth: row.get(start + 4)?,
        ordinal: row.get(start + 5)?,
        role: row.get(start + 6)?,
        status: row.get(start + 7)?,
        text: row.get(start + 8)?,
        created_at_ms: row.get(start + 9)?,
        updated_at_ms: row.get(start + 10)?,
        completed_at_ms: row.get(start + 11)?,
    })
}

#[derive(Debug)]
struct RawActivePathEntry {
    position: i64,
    message: RawMessage,
}

fn raw_active_path_entry(row: &Row<'_>) -> rusqlite::Result<RawActivePathEntry> {
    Ok(RawActivePathEntry {
        position: row.get(0)?,
        message: raw_message_from(row, 1)?,
    })
}

fn decode_active_path_entry(raw: RawActivePathEntry) -> Result<ActivePathEntry> {
    Ok(ActivePathEntry {
        position: decode_u64(raw.position, "active path position")?,
        message: decode_message(raw.message)?,
    })
}

fn decode_message(raw: RawMessage) -> Result<Message> {
    validate_persisted_text(&raw.text, MAX_MESSAGE_BYTES, "message text")?;
    Ok(Message {
        id: MessageId::parse(raw.id)?,
        chat_id: ChatId::parse(raw.chat_id)?,
        parent_id: raw.parent_id.map(MessageId::parse).transpose()?,
        sibling_ord: decode_u64(raw.sibling_ord, "message sibling ordinal")?,
        depth: decode_u64(raw.depth, "message depth")?,
        ordinal: decode_u64(raw.ordinal, "message ordinal")?,
        role: MessageRole::from_str(&raw.role)?,
        status: MessageStatus::from_str(&raw.status)?,
        text: raw.text,
        created_at_ms: TimestampMillis::new(raw.created_at_ms)?,
        updated_at_ms: TimestampMillis::new(raw.updated_at_ms)?,
        completed_at_ms: raw.completed_at_ms.map(TimestampMillis::new).transpose()?,
    })
}

fn query_message(connection: &Connection, message_id: &MessageId) -> Result<RawMessage> {
    connection
        .query_row(
            "SELECT id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
             FROM messages WHERE id = ?1",
            params![message_id.as_str()],
            raw_message,
        )
        .optional()?
        .ok_or(StorageError::NotFound { entity: "message" })
}

#[derive(Debug)]
struct RawRequestState {
    id: String,
    chat_id: String,
    user_message_id: String,
    assistant_message_id: String,
    provider_id: String,
    model_id: String,
    owner_label: String,
    stream_generation: String,
    status: String,
    last_delivered_seq: i64,
    last_durable_seq: i64,
    last_acked_seq: Option<i64>,
    provider_response_id: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    failure_code: Option<String>,
    started_at_ms: i64,
    updated_at_ms: i64,
    finished_at_ms: Option<i64>,
}

fn raw_request_state(row: &Row<'_>) -> rusqlite::Result<RawRequestState> {
    Ok(RawRequestState {
        id: row.get(0)?,
        chat_id: row.get(1)?,
        user_message_id: row.get(2)?,
        assistant_message_id: row.get(3)?,
        provider_id: row.get(4)?,
        model_id: row.get(5)?,
        owner_label: row.get(6)?,
        stream_generation: row.get(7)?,
        status: row.get(8)?,
        last_delivered_seq: row.get(9)?,
        last_durable_seq: row.get(10)?,
        last_acked_seq: row.get(11)?,
        provider_response_id: row.get(12)?,
        input_tokens: row.get(13)?,
        output_tokens: row.get(14)?,
        cached_input_tokens: row.get(15)?,
        reasoning_tokens: row.get(16)?,
        failure_code: row.get(17)?,
        started_at_ms: row.get(18)?,
        updated_at_ms: row.get(19)?,
        finished_at_ms: row.get(20)?,
    })
}

fn decode_request_state(raw: RawRequestState) -> Result<RequestState> {
    let usage = decode_usage(
        raw.input_tokens,
        raw.output_tokens,
        raw.cached_input_tokens,
        raw.reasoning_tokens,
    )?;
    let last_delivered_seq = decode_u64(raw.last_delivered_seq, "delivered sequence")?;
    let last_durable_seq = decode_u64(raw.last_durable_seq, "durable sequence")?;
    let last_acked_seq = raw
        .last_acked_seq
        .map(|sequence| decode_u64(sequence, "acknowledged sequence"))
        .transpose()?;
    validate_stream_sequence_invariant(last_delivered_seq, last_durable_seq, last_acked_seq)?;
    Ok(RequestState {
        id: RequestStateId::parse(raw.id)?,
        chat_id: ChatId::parse(raw.chat_id)?,
        user_message_id: MessageId::parse(raw.user_message_id)?,
        assistant_message_id: MessageId::parse(raw.assistant_message_id)?,
        selection: ProviderSelection {
            provider_id: ProviderId::from_str(&raw.provider_id)?,
            model_id: ModelId::parse(raw.model_id)?,
        },
        owner_label: StreamOwnerLabel::parse(raw.owner_label)?,
        stream_generation: StreamGeneration::parse(raw.stream_generation)?,
        status: RequestStatus::from_str(&raw.status)?,
        last_delivered_seq,
        last_durable_seq,
        last_acked_seq,
        provider_response_id: raw.provider_response_id,
        usage,
        failure_code: raw
            .failure_code
            .map(|code| RequestFailureCode::from_str(&code))
            .transpose()?,
        started_at_ms: TimestampMillis::new(raw.started_at_ms)?,
        updated_at_ms: TimestampMillis::new(raw.updated_at_ms)?,
        finished_at_ms: raw.finished_at_ms.map(TimestampMillis::new).transpose()?,
    })
}

fn load_request_state(
    connection: &Connection,
    request_state_id: &RequestStateId,
) -> Result<RequestState> {
    let raw = connection
        .query_row(
            "SELECT
                id, chat_id, user_message_id, assistant_message_id,
                provider_id, model_id, owner_label, stream_generation,
                status, last_delivered_seq, last_durable_seq, last_acked_seq,
                provider_response_id,
                input_tokens, output_tokens, cached_input_tokens, reasoning_tokens,
                failure_code, started_at_ms, updated_at_ms, finished_at_ms
             FROM request_state WHERE id = ?1",
            params![request_state_id.as_str()],
            raw_request_state,
        )
        .optional()?
        .ok_or(StorageError::NotFound {
            entity: "request state",
        })?;
    decode_request_state(raw)
}

fn apply_response_progress(
    transaction: &Transaction<'_>,
    checkpoint: &ResponseCheckpoint,
    terminal: Option<TerminalOutcome>,
) -> Result<ResponseProgress> {
    validate_checkpoint(checkpoint)?;
    let state = load_request_state(transaction, &checkpoint.request_state_id)?;
    ensure_running_and_owned(
        &state,
        &checkpoint.owner_label,
        &checkpoint.stream_generation,
    )?;
    if state.last_durable_seq != checkpoint.expected_last_durable_seq {
        return Err(StorageError::SequenceMismatch {
            expected: state.last_durable_seq,
            actual: checkpoint.expected_last_durable_seq,
        });
    }
    if checkpoint.through_seq > state.last_delivered_seq {
        return Err(StorageError::InvalidInput {
            field: "durable sequence",
            reason: "must not advance beyond the delivered sequence",
        });
    }

    validate_response_metadata(&state, checkpoint)?;
    let effective_at_ms = effective_stream_timestamp(&state, checkpoint.at_ms);
    let existing_bytes: i64 = transaction.query_row(
        "SELECT length(CAST(text AS BLOB)) FROM messages WHERE id = ?1",
        params![state.assistant_message_id.as_str()],
        |row| row.get(0),
    )?;
    let existing_bytes =
        usize::try_from(existing_bytes).map_err(|_| StorageError::IncompatibleSchema {
            reason: "assistant message size is outside the supported range",
        })?;
    let text_bytes = existing_bytes
        .checked_add(checkpoint.appended_text.len())
        .ok_or(StorageError::InvalidInput {
            field: "assistant message",
            reason: "exceeds the byte limit",
        })?;
    if text_bytes > MAX_MESSAGE_BYTES {
        return Err(StorageError::InvalidInput {
            field: "assistant message",
            reason: "exceeds the byte limit",
        });
    }

    let provider_response_id = checkpoint
        .provider_response_id
        .as_deref()
        .or(state.provider_response_id.as_deref());
    let usage = checkpoint.usage.or(state.usage);
    let usage_values = usage.map(encode_usage).transpose()?;
    let (input_tokens, output_tokens, cached_input_tokens, reasoning_tokens) =
        usage_values.unwrap_or((None, None, None, None));
    let through_seq = encode_u64(checkpoint.through_seq, "request sequence")?;
    let at_ms = effective_at_ms.get();

    let (request_status, message_status, failure_code, finished_at_ms) = match terminal {
        None => (RequestStatus::Running, MessageStatus::Partial, None, None),
        Some(TerminalOutcome::Completed) => (
            RequestStatus::Completed,
            MessageStatus::Complete,
            None,
            Some(at_ms),
        ),
        Some(TerminalOutcome::Cancelled) => (
            RequestStatus::Cancelled,
            MessageStatus::Partial,
            None,
            Some(at_ms),
        ),
        Some(TerminalOutcome::Failed(code)) => (
            RequestStatus::Failed,
            MessageStatus::Failed,
            Some(code.as_str()),
            Some(at_ms),
        ),
    };

    let changed_message = transaction.execute(
        "UPDATE messages
         SET text = text || ?2,
             status = ?3,
             updated_at_ms = max(updated_at_ms, ?4),
             completed_at_ms = CASE WHEN ?3 = 'complete' THEN ?4 ELSE NULL END
         WHERE id = ?1 AND status = 'partial'",
        params![
            state.assistant_message_id.as_str(),
            checkpoint.appended_text,
            message_status.as_str(),
            at_ms
        ],
    )?;
    if changed_message != 1 {
        return Err(StorageError::IncompatibleSchema {
            reason: "running request has no partial assistant message",
        });
    }
    let changed_request = transaction.execute(
        "UPDATE request_state
         SET status = ?2,
             last_durable_seq = ?3,
             provider_response_id = ?4,
             input_tokens = ?5,
             output_tokens = ?6,
             cached_input_tokens = ?7,
             reasoning_tokens = ?8,
             failure_code = ?9,
             updated_at_ms = max(updated_at_ms, ?10),
             finished_at_ms = ?11
         WHERE id = ?1
           AND owner_label = ?12
           AND stream_generation = ?13
           AND status = 'running'
           AND last_durable_seq = ?14",
        params![
            checkpoint.request_state_id.as_str(),
            request_status.as_str(),
            through_seq,
            provider_response_id,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            reasoning_tokens,
            failure_code,
            at_ms,
            finished_at_ms,
            checkpoint.owner_label.as_str(),
            checkpoint.stream_generation.as_str(),
            encode_u64(checkpoint.expected_last_durable_seq, "durable sequence")?
        ],
    )?;
    if changed_request != 1 {
        return Err(StorageError::SequenceMismatch {
            expected: state.last_durable_seq,
            actual: checkpoint.expected_last_durable_seq,
        });
    }
    transaction.execute(
        "UPDATE chats SET updated_at_ms = max(updated_at_ms, ?2) WHERE id = ?1",
        params![state.chat_id.as_str(), at_ms],
    )?;

    Ok(ResponseProgress {
        request_state_id: checkpoint.request_state_id.clone(),
        assistant_message_id: state.assistant_message_id,
        last_delivered_seq: state.last_delivered_seq,
        last_durable_seq: checkpoint.through_seq,
        last_acked_seq: state.last_acked_seq,
        text_bytes,
        status: request_status,
        updated_at_ms: effective_at_ms,
    })
}

fn validate_response_metadata(state: &RequestState, checkpoint: &ResponseCheckpoint) -> Result<()> {
    if let (Some(existing), Some(candidate)) = (
        state.provider_response_id.as_deref(),
        checkpoint.provider_response_id.as_deref(),
    ) && existing != candidate
    {
        return Err(StorageError::InvalidInput {
            field: "provider response ID",
            reason: "cannot change after it is set",
        });
    }
    if let (Some(existing), Some(candidate)) = (state.usage, checkpoint.usage)
        && (candidate.input_tokens < existing.input_tokens
            || candidate.output_tokens < existing.output_tokens
            || candidate.cached_input_tokens < existing.cached_input_tokens
            || candidate.reasoning_tokens < existing.reasoning_tokens)
    {
        return Err(StorageError::InvalidInput {
            field: "token usage",
            reason: "must be monotonic",
        });
    }
    Ok(())
}

fn validate_chat_title(title: &str) -> Result<()> {
    if title.len() > MAX_CHAT_TITLE_BYTES {
        return Err(StorageError::InvalidInput {
            field: "chat title",
            reason: "exceeds the byte limit",
        });
    }
    if title.contains('\0') {
        return Err(StorageError::InvalidInput {
            field: "chat title",
            reason: "contains a null character",
        });
    }
    Ok(())
}

fn validate_complete_message(role: MessageRole, text: &str) -> Result<()> {
    if role == MessageRole::User {
        return validate_user_message(text);
    }
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(StorageError::InvalidInput {
            field: "assistant message",
            reason: "exceeds the byte limit",
        });
    }
    if text.contains('\0') {
        return Err(StorageError::InvalidInput {
            field: "assistant message",
            reason: "contains a null character",
        });
    }
    Ok(())
}

fn validate_branch_depth(depth: u64) -> Result<()> {
    if depth > MAX_BRANCH_DEPTH {
        return Err(StorageError::InvalidInput {
            field: "message depth",
            reason: "exceeds the branch depth limit",
        });
    }
    Ok(())
}

fn current_active_leaf(
    connection: &Connection,
    chat_id: &ChatId,
) -> Result<Option<(MessageId, u64)>> {
    let raw = connection
        .query_row(
            "SELECT p.message_id, m.depth
             FROM active_path AS p
             JOIN messages AS m ON m.chat_id = p.chat_id AND m.id = p.message_id
             WHERE p.chat_id = ?1
             ORDER BY p.position DESC
             LIMIT 1",
            params![chat_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    raw.map(|(message_id, depth)| {
        Ok((
            MessageId::parse(message_id)?,
            decode_u64(depth, "message depth")?,
        ))
    })
    .transpose()
}

fn ensure_expected_active_leaf(
    connection: &Connection,
    chat_id: &ChatId,
    expected: Option<&MessageId>,
) -> Result<()> {
    let actual = current_active_leaf(connection, chat_id)?.map(|(message_id, _)| message_id);
    if actual.as_ref() != expected {
        return Err(StorageError::Conflict {
            entity: "active path",
        });
    }
    Ok(())
}

fn message_depth(connection: &Connection, chat_id: &ChatId, message_id: &MessageId) -> Result<u64> {
    let depth = connection
        .query_row(
            "SELECT depth FROM messages WHERE chat_id = ?1 AND id = ?2",
            params![chat_id.as_str(), message_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(StorageError::NotFound { entity: "message" })?;
    let depth = decode_u64(depth, "message depth")?;
    validate_branch_depth(depth)?;
    Ok(depth)
}

fn next_sibling_ordinal(
    connection: &Connection,
    chat_id: &ChatId,
    parent_id: Option<&MessageId>,
) -> Result<u64> {
    let maximum: Option<i64> = match parent_id {
        Some(parent_id) => connection.query_row(
            "SELECT max(sibling_ord)
             FROM messages WHERE chat_id = ?1 AND parent_id = ?2",
            params![chat_id.as_str(), parent_id.as_str()],
            |row| row.get(0),
        )?,
        None => connection.query_row(
            "SELECT max(sibling_ord)
             FROM messages WHERE chat_id = ?1 AND parent_id IS NULL",
            params![chat_id.as_str()],
            |row| row.get(0),
        )?,
    };
    let next = maximum
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(StorageError::InvalidInput {
            field: "message sibling ordinal",
            reason: "cannot advance beyond the supported range",
        })?;
    decode_u64(next, "message sibling ordinal")
}

fn next_message_ordinal(connection: &Connection, chat_id: &ChatId) -> Result<u64> {
    let maximum: Option<i64> = connection.query_row(
        "SELECT max(ordinal) FROM messages WHERE chat_id = ?1",
        params![chat_id.as_str()],
        |row| row.get(0),
    )?;
    let next = maximum
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(StorageError::InvalidInput {
            field: "message ordinal",
            reason: "cannot advance beyond the supported range",
        })?;
    decode_u64(next, "message ordinal")
}

fn replace_active_path(
    transaction: &Transaction<'_>,
    chat_id: &ChatId,
    leaf_message_id: &MessageId,
) -> Result<u64> {
    let leaf_depth = message_depth(transaction, chat_id, leaf_message_id)?;
    let path_length = leaf_depth
        .checked_add(1)
        .ok_or(StorageError::IncompatibleSchema {
            reason: "active path length exceeds the supported range",
        })?;
    let encoded_leaf_depth = encode_u64(leaf_depth, "message depth")?;
    let encoded_path_length = encode_u64(path_length, "active path length")?;

    // Parent/depth constraints make ancestry strictly descend to depth zero.
    // Materialize it inside SQLite once per statement instead of issuing one
    // SELECT per ancestor or retaining a million-ID Vec in application memory.
    // Only the divergent suffix is rewritten, keeping a sibling switch's WAL
    // proportional to the changed suffix rather than the full conversation.
    let (count, minimum, maximum, first_mismatch): (i64, Option<i64>, Option<i64>, Option<i64>) =
        transaction.query_row(
            "WITH RECURSIVE ancestry(message_id, parent_id, position) AS (
            SELECT id, parent_id, depth
            FROM messages
            WHERE chat_id = ?1 AND id = ?2
            UNION ALL
            SELECT parent.id, parent.parent_id, parent.depth
            FROM messages AS parent
            JOIN ancestry AS child ON parent.id = child.parent_id
            WHERE parent.chat_id = ?1
         )
         SELECT count(*), min(ancestry.position), max(ancestry.position),
                min(CASE
                    WHEN current.message_id IS NULL
                      OR current.message_id != ancestry.message_id
                    THEN ancestry.position
                END)
         FROM ancestry
         LEFT JOIN active_path AS current
           ON current.chat_id = ?1 AND current.position = ancestry.position",
            params![chat_id.as_str(), leaf_message_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    if count != encoded_path_length || minimum != Some(0) || maximum != Some(encoded_leaf_depth) {
        return Err(StorageError::IncompatibleSchema {
            reason: "message ancestry is incomplete or has invalid depth",
        });
    }
    let divergence = first_mismatch.unwrap_or(encoded_path_length);
    if divergence < 0 || divergence > encoded_path_length {
        return Err(StorageError::IncompatibleSchema {
            reason: "active path divergence is outside the ancestry",
        });
    }

    transaction.execute(
        "DELETE FROM active_path WHERE chat_id = ?1 AND position >= ?2",
        params![chat_id.as_str(), divergence],
    )?;
    let inserted = transaction.execute(
        "WITH RECURSIVE ancestry(message_id, parent_id, position) AS (
            SELECT id, parent_id, depth
            FROM messages
            WHERE chat_id = ?1 AND id = ?2
            UNION ALL
            SELECT parent.id, parent.parent_id, parent.depth
            FROM messages AS parent
            JOIN ancestry AS child ON parent.id = child.parent_id
            WHERE parent.chat_id = ?1
         )
         INSERT INTO active_path(chat_id, position, message_id)
         SELECT ?1, position, message_id
         FROM ancestry
         WHERE position >= ?3
         ORDER BY position ASC",
        params![chat_id.as_str(), leaf_message_id.as_str(), divergence],
    )?;
    let expected_inserted =
        encoded_path_length
            .checked_sub(divergence)
            .ok_or(StorageError::IncompatibleSchema {
                reason: "active path suffix length is invalid",
            })?;
    if i64::try_from(inserted).ok() != Some(expected_inserted) {
        return Err(StorageError::IncompatibleSchema {
            reason: "active path suffix was not materialized completely",
        });
    }
    Ok(path_length)
}

fn validate_rendered_html(html: &str) -> Result<()> {
    if html.len() > MAX_RENDERED_HTML_BYTES {
        return Err(StorageError::InvalidInput {
            field: "rendered HTML",
            reason: "exceeds the byte limit",
        });
    }
    if html.contains('\0') {
        return Err(StorageError::InvalidInput {
            field: "rendered HTML",
            reason: "contains a null character",
        });
    }
    Ok(())
}

fn validate_model_preferences(model_ids: &ProviderModelIds) -> Result<()> {
    for model_id in [
        &model_ids.openai,
        &model_ids.anthropic,
        &model_ids.deepseek,
        &model_ids.ollama_cloud,
        &model_ids.gemini,
    ] {
        if model_id.len() > crate::MAX_MODEL_ID_BYTES {
            return Err(StorageError::InvalidInput {
                field: "model ID preference",
                reason: "exceeds the byte limit",
            });
        }
        if model_id.chars().any(char::is_control) {
            return Err(StorageError::InvalidInput {
                field: "model ID preference",
                reason: "contains a control character",
            });
        }
    }
    Ok(())
}

fn validate_user_message(content: &str) -> Result<()> {
    if content.trim().is_empty() {
        return Err(StorageError::InvalidInput {
            field: "user message",
            reason: "must not be blank",
        });
    }
    if content.len() > MAX_USER_MESSAGE_BYTES {
        return Err(StorageError::InvalidInput {
            field: "user message",
            reason: "exceeds the byte limit",
        });
    }
    if content.contains('\0') {
        return Err(StorageError::InvalidInput {
            field: "user message",
            reason: "contains a null character",
        });
    }
    Ok(())
}

fn validate_checkpoint(checkpoint: &ResponseCheckpoint) -> Result<()> {
    if checkpoint.through_seq <= checkpoint.expected_last_durable_seq {
        return Err(StorageError::InvalidInput {
            field: "checkpoint sequence",
            reason: "must advance beyond the expected sequence",
        });
    }
    encode_u64(checkpoint.expected_last_durable_seq, "durable sequence")?;
    encode_u64(checkpoint.through_seq, "durable sequence")?;
    if checkpoint.appended_text.len() > MAX_CHECKPOINT_BYTES {
        return Err(StorageError::InvalidInput {
            field: "checkpoint text",
            reason: "exceeds the byte limit",
        });
    }
    if checkpoint.appended_text.contains('\0') {
        return Err(StorageError::InvalidInput {
            field: "checkpoint text",
            reason: "contains a null character",
        });
    }
    if let Some(response_id) = checkpoint.provider_response_id.as_deref() {
        if response_id.is_empty() || response_id.len() > MAX_PROVIDER_RESPONSE_ID_BYTES {
            return Err(StorageError::InvalidInput {
                field: "provider response ID",
                reason: "has an invalid byte length",
            });
        }
        if response_id.chars().any(char::is_control) {
            return Err(StorageError::InvalidInput {
                field: "provider response ID",
                reason: "contains a control character",
            });
        }
    }
    if let Some(usage) = checkpoint.usage {
        encode_usage(usage)?;
    }
    Ok(())
}

fn validate_delivery_checkpoint(checkpoint: &DeliveryCheckpoint) -> Result<()> {
    encode_u64(checkpoint.expected_last_delivered_seq, "delivered sequence")?;
    encode_u64(checkpoint.through_seq, "delivered sequence")?;
    let next = checkpoint
        .expected_last_delivered_seq
        .checked_add(1)
        .ok_or(StorageError::InvalidInput {
            field: "delivered sequence",
            reason: "cannot advance beyond the supported range",
        })?;
    if checkpoint.through_seq != next {
        return Err(StorageError::InvalidInput {
            field: "delivered sequence",
            reason: "must advance by exactly one contiguous sequence",
        });
    }
    Ok(())
}

fn validate_cumulative_ack(acknowledgement: &CumulativeAck) -> Result<()> {
    let previous = acknowledgement.expected_last_acked_seq.unwrap_or(0);
    if let Some(sequence) = acknowledgement.expected_last_acked_seq {
        if sequence == 0 {
            return Err(StorageError::InvalidInput {
                field: "acknowledged sequence",
                reason: "zero is represented by no acknowledgement",
            });
        }
        encode_u64(sequence, "acknowledged sequence")?;
    }
    encode_u64(acknowledgement.through_seq, "acknowledged sequence")?;
    if acknowledgement.through_seq <= previous {
        return Err(StorageError::InvalidInput {
            field: "acknowledged sequence",
            reason: "must cumulatively advance beyond the previous acknowledgement",
        });
    }
    Ok(())
}

fn ensure_running_and_owned(
    state: &RequestState,
    owner_label: &StreamOwnerLabel,
    stream_generation: &StreamGeneration,
) -> Result<()> {
    if state.status != RequestStatus::Running {
        return Err(StorageError::InvalidState {
            expected: RequestStatus::Running.as_str(),
            actual: state.status.as_str().to_owned(),
        });
    }
    ensure_stream_owned(state, owner_label, stream_generation)
}

fn ensure_stream_owned(
    state: &RequestState,
    owner_label: &StreamOwnerLabel,
    stream_generation: &StreamGeneration,
) -> Result<()> {
    if &state.owner_label != owner_label || &state.stream_generation != stream_generation {
        return Err(StorageError::Conflict {
            entity: "stream identity",
        });
    }
    Ok(())
}

fn effective_stream_timestamp(state: &RequestState, candidate: TimestampMillis) -> TimestampMillis {
    candidate.max(state.started_at_ms).max(state.updated_at_ms)
}

fn validate_stream_sequence_invariant(
    last_delivered_seq: u64,
    last_durable_seq: u64,
    last_acked_seq: Option<u64>,
) -> Result<()> {
    if last_durable_seq > last_delivered_seq
        || last_acked_seq.is_some_and(|sequence| sequence == 0 || sequence > last_durable_seq)
    {
        return Err(StorageError::IncompatibleSchema {
            reason: "stream journal sequence invariant failed",
        });
    }
    Ok(())
}

fn validate_page_size(limit: u16) -> Result<()> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(StorageError::InvalidInput {
            field: "page size",
            reason: "must be between 1 and 200",
        });
    }
    Ok(())
}

fn validate_search_query(query: &str) -> Result<()> {
    let count = query.chars().count();
    if count == 0 || count > MAX_SEARCH_QUERY_CHARS {
        return Err(StorageError::InvalidInput {
            field: "search query",
            reason: "must contain between 1 and 256 characters",
        });
    }
    if query.chars().any(|character| character == '\0') {
        return Err(StorageError::InvalidInput {
            field: "search query",
            reason: "contains a null character",
        });
    }
    Ok(())
}

fn validate_persisted_text(value: &str, max_bytes: usize, field: &'static str) -> Result<()> {
    if value.len() > max_bytes || value.contains('\0') {
        return Err(StorageError::IncompatibleSchema {
            reason: match field {
                "chat title" => "chat title violates the storage contract",
                "message text" => "message text violates the storage contract",
                _ => "stored text violates the storage contract",
            },
        });
    }
    Ok(())
}

fn ensure_chat_exists(connection: &Connection, chat_id: &ChatId) -> Result<()> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM chats WHERE id = ?1",
            params![chat_id.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(StorageError::NotFound { entity: "chat" });
    }
    Ok(())
}

fn encode_u64(value: u64, field: &'static str) -> Result<i64> {
    if value > crate::MAX_SAFE_INTEGER {
        return Err(StorageError::InvalidInput {
            field,
            reason: "exceeds the safe integer range",
        });
    }
    i64::try_from(value).map_err(|_| StorageError::InvalidInput {
        field,
        reason: "exceeds the database range",
    })
}

fn ensure_revision_can_advance(value: u64, field: &'static str) -> Result<()> {
    if value >= crate::MAX_SAFE_INTEGER {
        return Err(StorageError::InvalidInput {
            field,
            reason: "cannot advance beyond the safe integer range",
        });
    }
    Ok(())
}

fn decode_u64(value: i64, field: &'static str) -> Result<u64> {
    let value = u64::try_from(value).map_err(|_| StorageError::IncompatibleSchema {
        reason: match field {
            "message ordinal" => "message ordinal is negative",
            "request sequence" => "request sequence is negative",
            _ => "numeric value is outside the supported range",
        },
    })?;
    if value > crate::MAX_SAFE_INTEGER {
        return Err(StorageError::IncompatibleSchema {
            reason: "numeric value exceeds the safe integer range",
        });
    }
    Ok(value)
}

type EncodedUsage = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);

fn encode_usage(usage: TokenUsage) -> Result<EncodedUsage> {
    Ok((
        Some(encode_u64(usage.input_tokens, "input token usage")?),
        Some(encode_u64(usage.output_tokens, "output token usage")?),
        Some(encode_u64(
            usage.cached_input_tokens,
            "cached input token usage",
        )?),
        Some(encode_u64(usage.reasoning_tokens, "reasoning token usage")?),
    ))
}

fn decode_usage(
    input: Option<i64>,
    output: Option<i64>,
    cached: Option<i64>,
    reasoning: Option<i64>,
) -> Result<Option<TokenUsage>> {
    match (input, output, cached, reasoning) {
        (None, None, None, None) => Ok(None),
        (Some(input), Some(output), Some(cached), Some(reasoning)) => Ok(Some(TokenUsage {
            input_tokens: decode_u64(input, "input token usage")?,
            output_tokens: decode_u64(output, "output token usage")?,
            cached_input_tokens: decode_u64(cached, "cached input token usage")?,
            reasoning_tokens: decode_u64(reasoning, "reasoning token usage")?,
        })),
        _ => Err(StorageError::IncompatibleSchema {
            reason: "token usage columns are incomplete",
        }),
    }
}

fn wal_checkpoint_telemetry(
    connection: &Connection,
    mode: &'static str,
    database_path: &Path,
) -> Result<WalCheckpointTelemetry> {
    let sql = match mode {
        "PASSIVE" => "PRAGMA wal_checkpoint(PASSIVE)",
        "RESTART" => "PRAGMA wal_checkpoint(RESTART)",
        "TRUNCATE" => "PRAGMA wal_checkpoint(TRUNCATE)",
        _ => {
            return Err(StorageError::InvalidInput {
                field: "WAL checkpoint mode",
                reason: "is not supported",
            });
        }
    };
    let (busy, log_frames, checkpointed_frames): (i64, i64, i64) =
        connection.query_row(sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let log_frames = decode_u64(log_frames, "WAL log frame count")?;
    let checkpointed_frames = decode_u64(checkpointed_frames, "WAL checkpointed frame count")?;
    let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
    let page_size_bytes = decode_u64(page_size, "SQLite page size")?;
    let frame_payload_bytes =
        log_frames
            .checked_mul(page_size_bytes)
            .ok_or(StorageError::IncompatibleSchema {
                reason: "WAL frame byte count overflowed",
            })?;
    let mut wal_path = database_path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let wal_file_bytes = match fs::metadata(PathBuf::from(wal_path)) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(error) => return Err(StorageError::PathUnavailable(error)),
    };
    Ok(WalCheckpointTelemetry {
        busy: busy != 0,
        log_frames,
        checkpointed_frames,
        remaining_frames: log_frames.saturating_sub(checkpointed_frames),
        page_size_bytes,
        frame_payload_bytes,
        wal_file_bytes,
    })
}

fn validate_wal_threshold(threshold: Option<u64>, field: &'static str) -> Result<()> {
    if threshold.is_some_and(|value| value == 0 || value > crate::MAX_SAFE_INTEGER) {
        return Err(StorageError::InvalidInput {
            field,
            reason: "must be between 1 and the safe integer limit",
        });
    }
    Ok(())
}

fn map_constraint(error: rusqlite::Error, entity: &'static str) -> StorageError {
    match &error {
        rusqlite::Error::SqliteFailure(code, _) if code.code == ErrorCode::ConstraintViolation => {
            StorageError::Conflict { entity }
        }
        _ => StorageError::Database(error),
    }
}
