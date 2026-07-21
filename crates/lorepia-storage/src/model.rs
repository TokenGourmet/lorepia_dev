use std::{fmt, str::FromStr, time::SystemTime};

use serde::Serialize;

use crate::{
    CharacterId, ChatId, MessageId, ModelId, RequestStateId, Result, StorageError, StreamGeneration,
};

pub const CURRENT_SCHEMA_VERSION: i64 = 3;
pub const BUSY_TIMEOUT_MS: u64 = 250;
pub const MAX_CHAT_TITLE_BYTES: usize = 1024;
pub const MAX_USER_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
/// Aggregate native IPC budget for one history page, including conservative
/// per-row envelope allowance in addition to UTF-8 message bodies.
pub const MAX_MESSAGE_PAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CHECKPOINT_BYTES: usize = 64 * 1024;
pub const MAX_PROVIDER_RESPONSE_ID_BYTES: usize = 256;
pub const MAX_STREAM_OWNER_LABEL_BYTES: usize = 128;
pub const MAX_BRANCH_DEPTH: u64 = 1_000_000;
pub const MAX_RENDERED_HTML_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RENDER_CACHE_EVICTION: u16 = 1_000;
pub const MAX_PAGE_SIZE: u16 = 200;
pub const MAX_SEARCH_QUERY_CHARS: usize = 256;
pub const MAX_SHORT_QUERY_SCAN_ROWS: u16 = 4_096;
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct StreamOwnerLabel(String);

impl StreamOwnerLabel {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_STREAM_OWNER_LABEL_BYTES {
            return Err(StorageError::InvalidInput {
                field: "stream owner label",
                reason: "must contain between 1 and 128 bytes",
            });
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'/' | b':' | b'_'))
        {
            return Err(StorageError::InvalidInput {
                field: "stream owner label",
                reason: "contains a character outside the native label alphabet",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StreamOwnerLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for StreamOwnerLabel {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl TryFrom<String> for StreamOwnerLabel {
    type Error = StorageError;

    fn try_from(value: String) -> Result<Self> {
        Self::parse(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct TimestampMillis(i64);

impl TimestampMillis {
    pub fn new(value: i64) -> Result<Self> {
        if value < 0 || value as u64 > MAX_SAFE_INTEGER {
            return Err(StorageError::InvalidInput {
                field: "timestamp",
                reason: "must be a non-negative safe integer",
            });
        }
        Ok(Self(value))
    }

    pub fn now() -> Result<Self> {
        let millis = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| StorageError::ClockBeforeEpoch)?
            .as_millis();
        let millis = i64::try_from(millis).map_err(|_| StorageError::InvalidInput {
            field: "timestamp",
            reason: "exceeds the database range",
        })?;
        Self::new(millis)
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    #[serde(rename = "openai")]
    OpenAi,
    Anthropic,
    #[serde(rename = "deepseek")]
    DeepSeek,
    OllamaCloud,
    Gemini,
    VertexAi,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::DeepSeek => "deepseek",
            Self::OllamaCloud => "ollama_cloud",
            Self::Gemini => "gemini",
            Self::VertexAi => "vertex_ai",
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "deepseek" => Ok(Self::DeepSeek),
            "ollama_cloud" => Ok(Self::OllamaCloud),
            "gemini" => Ok(Self::Gemini),
            "vertex_ai" => Ok(Self::VertexAi),
            _ => Err(StorageError::IncompatibleSchema {
                reason: "database contains an unknown provider ID",
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSelection {
    pub provider_id: ProviderId,
    pub model_id: ModelId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: ChatId,
    pub character_id: CharacterId,
    pub title: String,
    pub revision: u64,
    pub created_at_ms: TimestampMillis,
    pub updated_at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateChat {
    pub character_id: CharacterId,
    pub title: String,
    pub at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatCursor {
    pub updated_at_ms: TimestampMillis,
    pub chat_id: ChatId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatPage {
    pub chats: Vec<Chat>,
    pub next_cursor: Option<ChatCursor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

impl FromStr for MessageRole {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            _ => Err(StorageError::IncompatibleSchema {
                reason: "database contains an unknown message role",
            }),
        }
    }
}

impl MessageRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Complete,
    Partial,
    Failed,
}

impl MessageStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for MessageStatus {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "complete" => Ok(Self::Complete),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            _ => Err(StorageError::IncompatibleSchema {
                reason: "database contains an unknown message status",
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: MessageId,
    pub chat_id: ChatId,
    pub parent_id: Option<MessageId>,
    pub sibling_ord: u64,
    pub depth: u64,
    pub ordinal: u64,
    pub role: MessageRole,
    pub status: MessageStatus,
    pub text: String,
    pub created_at_ms: TimestampMillis,
    pub updated_at_ms: TimestampMillis,
    pub completed_at_ms: Option<TimestampMillis>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub messages: Vec<Message>,
    pub next_ordinal: Option<u64>,
}

/// Exclusive keyset cursor for walking from the newest messages toward older
/// messages in one chat. The embedded chat identity must match the separately
/// supplied query identity so a cursor can never be replayed across chats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageOrdinalCursor {
    pub chat_id: ChatId,
    pub ordinal: u64,
}

impl MessageOrdinalCursor {
    pub fn new(chat_id: ChatId, ordinal: u64) -> Result<Self> {
        if ordinal == 0 || ordinal > MAX_SAFE_INTEGER {
            return Err(StorageError::InvalidInput {
                field: "message cursor ordinal",
                reason: "must be between 1 and the safe integer limit",
            });
        }
        Ok(Self { chat_id, ordinal })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentMessagePage {
    /// Canonical display order, oldest to newest, even though SQLite reads the
    /// newest rows first to keep startup work bounded.
    pub messages: Vec<Message>,
    pub older_cursor: Option<MessageOrdinalCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchCursor {
    pub sibling_ord: u64,
    pub message_id: MessageId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchPage {
    pub messages: Vec<Message>,
    pub next_cursor: Option<BranchCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageTimelineCursor {
    pub created_at_ms: TimestampMillis,
    pub message_id: MessageId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageTimelinePage {
    pub messages: Vec<Message>,
    pub next_cursor: Option<MessageTimelineCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePathEntry {
    pub position: u64,
    pub message: Message,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePathPage {
    pub entries: Vec<ActivePathEntry>,
    pub next_position: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePathSelection {
    pub chat_id: ChatId,
    pub leaf_message_id: MessageId,
    pub path_length: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectActivePath {
    pub chat_id: ChatId,
    pub expected_leaf_id: Option<MessageId>,
    pub leaf_message_id: MessageId,
    pub at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendBranchMessage {
    pub chat_id: ChatId,
    pub parent_id: Option<MessageId>,
    pub expected_active_leaf_id: Option<MessageId>,
    pub role: MessageRole,
    pub text: String,
    pub at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendedBranchMessage {
    pub message: Message,
    pub active_path: ActivePathSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct RendererVersion(u64);

impl RendererVersion {
    pub fn new(value: u64) -> Result<Self> {
        if value == 0 || value > MAX_SAFE_INTEGER {
            return Err(StorageError::InvalidInput {
                field: "renderer version",
                reason: "must be between 1 and the safe integer limit",
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedRender {
    pub message_id: MessageId,
    pub renderer_version: RendererVersion,
    pub html: String,
    pub last_used_at_ms: TimestampMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCacheCas {
    pub renderer_version: RendererVersion,
    pub last_used_at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PutRenderCache {
    pub message_id: MessageId,
    pub renderer_version: RendererVersion,
    pub html: String,
    pub expected: Option<RenderCacheCas>,
    pub at_ms: TimestampMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvictRenderCache {
    pub older_than_ms: TimestampMillis,
    pub limit: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderCacheEviction {
    pub evicted_entries: u64,
    pub evicted_html_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalCheckpointPolicy {
    pub restart_threshold_bytes: Option<u64>,
    pub emergency_truncate_threshold_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalCheckpointTelemetry {
    pub busy: bool,
    pub log_frames: u64,
    pub checkpointed_frames: u64,
    pub remaining_frames: u64,
    pub page_size_bytes: u64,
    pub frame_payload_bytes: u64,
    pub wal_file_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalMaintenanceReport {
    pub passive: WalCheckpointTelemetry,
    pub restart: Option<WalCheckpointTelemetry>,
    pub truncate: Option<WalCheckpointTelemetry>,
    pub restart_threshold_bytes: Option<u64>,
    pub emergency_truncate_threshold_bytes: Option<u64>,
    pub threshold_exceeded: bool,
    pub emergency_truncate_threshold_exceeded: bool,
    pub starvation_observed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSearchHit {
    pub message: Message,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginTurn {
    pub chat_id: ChatId,
    pub selection: ProviderSelection,
    pub owner_label: StreamOwnerLabel,
    pub stream_generation: StreamGeneration,
    pub user_text: String,
    pub started_at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedTurn {
    pub request_state_id: RequestStateId,
    pub user_message_id: MessageId,
    pub assistant_message_id: MessageId,
    pub user_ordinal: u64,
    pub assistant_ordinal: u64,
    pub owner_label: StreamOwnerLabel,
    pub stream_generation: StreamGeneration,
    pub last_delivered_seq: u64,
    pub last_durable_seq: u64,
    pub last_acked_seq: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub reasoning_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseCheckpoint {
    pub request_state_id: RequestStateId,
    pub owner_label: StreamOwnerLabel,
    pub stream_generation: StreamGeneration,
    pub expected_last_durable_seq: u64,
    pub through_seq: u64,
    pub appended_text: String,
    pub provider_response_id: Option<String>,
    pub usage: Option<TokenUsage>,
    pub at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryCheckpoint {
    pub request_state_id: RequestStateId,
    pub owner_label: StreamOwnerLabel,
    pub stream_generation: StreamGeneration,
    pub expected_last_delivered_seq: u64,
    pub through_seq: u64,
    pub at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CumulativeAck {
    pub request_state_id: RequestStateId,
    pub owner_label: StreamOwnerLabel,
    pub stream_generation: StreamGeneration,
    pub expected_last_acked_seq: Option<u64>,
    pub through_seq: u64,
    pub at_ms: TimestampMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RequestFailureCode {
    NetworkUnavailable,
    AuthenticationFailed,
    RateLimited,
    ProviderRejected,
    Timeout,
    ProtocolViolation,
    ResponseTooLarge,
    Internal,
    AppRestarted,
}

impl RequestFailureCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NetworkUnavailable => "NETWORK_UNAVAILABLE",
            Self::AuthenticationFailed => "AUTHENTICATION_FAILED",
            Self::RateLimited => "RATE_LIMITED",
            Self::ProviderRejected => "PROVIDER_REJECTED",
            Self::Timeout => "TIMEOUT",
            Self::ProtocolViolation => "PROTOCOL_VIOLATION",
            Self::ResponseTooLarge => "RESPONSE_TOO_LARGE",
            Self::Internal => "INTERNAL",
            Self::AppRestarted => "APP_RESTARTED",
        }
    }
}

impl FromStr for RequestFailureCode {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "NETWORK_UNAVAILABLE" => Ok(Self::NetworkUnavailable),
            "AUTHENTICATION_FAILED" => Ok(Self::AuthenticationFailed),
            "RATE_LIMITED" => Ok(Self::RateLimited),
            "PROVIDER_REJECTED" => Ok(Self::ProviderRejected),
            "TIMEOUT" => Ok(Self::Timeout),
            "PROTOCOL_VIOLATION" => Ok(Self::ProtocolViolation),
            "RESPONSE_TOO_LARGE" => Ok(Self::ResponseTooLarge),
            "INTERNAL" => Ok(Self::Internal),
            "APP_RESTARTED" => Ok(Self::AppRestarted),
            _ => Err(StorageError::IncompatibleSchema {
                reason: "database contains an unknown failure code",
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl RequestStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }
}

impl FromStr for RequestStatus {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(StorageError::IncompatibleSchema {
                reason: "database contains an unknown request status",
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalOutcome {
    Completed,
    Cancelled,
    Failed(RequestFailureCode),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCheckpoint {
    pub checkpoint: ResponseCheckpoint,
    pub outcome: TerminalOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseProgress {
    pub request_state_id: RequestStateId,
    pub assistant_message_id: MessageId,
    pub last_delivered_seq: u64,
    pub last_durable_seq: u64,
    pub last_acked_seq: Option<u64>,
    pub text_bytes: usize,
    pub status: RequestStatus,
    pub updated_at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSequenceProgress {
    pub request_state_id: RequestStateId,
    pub owner_label: StreamOwnerLabel,
    pub stream_generation: StreamGeneration,
    pub last_delivered_seq: u64,
    pub last_durable_seq: u64,
    pub last_acked_seq: Option<u64>,
    pub status: RequestStatus,
    pub updated_at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestState {
    pub id: RequestStateId,
    pub chat_id: ChatId,
    pub user_message_id: MessageId,
    pub assistant_message_id: MessageId,
    pub selection: ProviderSelection,
    pub owner_label: StreamOwnerLabel,
    pub stream_generation: StreamGeneration,
    pub status: RequestStatus,
    pub last_delivered_seq: u64,
    pub last_durable_seq: u64,
    pub last_acked_seq: Option<u64>,
    pub provider_response_id: Option<String>,
    pub usage: Option<TokenUsage>,
    pub failure_code: Option<RequestFailureCode>,
    pub started_at_ms: TimestampMillis,
    pub updated_at_ms: TimestampMillis,
    pub finished_at_ms: Option<TimestampMillis>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPreferences {
    pub selected_provider_id: ProviderId,
    pub model_ids: ProviderModelIds,
    pub theme: Theme,
    pub default_mode: DefaultMode,
    pub revision: u64,
    pub updated_at_ms: TimestampMillis,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelIds {
    pub openai: String,
    pub anthropic: String,
    pub deepseek: String,
    pub ollama_cloud: String,
    pub gemini: String,
}

impl ProviderModelIds {
    pub fn get(&self, provider_id: ProviderId) -> Option<&str> {
        match provider_id {
            ProviderId::OpenAi => Some(&self.openai),
            ProviderId::Anthropic => Some(&self.anthropic),
            ProviderId::DeepSeek => Some(&self.deepseek),
            ProviderId::OllamaCloud => Some(&self.ollama_cloud),
            ProviderId::Gemini => Some(&self.gemini),
            ProviderId::VertexAi => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

impl FromStr for Theme {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(StorageError::IncompatibleSchema {
                reason: "database contains an unknown theme",
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultMode {
    #[default]
    Chat,
    Story,
}

impl DefaultMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Story => "story",
        }
    }
}

impl FromStr for DefaultMode {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "chat" => Ok(Self::Chat),
            "story" => Ok(Self::Story),
            _ => Err(StorageError::IncompatibleSchema {
                reason: "database contains an unknown default mode",
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdatePreferences {
    pub expected_revision: u64,
    pub selected_provider_id: ProviderId,
    pub model_ids: ProviderModelIds,
    pub theme: Theme,
    pub default_mode: DefaultMode,
    pub at_ms: TimestampMillis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupReport {
    pub schema_version: i64,
    pub journal_mode: String,
    pub busy_timeout_ms: u64,
    pub recovered_request_count: u64,
}
