use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use lorepia_storage::{
    AppPreferences, CharacterId, Chat, ChatCursor, ChatId, CreateChat, DefaultMode, Message,
    MessageRole, MessageStatus, ProviderId, ProviderModelIds, StorageError, Store, Theme,
    TimestampMillis, UpdatePreferences,
};
use serde::{Deserialize, Serialize};
use tauri::State;

const STORAGE_FILE_NAME: &str = "lorepia.sqlite3";
const MAX_CHAT_PAGE: u16 = 100;
const MAX_MESSAGE_PAGE: u16 = 200;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageCommandError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

impl StorageCommandError {
    fn unavailable(code: &'static str) -> Self {
        Self {
            code,
            message: "local storage is unavailable",
        }
    }

    pub(crate) fn from_storage(error: StorageError) -> Self {
        match error {
            StorageError::FutureSchema { .. } => Self {
                code: "STORAGE_SCHEMA_TOO_NEW",
                message: "local storage was created by a newer app version",
            },
            StorageError::IncompatibleSchema { .. } => Self {
                code: "STORAGE_INCOMPATIBLE",
                message: "local storage schema is incompatible",
            },
            StorageError::PathUnavailable(_) | StorageError::ClockBeforeEpoch => {
                Self::unavailable("STORAGE_UNAVAILABLE")
            }
            StorageError::NotFound { entity: "chat" } => Self {
                code: "CHAT_NOT_FOUND",
                message: "chat was not found",
            },
            StorageError::Conflict {
                entity: "chat with an active request",
            } => Self {
                code: "CHAT_ACTIVE_STREAM",
                message: "chat has an active response",
            },
            StorageError::Conflict {
                entity: "settings revision",
            } => Self {
                code: "SETTINGS_CONFLICT",
                message: "settings changed concurrently",
            },
            StorageError::Conflict {
                entity: "database lease",
            } => Self {
                code: "STORAGE_ALREADY_OPEN",
                message: "local storage is already open by another app instance",
            },
            StorageError::InvalidInput { .. } => Self {
                code: "STORAGE_INPUT_INVALID",
                message: "storage input is invalid",
            },
            StorageError::NotFound { .. }
            | StorageError::Conflict { .. }
            | StorageError::InvalidState { .. }
            | StorageError::SequenceMismatch { .. }
            | StorageError::Database(_) => Self {
                code: "STORAGE_WRITE_FAILED",
                message: "local storage operation failed",
            },
        }
    }
}

#[derive(Clone)]
enum StorageBackend {
    Ready(Store),
    Unavailable(StorageCommandError),
}

#[derive(Clone)]
pub(crate) struct StorageState {
    backend: Arc<StorageBackend>,
    operation_gate: Arc<Mutex<()>>,
}

impl StorageState {
    pub(crate) fn open(app_local_data_dir: Result<PathBuf, tauri::Error>) -> Self {
        let backend = match app_local_data_dir {
            Ok(directory) => match Store::open(directory.join(STORAGE_FILE_NAME)) {
                Ok(store) => StorageBackend::Ready(store),
                Err(error) => StorageBackend::Unavailable(StorageCommandError::from_storage(error)),
            },
            Err(_) => {
                StorageBackend::Unavailable(StorageCommandError::unavailable("STORAGE_UNAVAILABLE"))
            }
        };
        Self {
            backend: Arc::new(backend),
            operation_gate: Arc::new(Mutex::new(())),
        }
    }

    fn store(&self) -> Result<Store, StorageCommandError> {
        match self.backend.as_ref() {
            StorageBackend::Ready(store) => Ok(store.clone()),
            StorageBackend::Unavailable(error) => Err(error.clone()),
        }
    }

    pub(crate) async fn run<T, F>(&self, operation: F) -> Result<T, StorageCommandError>
    where
        T: Send + 'static,
        F: FnOnce(Store) -> Result<T, StorageError> + Send + 'static,
    {
        let store = self.store()?;
        let operation_gate = Arc::clone(&self.operation_gate);
        tauri::async_runtime::spawn_blocking(move || {
            let _guard = operation_gate.lock().map_err(|_| StorageCommandError {
                code: "STORAGE_TASK_FAILED",
                message: "local storage operation gate failed",
            })?;
            operation(store).map_err(StorageCommandError::from_storage)
        })
        .await
        .map_err(|_| StorageCommandError {
            code: "STORAGE_TASK_FAILED",
            message: "local storage task failed",
        })?
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageStatusResponse {
    available: bool,
    schema_version: Option<u64>,
    error_code: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredChatResponse {
    id: String,
    character_id: String,
    title: String,
    revision: u64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl From<Chat> for StoredChatResponse {
    fn from(chat: Chat) -> Self {
        Self {
            id: chat.id.to_string(),
            character_id: chat.character_id.to_string(),
            title: chat.title,
            revision: chat.revision,
            created_at_ms: chat.created_at_ms.get(),
            updated_at_ms: chat.updated_at_ms.get(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatPageResponse {
    items: Vec<StoredChatResponse>,
    next_cursor: Option<ChatCursorResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ChatCursorInput {
    updated_at_ms: i64,
    chat_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatCursorResponse {
    updated_at_ms: i64,
    chat_id: String,
}

impl From<ChatCursor> for ChatCursorResponse {
    fn from(cursor: ChatCursor) -> Self {
        Self {
            updated_at_ms: cursor.updated_at_ms.get(),
            chat_id: cursor.chat_id.to_string(),
        }
    }
}

fn parse_chat_cursor(cursor: ChatCursorInput) -> Result<ChatCursor, StorageError> {
    Ok(ChatCursor {
        updated_at_ms: TimestampMillis::new(cursor.updated_at_ms)?,
        chat_id: ChatId::parse(cursor.chat_id)?,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredMessageResponse {
    id: String,
    chat_id: String,
    ordinal: u64,
    role: &'static str,
    text: String,
    state: &'static str,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl From<Message> for StoredMessageResponse {
    fn from(message: Message) -> Self {
        Self {
            id: message.id.to_string(),
            chat_id: message.chat_id.to_string(),
            ordinal: message.ordinal,
            role: match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            },
            text: message.text,
            state: match message.status {
                MessageStatus::Complete => "complete",
                MessageStatus::Partial => "partial",
                MessageStatus::Failed => "failed",
            },
            created_at_ms: message.created_at_ms.get(),
            updated_at_ms: message.updated_at_ms.get(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MessagePageResponse {
    items: Vec<StoredMessageResponse>,
    next_ordinal: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteChatResponse {
    chat_id: String,
    deleted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AppPreferencesInput {
    selected_provider_id: String,
    model_ids: BTreeMap<String, String>,
    theme: String,
    default_mode: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppPreferencesValueResponse {
    selected_provider_id: &'static str,
    model_ids: BTreeMap<&'static str, String>,
    theme: &'static str,
    default_mode: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionedAppPreferencesResponse {
    revision: u64,
    value: AppPreferencesValueResponse,
}

fn provider_from_product(value: &str) -> Result<ProviderId, StorageCommandError> {
    match value {
        "openai" => Ok(ProviderId::OpenAi),
        "anthropic" => Ok(ProviderId::Anthropic),
        "deepseek" => Ok(ProviderId::DeepSeek),
        "ollama-cloud" => Ok(ProviderId::OllamaCloud),
        "google-gemini" => Ok(ProviderId::Gemini),
        "google-vertex-ai" => Ok(ProviderId::VertexAi),
        _ => Err(StorageCommandError {
            code: "STORAGE_INPUT_INVALID",
            message: "provider preference is invalid",
        }),
    }
}

fn provider_to_product(value: ProviderId) -> &'static str {
    match value {
        ProviderId::OpenAi => "openai",
        ProviderId::Anthropic => "anthropic",
        ProviderId::DeepSeek => "deepseek",
        ProviderId::OllamaCloud => "ollama-cloud",
        ProviderId::Gemini => "google-gemini",
        ProviderId::VertexAi => "google-vertex-ai",
    }
}

fn parse_theme(value: &str) -> Result<Theme, StorageCommandError> {
    match value {
        "system" => Ok(Theme::System),
        "light" => Ok(Theme::Light),
        "dark" => Ok(Theme::Dark),
        _ => Err(StorageCommandError {
            code: "STORAGE_INPUT_INVALID",
            message: "theme preference is invalid",
        }),
    }
}

fn parse_default_mode(value: &str) -> Result<DefaultMode, StorageCommandError> {
    match value {
        "chat" => Ok(DefaultMode::Chat),
        "story" => Ok(DefaultMode::Story),
        _ => Err(StorageCommandError {
            code: "STORAGE_INPUT_INVALID",
            message: "default mode preference is invalid",
        }),
    }
}

fn model_ids_from_product(
    mut values: BTreeMap<String, String>,
) -> Result<ProviderModelIds, StorageCommandError> {
    const ALLOWED: [&str; 5] = [
        "openai",
        "anthropic",
        "deepseek",
        "ollama-cloud",
        "google-gemini",
    ];
    if values.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(StorageCommandError {
            code: "STORAGE_INPUT_INVALID",
            message: "model preference is invalid",
        });
    }
    Ok(ProviderModelIds {
        openai: values.remove("openai").unwrap_or_default(),
        anthropic: values.remove("anthropic").unwrap_or_default(),
        deepseek: values.remove("deepseek").unwrap_or_default(),
        ollama_cloud: values.remove("ollama-cloud").unwrap_or_default(),
        gemini: values.remove("google-gemini").unwrap_or_default(),
    })
}

fn preferences_response(preferences: AppPreferences) -> VersionedAppPreferencesResponse {
    let mut model_ids = BTreeMap::new();
    model_ids.insert("openai", preferences.model_ids.openai);
    model_ids.insert("anthropic", preferences.model_ids.anthropic);
    model_ids.insert("deepseek", preferences.model_ids.deepseek);
    model_ids.insert("ollama-cloud", preferences.model_ids.ollama_cloud);
    model_ids.insert("google-gemini", preferences.model_ids.gemini);
    VersionedAppPreferencesResponse {
        revision: preferences.revision,
        value: AppPreferencesValueResponse {
            selected_provider_id: provider_to_product(preferences.selected_provider_id),
            model_ids,
            theme: match preferences.theme {
                Theme::System => "system",
                Theme::Light => "light",
                Theme::Dark => "dark",
            },
            default_mode: match preferences.default_mode {
                DefaultMode::Chat => "chat",
                DefaultMode::Story => "story",
            },
        },
    }
}

#[tauri::command]
pub(crate) fn get_storage_status(storage: State<'_, StorageState>) -> StorageStatusResponse {
    match storage.backend.as_ref() {
        StorageBackend::Ready(store) => StorageStatusResponse {
            available: true,
            schema_version: u64::try_from(store.startup_report().schema_version).ok(),
            error_code: None,
        },
        StorageBackend::Unavailable(error) => StorageStatusResponse {
            available: false,
            schema_version: None,
            error_code: Some(error.code),
        },
    }
}

#[tauri::command]
pub(crate) async fn create_chat(
    character_id: String,
    title: String,
    storage: State<'_, StorageState>,
) -> Result<StoredChatResponse, StorageCommandError> {
    let character_id =
        CharacterId::parse(character_id).map_err(StorageCommandError::from_storage)?;
    let at_ms = TimestampMillis::now().map_err(StorageCommandError::from_storage)?;
    storage
        .run(move |store| {
            store.create_chat(CreateChat {
                character_id,
                title,
                at_ms,
            })
        })
        .await
        .map(Into::into)
}

#[tauri::command]
pub(crate) async fn list_chats(
    limit: u16,
    before: Option<ChatCursorInput>,
    storage: State<'_, StorageState>,
) -> Result<ChatPageResponse, StorageCommandError> {
    if limit == 0 || limit > MAX_CHAT_PAGE {
        return Err(StorageCommandError {
            code: "STORAGE_INPUT_INVALID",
            message: "chat page size is invalid",
        });
    }
    let before = before
        .map(parse_chat_cursor)
        .transpose()
        .map_err(StorageCommandError::from_storage)?;
    let page = storage
        .run(move |store| store.list_chats(limit, before.as_ref()))
        .await?;
    Ok(ChatPageResponse {
        items: page.chats.into_iter().map(Into::into).collect(),
        next_cursor: page.next_cursor.map(Into::into),
    })
}

#[tauri::command]
pub(crate) async fn load_chat_messages(
    chat_id: String,
    limit: u16,
    after_ordinal: Option<u64>,
    storage: State<'_, StorageState>,
) -> Result<MessagePageResponse, StorageCommandError> {
    if limit == 0 || limit > MAX_MESSAGE_PAGE {
        return Err(StorageCommandError {
            code: "STORAGE_INPUT_INVALID",
            message: "message page size is invalid",
        });
    }
    let chat_id = ChatId::parse(chat_id).map_err(StorageCommandError::from_storage)?;
    let page = storage
        .run(move |store| store.load_messages(&chat_id, after_ordinal, limit))
        .await?;
    Ok(MessagePageResponse {
        items: page.messages.into_iter().map(Into::into).collect(),
        next_ordinal: page.next_ordinal,
    })
}

#[tauri::command]
pub(crate) async fn delete_chat(
    chat_id: String,
    storage: State<'_, StorageState>,
) -> Result<DeleteChatResponse, StorageCommandError> {
    let chat_id = ChatId::parse(chat_id).map_err(StorageCommandError::from_storage)?;
    let response_id = chat_id.to_string();
    storage
        .run(move |store| store.delete_chat(&chat_id))
        .await?;
    Ok(DeleteChatResponse {
        chat_id: response_id,
        deleted: true,
    })
}

#[tauri::command]
pub(crate) async fn get_app_preferences(
    storage: State<'_, StorageState>,
) -> Result<VersionedAppPreferencesResponse, StorageCommandError> {
    storage
        .run(|store| store.load_preferences())
        .await
        .map(preferences_response)
}

#[tauri::command]
pub(crate) async fn update_app_preferences(
    expected_revision: u64,
    value: AppPreferencesInput,
    storage: State<'_, StorageState>,
) -> Result<VersionedAppPreferencesResponse, StorageCommandError> {
    let selected_provider_id = provider_from_product(&value.selected_provider_id)?;
    let model_ids = model_ids_from_product(value.model_ids)?;
    let theme = parse_theme(&value.theme)?;
    let default_mode = parse_default_mode(&value.default_mode)?;
    let at_ms = TimestampMillis::now().map_err(StorageCommandError::from_storage)?;
    storage
        .run(move |store| {
            store.save_preferences(UpdatePreferences {
                expected_revision,
                selected_provider_id,
                model_ids,
                theme,
                default_mode,
                at_ms,
            })
        })
        .await
        .map(preferences_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_preferences_use_product_ids_and_no_secret_fields() {
        let response = preferences_response(AppPreferences {
            selected_provider_id: ProviderId::Gemini,
            model_ids: ProviderModelIds {
                gemini: "gemini-example".to_owned(),
                ..ProviderModelIds::default()
            },
            theme: Theme::Dark,
            default_mode: DefaultMode::Story,
            revision: 3,
            updated_at_ms: TimestampMillis::new(10).unwrap(),
        });
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["value"]["selectedProviderId"], "google-gemini");
        assert_eq!(json["value"]["modelIds"]["google-gemini"], "gemini-example");
        let serialized = json.to_string().to_ascii_lowercase();
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("credential"));
        assert!(!serialized.contains("controltoken"));
    }

    #[test]
    fn unknown_model_preference_keys_are_rejected() {
        let invalid = BTreeMap::from([("apiKey".to_owned(), "secret".to_owned())]);
        assert!(model_ids_from_product(invalid).is_err());
    }

    #[test]
    fn chat_cursor_input_is_closed_and_range_checked() {
        let valid: ChatCursorInput = serde_json::from_value(serde_json::json!({
            "updatedAtMs": 12,
            "chatId": "a".repeat(32),
        }))
        .unwrap();
        let parsed = parse_chat_cursor(valid).unwrap();
        assert_eq!(parsed.updated_at_ms.get(), 12);
        assert_eq!(parsed.chat_id.as_str(), "a".repeat(32));

        assert!(
            serde_json::from_value::<ChatCursorInput>(serde_json::json!({
                "updatedAtMs": 12,
                "chatId": "a".repeat(32),
                "sql": "not accepted",
            }))
            .is_err()
        );
        assert!(
            parse_chat_cursor(ChatCursorInput {
                updated_at_ms: -1,
                chat_id: "a".repeat(32),
            })
            .is_err()
        );
    }
}
