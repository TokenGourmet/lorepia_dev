use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use fs2::FileExt;
use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Row, Transaction, TransactionBehavior, params,
};

use crate::{
    AppPreferences, BeginTurn, CharacterId, Chat, ChatCursor, ChatId, ChatPage, CreateChat,
    DefaultMode, MAX_CHAT_TITLE_BYTES, MAX_CHECKPOINT_BYTES, MAX_MESSAGE_BYTES, MAX_PAGE_SIZE,
    MAX_PROVIDER_RESPONSE_ID_BYTES, MAX_SEARCH_QUERY_CHARS, MAX_SHORT_QUERY_SCAN_ROWS,
    MAX_USER_MESSAGE_BYTES, Message, MessageId, MessagePage, MessageRole, MessageSearchHit,
    MessageStatus, ModelId, ProviderId, ProviderModelIds, ProviderSelection, RequestFailureCode,
    RequestState, RequestStateId, RequestStatus, ResponseCheckpoint, ResponseProgress, Result,
    StartedTurn, StartupReport, StorageError, TerminalCheckpoint, TerminalOutcome, Theme,
    TimestampMillis, TokenUsage, UpdatePreferences, migration,
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

        let request_state_id = RequestStateId::new();
        let user_message_id = MessageId::new();
        let assistant_message_id = MessageId::new();
        let at_ms = input.started_at_ms.get();

        transaction
            .execute(
                "INSERT INTO messages(
                    id, chat_id, ordinal, role, status, text, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, 'user', 'complete', ?4, ?5, ?5)",
                params![
                    user_message_id.as_str(),
                    input.chat_id.as_str(),
                    user_ordinal_i64,
                    input.user_text,
                    at_ms
                ],
            )
            .map_err(|error| map_constraint(error, "message"))?;
        transaction
            .execute(
                "INSERT INTO messages(
                    id, chat_id, ordinal, role, status, text, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, 'assistant', 'partial', '', ?4, ?4)",
                params![
                    assistant_message_id.as_str(),
                    input.chat_id.as_str(),
                    assistant_ordinal_i64,
                    at_ms
                ],
            )
            .map_err(|error| map_constraint(error, "message"))?;
        transaction
            .execute(
                "INSERT INTO request_state(
                    id, chat_id, user_message_id, assistant_message_id,
                    provider_id, model_id, status, last_seq, started_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', 0, ?7, ?7)",
                params![
                    request_state_id.as_str(),
                    input.chat_id.as_str(),
                    user_message_id.as_str(),
                    assistant_message_id.as_str(),
                    input.selection.provider_id.as_str(),
                    input.selection.model_id.as_str(),
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
            last_seq: 0,
        })
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
        let raw = connection
            .query_row(
                "SELECT
                    id, chat_id, user_message_id, assistant_message_id,
                    provider_id, model_id, status, last_seq, provider_response_id,
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
            "SELECT id, chat_id, ordinal, role, status, text, created_at_ms, updated_at_ms
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
                    SELECT id, chat_id, ordinal, role, status, text, created_at_ms, updated_at_ms
                    FROM messages
                    WHERE chat_id = ?1
                    ORDER BY ordinal DESC
                    LIMIT ?4
                 )
                 SELECT id, chat_id, ordinal, role, status, text, created_at_ms, updated_at_ms
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
                "SELECT m.id, m.chat_id, m.ordinal, m.role, m.status, m.text,
                        m.created_at_ms, m.updated_at_ms
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
    ordinal: i64,
    role: String,
    status: String,
    text: String,
    created_at_ms: i64,
    updated_at_ms: i64,
}

fn raw_message(row: &Row<'_>) -> rusqlite::Result<RawMessage> {
    Ok(RawMessage {
        id: row.get(0)?,
        chat_id: row.get(1)?,
        ordinal: row.get(2)?,
        role: row.get(3)?,
        status: row.get(4)?,
        text: row.get(5)?,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

fn decode_message(raw: RawMessage) -> Result<Message> {
    validate_persisted_text(&raw.text, MAX_MESSAGE_BYTES, "message text")?;
    Ok(Message {
        id: MessageId::parse(raw.id)?,
        chat_id: ChatId::parse(raw.chat_id)?,
        ordinal: decode_u64(raw.ordinal, "message ordinal")?,
        role: MessageRole::from_str(&raw.role)?,
        status: MessageStatus::from_str(&raw.status)?,
        text: raw.text,
        created_at_ms: TimestampMillis::new(raw.created_at_ms)?,
        updated_at_ms: TimestampMillis::new(raw.updated_at_ms)?,
    })
}

#[derive(Debug)]
struct RawRequestState {
    id: String,
    chat_id: String,
    user_message_id: String,
    assistant_message_id: String,
    provider_id: String,
    model_id: String,
    status: String,
    last_seq: i64,
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
        status: row.get(6)?,
        last_seq: row.get(7)?,
        provider_response_id: row.get(8)?,
        input_tokens: row.get(9)?,
        output_tokens: row.get(10)?,
        cached_input_tokens: row.get(11)?,
        reasoning_tokens: row.get(12)?,
        failure_code: row.get(13)?,
        started_at_ms: row.get(14)?,
        updated_at_ms: row.get(15)?,
        finished_at_ms: row.get(16)?,
    })
}

fn decode_request_state(raw: RawRequestState) -> Result<RequestState> {
    let usage = decode_usage(
        raw.input_tokens,
        raw.output_tokens,
        raw.cached_input_tokens,
        raw.reasoning_tokens,
    )?;
    Ok(RequestState {
        id: RequestStateId::parse(raw.id)?,
        chat_id: ChatId::parse(raw.chat_id)?,
        user_message_id: MessageId::parse(raw.user_message_id)?,
        assistant_message_id: MessageId::parse(raw.assistant_message_id)?,
        selection: ProviderSelection {
            provider_id: ProviderId::from_str(&raw.provider_id)?,
            model_id: ModelId::parse(raw.model_id)?,
        },
        status: RequestStatus::from_str(&raw.status)?,
        last_seq: decode_u64(raw.last_seq, "request sequence")?,
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

fn apply_response_progress(
    transaction: &Transaction<'_>,
    checkpoint: &ResponseCheckpoint,
    terminal: Option<TerminalOutcome>,
) -> Result<ResponseProgress> {
    validate_checkpoint(checkpoint)?;
    let raw = transaction
        .query_row(
            "SELECT
                id, chat_id, user_message_id, assistant_message_id,
                provider_id, model_id, status, last_seq, provider_response_id,
                input_tokens, output_tokens, cached_input_tokens, reasoning_tokens,
                failure_code, started_at_ms, updated_at_ms, finished_at_ms
             FROM request_state WHERE id = ?1",
            params![checkpoint.request_state_id.as_str()],
            raw_request_state,
        )
        .optional()?
        .ok_or(StorageError::NotFound {
            entity: "request state",
        })?;
    let state = decode_request_state(raw)?;
    if state.status != RequestStatus::Running {
        return Err(StorageError::InvalidState {
            expected: RequestStatus::Running.as_str(),
            actual: state.status.as_str().to_owned(),
        });
    }
    if state.last_seq != checkpoint.expected_last_seq {
        return Err(StorageError::SequenceMismatch {
            expected: state.last_seq,
            actual: checkpoint.expected_last_seq,
        });
    }

    validate_response_metadata(&state, checkpoint)?;
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
    let at_ms = checkpoint.at_ms.get();

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
             updated_at_ms = max(updated_at_ms, ?4)
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
             last_seq = ?3,
             provider_response_id = ?4,
             input_tokens = ?5,
             output_tokens = ?6,
             cached_input_tokens = ?7,
             reasoning_tokens = ?8,
             failure_code = ?9,
             updated_at_ms = max(updated_at_ms, ?10),
             finished_at_ms = ?11
         WHERE id = ?1 AND status = 'running' AND last_seq = ?12",
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
            encode_u64(checkpoint.expected_last_seq, "request sequence")?
        ],
    )?;
    if changed_request != 1 {
        return Err(StorageError::SequenceMismatch {
            expected: state.last_seq,
            actual: checkpoint.expected_last_seq,
        });
    }
    transaction.execute(
        "UPDATE chats SET updated_at_ms = max(updated_at_ms, ?2) WHERE id = ?1",
        params![state.chat_id.as_str(), at_ms],
    )?;

    Ok(ResponseProgress {
        request_state_id: checkpoint.request_state_id.clone(),
        assistant_message_id: state.assistant_message_id,
        last_seq: checkpoint.through_seq,
        text_bytes,
        status: request_status,
    })
}

fn validate_response_metadata(state: &RequestState, checkpoint: &ResponseCheckpoint) -> Result<()> {
    if checkpoint.at_ms < state.started_at_ms || checkpoint.at_ms < state.updated_at_ms {
        return Err(StorageError::InvalidInput {
            field: "checkpoint timestamp",
            reason: "must not move backwards",
        });
    }
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
    if checkpoint.through_seq <= checkpoint.expected_last_seq {
        return Err(StorageError::InvalidInput {
            field: "checkpoint sequence",
            reason: "must advance beyond the expected sequence",
        });
    }
    encode_u64(checkpoint.expected_last_seq, "request sequence")?;
    encode_u64(checkpoint.through_seq, "request sequence")?;
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

fn map_constraint(error: rusqlite::Error, entity: &'static str) -> StorageError {
    match &error {
        rusqlite::Error::SqliteFailure(code, _) if code.code == ErrorCode::ConstraintViolation => {
            StorageError::Conflict { entity }
        }
        _ => StorageError::Database(error),
    }
}
