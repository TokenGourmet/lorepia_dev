#![forbid(unsafe_code)]

mod error;
mod id;
mod migration;
mod model;
mod store;

pub use error::{Result, StorageError};
pub use id::{
    CharacterId, ChatId, MAX_ID_BYTES, MAX_MODEL_ID_BYTES, MessageId, ModelId, RequestStateId,
    StreamGeneration,
};
pub use model::{
    ActivePathEntry, ActivePathPage, ActivePathSelection, AppPreferences, AppendBranchMessage,
    AppendedBranchMessage, BUSY_TIMEOUT_MS, BeginTurn, BranchCursor, BranchPage,
    CURRENT_SCHEMA_VERSION, CachedRender, Chat, ChatCursor, ChatPage, CreateChat, CumulativeAck,
    DefaultMode, DeliveryCheckpoint, EvictRenderCache, MAX_BRANCH_DEPTH, MAX_CHAT_TITLE_BYTES,
    MAX_CHECKPOINT_BYTES, MAX_MESSAGE_BYTES, MAX_MESSAGE_PAGE_BYTES, MAX_PAGE_SIZE,
    MAX_PROVIDER_RESPONSE_ID_BYTES, MAX_RENDER_CACHE_EVICTION, MAX_RENDERED_HTML_BYTES,
    MAX_SAFE_INTEGER, MAX_SEARCH_QUERY_CHARS, MAX_SHORT_QUERY_SCAN_ROWS,
    MAX_STREAM_OWNER_LABEL_BYTES, MAX_USER_MESSAGE_BYTES, Message, MessageOrdinalCursor,
    MessagePage, MessageRole, MessageSearchHit, MessageStatus, MessageTimelineCursor,
    MessageTimelinePage, ProviderId, ProviderModelIds, ProviderSelection, PutRenderCache,
    RecentMessagePage, RenderCacheCas, RenderCacheEviction, RendererVersion, RequestFailureCode,
    RequestState, RequestStatus, ResponseCheckpoint, ResponseProgress, SelectActivePath,
    StartedTurn, StartupReport, StreamOwnerLabel, StreamSequenceProgress, TerminalCheckpoint,
    TerminalOutcome, Theme, TimestampMillis, TokenUsage, UpdatePreferences, WalCheckpointPolicy,
    WalCheckpointTelemetry, WalMaintenanceReport,
};
pub use store::Store;
