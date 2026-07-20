#![forbid(unsafe_code)]

mod error;
mod id;
mod migration;
mod model;
mod store;

pub use error::{Result, StorageError};
pub use id::{
    CharacterId, ChatId, MAX_ID_BYTES, MAX_MODEL_ID_BYTES, MessageId, ModelId, RequestStateId,
};
pub use model::{
    AppPreferences, BUSY_TIMEOUT_MS, BeginTurn, CURRENT_SCHEMA_VERSION, Chat, ChatCursor, ChatPage,
    CreateChat, DefaultMode, MAX_CHAT_TITLE_BYTES, MAX_CHECKPOINT_BYTES, MAX_MESSAGE_BYTES,
    MAX_PAGE_SIZE, MAX_PROVIDER_RESPONSE_ID_BYTES, MAX_SAFE_INTEGER, MAX_SEARCH_QUERY_CHARS,
    MAX_SHORT_QUERY_SCAN_ROWS, MAX_USER_MESSAGE_BYTES, Message, MessagePage, MessageRole,
    MessageSearchHit, MessageStatus, ProviderId, ProviderModelIds, ProviderSelection,
    RequestFailureCode, RequestState, RequestStatus, ResponseCheckpoint, ResponseProgress,
    StartedTurn, StartupReport, TerminalCheckpoint, TerminalOutcome, Theme, TimestampMillis,
    TokenUsage, UpdatePreferences,
};
pub use store::Store;
