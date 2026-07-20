use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};

use lorepia_credential_vault::{CredentialVaultError, CredentialVaultErrorCode};
use lorepia_provider_runtime::{
    CompletionReason, EndpointSelection, ProviderCredential, ProviderRunOutcome, ProviderRuntime,
    ProviderStreamEvent as RuntimeStreamEvent, RuntimeError, TokenUsage,
};
use lorepia_providers::{
    AnthropicOptions, ChatMessage, DeepSeekOptions, GenerationOptions, GoogleOptions, MessageRole,
    OllamaCloudOptions, OpenAiOptions, ProviderId, ProviderOptions, ProviderRequest,
    compile_request,
};
use lorepia_storage::{
    BeginTurn, ChatId as StorageChatId, ModelId as StorageModelId, ProviderId as StorageProviderId,
    ProviderSelection as StorageProviderSelection, RequestFailureCode as StorageFailureCode,
    RequestStateId, RequestStatus as StorageRequestStatus, ResponseCheckpoint, StartedTurn,
    TerminalCheckpoint, TerminalOutcome, TimestampMillis, TokenUsage as StorageTokenUsage,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tauri::{State, ipc::Channel};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::{
    credential_commands::{CredentialVaultState, run_vault_operation},
    storage_commands::{StorageCommandError, StorageState},
};

const MAX_ACTIVE_STREAMS: usize = 128;
const MAX_IN_FLIGHT: u64 = 4;
const ACK_TIMEOUT: Duration = Duration::from_secs(30);
const TERMINAL_RETENTION: Duration = Duration::from_secs(5 * 60);
const DIRECT_CHANNEL_BUDGET_BYTES: usize = 4_096;
const MAX_DELTA_FRAGMENT_BYTES: usize = 512;
const MAX_PROVIDER_RESPONSE_ID_BYTES: usize = 256;
const FIRST_CHAT_MAX_INPUT_BYTES: usize = 64 * 1024;
const FIRST_CHAT_MAX_OUTPUT_TOKENS: u32 = 512;
const STORAGE_FLUSH_BYTES: usize = 4 * 1024;
const STORAGE_FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const TERMINAL_STORAGE_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(25), Duration::from_millis(100)];
const FIRST_CHAT_SYSTEM_PROMPT: &str = "You are Seraphine, the librarian of a moonlit archive. Stay in character, answer the user's latest message naturally in the user's language, and never claim access to tools, memories, or facts that are not included in this request.";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderCommandError {
    code: String,
    message: String,
    http_status: Option<u16>,
    retriable: bool,
}

impl ProviderCommandError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: truncate_utf8(code.into(), 64),
            message: truncate_utf8(message.into(), 512),
            http_status: None,
            retriable: false,
        }
    }

    fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, message)
    }

    fn from_runtime(error: RuntimeError) -> Self {
        Self {
            code: truncate_utf8(error.code().to_owned(), 64),
            message: truncate_utf8(error.message().to_owned(), 512),
            http_status: error.http_status(),
            retriable: error.is_retriable(),
        }
    }

    fn from_vault(error: CredentialVaultError) -> Self {
        let code = match error.code {
            CredentialVaultErrorCode::UnsupportedProvider => "CREDENTIAL_UNSUPPORTED",
            CredentialVaultErrorCode::InvalidSecret => "CREDENTIAL_INVALID",
            CredentialVaultErrorCode::SecretTooLarge => "CREDENTIAL_TOO_LARGE",
            CredentialVaultErrorCode::NotConfigured => "CREDENTIAL_NOT_CONFIGURED",
            CredentialVaultErrorCode::StoreUnavailable => "CREDENTIAL_STORE_UNAVAILABLE",
            CredentialVaultErrorCode::StoreLocked => "CREDENTIAL_STORE_LOCKED",
            CredentialVaultErrorCode::StoreFailure => "CREDENTIAL_STORE_FAILED",
            CredentialVaultErrorCode::InternalState => "CREDENTIAL_INTERNAL_STATE",
        };
        Self::new(code, error.to_string())
    }

    fn from_storage(error: StorageCommandError) -> Self {
        Self::new(error.code, error.message)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ProviderChannelEvent {
    Started {
        request_id: String,
        seq: u64,
        max_in_flight: u64,
    },
    ProviderResponseId {
        request_id: String,
        seq: u64,
        id: String,
    },
    TextDelta {
        request_id: String,
        seq: u64,
        text: String,
    },
    ReasoningDelta {
        request_id: String,
        seq: u64,
        text: String,
    },
    RefusalDelta {
        request_id: String,
        seq: u64,
        text: String,
    },
    Usage {
        request_id: String,
        seq: u64,
        usage: TokenUsage,
    },
    Completed {
        request_id: String,
        seq: u64,
        reason: Option<CompletionReason>,
        usage: Option<TokenUsage>,
    },
    Cancelled {
        request_id: String,
        seq: u64,
    },
    Failed {
        request_id: String,
        seq: u64,
        error: ProviderCommandError,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum TerminalReceipt {
    Completed {
        seq: u64,
        reason: Option<CompletionReason>,
        usage: Option<TokenUsage>,
    },
    Cancelled {
        seq: u64,
    },
    Failed {
        seq: u64,
        error: ProviderCommandError,
    },
}

impl TerminalReceipt {
    const fn seq(&self) -> u64 {
        match self {
            Self::Completed { seq, .. } | Self::Cancelled { seq } | Self::Failed { seq, .. } => {
                *seq
            }
        }
    }
}

#[derive(Debug)]
struct StreamMachine {
    last_sent_seq: u64,
    acknowledged_through: Option<u64>,
    cancel_requested: bool,
    terminal: Option<TerminalReceipt>,
    terminal_snapshot_returned: bool,
    terminal_committing: bool,
}

impl StreamMachine {
    const fn after_started() -> Self {
        Self {
            last_sent_seq: 0,
            acknowledged_through: None,
            cancel_requested: false,
            terminal: None,
            terminal_snapshot_returned: false,
            terminal_committing: false,
        }
    }

    fn in_flight(&self) -> u64 {
        let acknowledged_count = self
            .acknowledged_through
            .map_or(0, |sequence| sequence.saturating_add(1));
        self.last_sent_seq
            .saturating_add(1)
            .saturating_sub(acknowledged_count)
    }

    fn can_evict(&self) -> bool {
        self.terminal.as_ref().is_some_and(|terminal| {
            self.terminal_snapshot_returned
                && self
                    .acknowledged_through
                    .is_some_and(|acked| acked >= terminal.seq())
        })
    }
}

struct StreamRequestState {
    control_token: String,
    cancellation: CancellationToken,
    machine: AsyncMutex<StreamMachine>,
    notify: Notify,
}

impl StreamRequestState {
    fn new(control_token: String) -> Self {
        Self {
            control_token,
            cancellation: CancellationToken::new(),
            machine: AsyncMutex::new(StreamMachine::after_started()),
            notify: Notify::new(),
        }
    }

    fn authenticates(&self, supplied: &str) -> bool {
        let expected = self.control_token.as_bytes();
        let supplied = supplied.as_bytes();
        expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
    }
}

#[derive(Clone, Default)]
pub(crate) struct ProviderStreamRegistry {
    requests: Arc<Mutex<HashMap<String, Arc<StreamRequestState>>>>,
}

impl ProviderStreamRegistry {
    fn insert_new(&self) -> Result<(String, Arc<StreamRequestState>), ProviderCommandError> {
        let mut requests = self.requests.lock().map_err(|_| {
            ProviderCommandError::internal(
                "STREAM_REGISTRY_FAILED",
                "stream registry is unavailable",
            )
        })?;
        if requests.len() >= MAX_ACTIVE_STREAMS {
            return Err(ProviderCommandError::new(
                "TOO_MANY_ACTIVE_STREAMS",
                "too many provider streams are active",
            ));
        }
        loop {
            let request_id = format!("provider-{}", Uuid::new_v4().simple());
            if requests.contains_key(&request_id) {
                continue;
            }
            let token = Uuid::new_v4().simple().to_string();
            let state = Arc::new(StreamRequestState::new(token));
            requests.insert(request_id.clone(), Arc::clone(&state));
            return Ok((request_id, state));
        }
    }

    fn authenticated(
        &self,
        request_id: &str,
        control_token: &str,
    ) -> Result<Arc<StreamRequestState>, ProviderCommandError> {
        let state = self
            .requests
            .lock()
            .map_err(|_| {
                ProviderCommandError::internal(
                    "STREAM_REGISTRY_FAILED",
                    "stream registry is unavailable",
                )
            })?
            .get(request_id)
            .cloned()
            .ok_or_else(request_not_found)?;
        if state.authenticates(control_token) {
            Ok(state)
        } else {
            Err(request_not_found())
        }
    }

    fn remove_if_same(&self, request_id: &str, expected: &Arc<StreamRequestState>) {
        let Ok(mut requests) = self.requests.lock() else {
            return;
        };
        if requests
            .get(request_id)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            requests.remove(request_id);
        }
    }
}

fn request_not_found() -> ProviderCommandError {
    ProviderCommandError::new("STREAM_NOT_FOUND", "provider stream was not found")
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartProviderStreamResponse {
    request_id: String,
    control_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FirstChatProfile {
    provider_id: ProviderId,
    model_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AckProviderStreamResponse {
    request_id: String,
    acknowledged_through: u64,
    in_flight: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelProviderStreamResponse {
    request_id: String,
    accepted: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderStreamSnapshot {
    request_id: String,
    last_sent_seq: u64,
    acknowledged_through: Option<u64>,
    in_flight: u64,
    cancel_requested: bool,
    terminal: Option<TerminalReceipt>,
}

#[tauri::command]
pub(crate) async fn start_provider_stream(
    chat_id: String,
    profile: FirstChatProfile,
    user_message: String,
    on_event: Channel<ProviderChannelEvent>,
    vault: State<'_, CredentialVaultState>,
    registry: State<'_, ProviderStreamRegistry>,
    storage: State<'_, StorageState>,
) -> Result<StartProviderStreamResponse, ProviderCommandError> {
    let chat_id = StorageChatId::parse(chat_id).map_err(|_| {
        ProviderCommandError::new("FIRST_CHAT_ID_INVALID", "chat identifier is invalid")
    })?;
    let request = build_first_chat_request(profile, user_message)?;
    let selection = storage_provider_selection(&request)?;
    let persisted_user_text = request
        .messages
        .get(1)
        .map(|message| message.content.clone())
        .ok_or_else(|| {
            ProviderCommandError::internal(
                "FIRST_CHAT_INTERNAL_STATE",
                "first chat request did not contain a user message",
            )
        })?;
    let endpoint = EndpointSelection::Official;
    let credential = load_provider_credential(&request, &endpoint, vault.vault()).await?;
    let (request_id, state) = registry.insert_new()?;
    let started_at_ms = TimestampMillis::now().map_err(|_| {
        ProviderCommandError::new("STORAGE_UNAVAILABLE", "local storage is unavailable")
    })?;
    let storage_for_turn = storage.inner().clone();
    let started_turn_result = storage_for_turn
        .run(move |store| {
            store.begin_turn(BeginTurn {
                chat_id,
                selection,
                user_text: persisted_user_text,
                started_at_ms,
            })
        })
        .await;
    let started_turn = match started_turn_result {
        Ok(started_turn) => started_turn,
        Err(error) => {
            registry.remove_if_same(&request_id, &state);
            return Err(ProviderCommandError::from_storage(error));
        }
    };
    let started = ProviderChannelEvent::Started {
        request_id: request_id.clone(),
        seq: 0,
        max_in_flight: MAX_IN_FLIGHT,
    };
    if let Err(error) = send_direct(&on_event, started) {
        fail_started_turn(&storage_for_turn, &started_turn, started_at_ms).await;
        registry.remove_if_same(&request_id, &state);
        return Err(error);
    }

    let control_token = state.control_token.clone();
    let request_id_for_task = request_id.clone();
    let registry_for_task = registry.inner().clone();
    let storage_for_task = storage.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_stream_bridge(StreamBridgeInput {
            request_id: request_id_for_task,
            state,
            request,
            endpoint,
            credential,
            on_event,
            registry: registry_for_task,
            storage: storage_for_task,
            started_turn,
            started_at_ms,
        })
        .await;
    });
    Ok(StartProviderStreamResponse {
        request_id,
        control_token,
    })
}

fn storage_provider_selection(
    request: &ProviderRequest,
) -> Result<StorageProviderSelection, ProviderCommandError> {
    let provider_id = match request.provider {
        ProviderId::OpenAi => StorageProviderId::OpenAi,
        ProviderId::Anthropic => StorageProviderId::Anthropic,
        ProviderId::DeepSeek => StorageProviderId::DeepSeek,
        ProviderId::OllamaCloud => StorageProviderId::OllamaCloud,
        ProviderId::GoogleGemini => StorageProviderId::Gemini,
        ProviderId::GoogleVertexAi => StorageProviderId::VertexAi,
    };
    let model_id = StorageModelId::parse(request.model_id.clone()).map_err(|_| {
        ProviderCommandError::new(
            "FIRST_CHAT_PROFILE_INVALID",
            "provider or model selection is invalid",
        )
    })?;
    Ok(StorageProviderSelection {
        provider_id,
        model_id,
    })
}

async fn fail_started_turn(
    storage: &StorageState,
    started: &StartedTurn,
    started_at_ms: TimestampMillis,
) {
    let checkpoint = ResponseCheckpoint {
        request_state_id: started.request_state_id.clone(),
        expected_last_seq: started.last_seq,
        through_seq: started.last_seq.saturating_add(1),
        appended_text: String::new(),
        provider_response_id: None,
        usage: None,
        at_ms: started_at_ms,
    };
    let _ = storage
        .run(move |store| store.fail_turn(checkpoint, StorageFailureCode::Internal))
        .await;
}

fn build_first_chat_request(
    profile: FirstChatProfile,
    user_message: String,
) -> Result<ProviderRequest, ProviderCommandError> {
    let model_id = profile.model_id.trim().to_owned();
    if model_id != profile.model_id {
        return Err(ProviderCommandError::new(
            "FIRST_CHAT_PROFILE_INVALID",
            "model identifier must use its canonical form",
        ));
    }
    let content = user_message.trim().to_owned();
    if content.is_empty() || content.len() > FIRST_CHAT_MAX_INPUT_BYTES || content.contains('\0') {
        return Err(ProviderCommandError::new(
            "FIRST_CHAT_MESSAGE_INVALID",
            "first chat message is empty or exceeds the product limit",
        ));
    }

    let provider_options = match profile.provider_id {
        ProviderId::OpenAi => ProviderOptions::OpenAi(OpenAiOptions::default()),
        ProviderId::Anthropic => ProviderOptions::Anthropic(AnthropicOptions::default()),
        ProviderId::DeepSeek => ProviderOptions::DeepSeek(DeepSeekOptions::default()),
        ProviderId::OllamaCloud => ProviderOptions::OllamaCloud(OllamaCloudOptions::default()),
        ProviderId::GoogleGemini => ProviderOptions::GoogleGemini(GoogleOptions::default()),
        ProviderId::GoogleVertexAi => {
            return Err(ProviderCommandError::new(
                "VERTEX_OAUTH_NOT_CONFIGURED",
                "Vertex AI requires a native OAuth access-token flow",
            ));
        }
    };
    let request = ProviderRequest {
        provider: profile.provider_id,
        model_id,
        messages: vec![
            ChatMessage::new(MessageRole::System, FIRST_CHAT_SYSTEM_PROMPT),
            ChatMessage::new(MessageRole::User, content),
        ],
        generation: GenerationOptions {
            max_output_tokens: Some(FIRST_CHAT_MAX_OUTPUT_TOKENS),
            ..GenerationOptions::default()
        },
        provider_options,
        tokenizer_override: None,
        additional_parameters: BTreeMap::new(),
    };
    compile_request(&request).map_err(|_| {
        ProviderCommandError::new(
            "FIRST_CHAT_PROFILE_INVALID",
            "provider or model selection is invalid",
        )
    })?;
    Ok(request)
}

#[tauri::command]
pub(crate) async fn ack_provider_stream(
    request_id: String,
    control_token: String,
    seq: u64,
    registry: State<'_, ProviderStreamRegistry>,
) -> Result<AckProviderStreamResponse, ProviderCommandError> {
    let state = registry.authenticated(&request_id, &control_token)?;
    let (response, should_remove) = {
        let mut machine = state.machine.lock().await;
        if seq > machine.last_sent_seq
            || machine
                .acknowledged_through
                .is_some_and(|acknowledged| seq < acknowledged)
        {
            return Err(ProviderCommandError::new(
                "INVALID_ACK",
                "acknowledgement sequence is outside the delivered range",
            ));
        }
        machine.acknowledged_through = Some(seq);
        let response = AckProviderStreamResponse {
            request_id: request_id.clone(),
            acknowledged_through: seq,
            in_flight: machine.in_flight(),
        };
        (response, machine.can_evict())
    };
    state.notify.notify_waiters();
    if should_remove {
        registry.remove_if_same(&request_id, &state);
    }
    Ok(response)
}

#[tauri::command]
pub(crate) async fn cancel_provider_stream(
    request_id: String,
    control_token: String,
    registry: State<'_, ProviderStreamRegistry>,
) -> Result<CancelProviderStreamResponse, ProviderCommandError> {
    let state = registry.authenticated(&request_id, &control_token)?;
    let accepted = {
        let mut machine = state.machine.lock().await;
        if machine.cancel_requested || machine.terminal.is_some() || machine.terminal_committing {
            false
        } else {
            machine.cancel_requested = true;
            true
        }
    };
    if accepted {
        state.cancellation.cancel();
        state.notify.notify_waiters();
    }
    Ok(CancelProviderStreamResponse {
        request_id,
        accepted,
    })
}

#[tauri::command]
pub(crate) async fn get_provider_stream_snapshot(
    request_id: String,
    control_token: String,
    registry: State<'_, ProviderStreamRegistry>,
) -> Result<ProviderStreamSnapshot, ProviderCommandError> {
    let state = registry.authenticated(&request_id, &control_token)?;
    let (snapshot, should_remove) = {
        let mut machine = state.machine.lock().await;
        if machine.terminal.is_some() {
            machine.terminal_snapshot_returned = true;
        }
        let snapshot = ProviderStreamSnapshot {
            request_id: request_id.clone(),
            last_sent_seq: machine.last_sent_seq,
            acknowledged_through: machine.acknowledged_through,
            in_flight: machine.in_flight(),
            cancel_requested: machine.cancel_requested,
            terminal: machine.terminal.clone(),
        };
        (snapshot, machine.can_evict())
    };
    ensure_direct_serializable(&snapshot)?;
    if should_remove {
        registry.remove_if_same(&request_id, &state);
    }
    Ok(snapshot)
}

async fn load_provider_credential(
    request: &ProviderRequest,
    endpoint: &EndpointSelection,
    vault: Arc<lorepia_credential_vault::CredentialVault>,
) -> Result<ProviderCredential, ProviderCommandError> {
    if request.provider == ProviderId::GoogleVertexAi {
        return Err(ProviderCommandError::new(
            "VERTEX_OAUTH_NOT_CONFIGURED",
            "Vertex AI requires a native OAuth access-token flow",
        ));
    }
    let provider = request.provider;
    let loaded = run_vault_operation(move || vault.load_api_key_for_native_use(provider))
        .await
        .map_err(ProviderCommandError::from_vault)?;
    let copied = loaded.as_bytes().to_vec();
    let secret = String::from_utf8(copied).map_err(|error| {
        let mut bytes = error.into_bytes();
        bytes.zeroize();
        ProviderCommandError::new(
            "CREDENTIAL_INVALID_ENCODING",
            "stored provider credential is not valid UTF-8",
        )
    })?;
    match endpoint {
        EndpointSelection::Official => ProviderCredential::for_official(provider, secret),
        EndpointSelection::Override { endpoint } => {
            let host = Url::parse(endpoint.as_str())
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .ok_or_else(|| {
                    ProviderCommandError::new(
                        "INVALID_ENDPOINT",
                        "override endpoint did not contain a DNS host",
                    )
                })?;
            ProviderCredential::for_override_host(host, secret)
        }
    }
    .map_err(ProviderCommandError::from_runtime)
}

struct StreamPersistence {
    request_state_id: RequestStateId,
    last_persisted_seq: u64,
    through_seq: u64,
    appended_text: String,
    provider_response_id: Option<String>,
    usage: StorageTokenUsage,
    has_usage: bool,
    last_at_ms: TimestampMillis,
    flush_deadline: Option<tokio::time::Instant>,
}

impl StreamPersistence {
    fn new(started: StartedTurn, started_at_ms: TimestampMillis) -> Self {
        Self {
            request_state_id: started.request_state_id,
            last_persisted_seq: started.last_seq,
            through_seq: started.last_seq,
            appended_text: String::new(),
            provider_response_id: None,
            usage: StorageTokenUsage::default(),
            has_usage: false,
            last_at_ms: started_at_ms,
            flush_deadline: None,
        }
    }

    fn record(
        &mut self,
        seq: u64,
        visible_text: Option<&str>,
        provider_response_id: Option<&str>,
        usage: Option<&TokenUsage>,
    ) -> Result<(), ProviderCommandError> {
        if seq != self.through_seq.saturating_add(1) {
            return Err(ProviderCommandError::internal(
                "STORAGE_SEQUENCE_INVALID",
                "stream persistence sequence is invalid",
            ));
        }
        if let Some(text) = visible_text {
            self.appended_text.push_str(text);
        }
        if let Some(id) = provider_response_id {
            self.provider_response_id = Some(id.to_owned());
        }
        if let Some(usage) = usage {
            self.merge_usage(usage);
        }
        self.through_seq = seq;
        self.flush_deadline
            .get_or_insert_with(|| tokio::time::Instant::now() + STORAGE_FLUSH_INTERVAL);
        Ok(())
    }

    fn merge_usage(&mut self, usage: &TokenUsage) {
        if let Some(value) = usage.input_tokens {
            self.usage.input_tokens = value;
            self.has_usage = true;
        }
        if let Some(value) = usage.output_tokens {
            self.usage.output_tokens = value;
            self.has_usage = true;
        }
        if let Some(value) = usage.cached_input_tokens {
            self.usage.cached_input_tokens = value;
            self.has_usage = true;
        }
        if let Some(value) = usage.reasoning_tokens {
            self.usage.reasoning_tokens = value;
            self.has_usage = true;
        }
    }

    fn is_dirty(&self) -> bool {
        self.through_seq > self.last_persisted_seq
    }

    fn should_flush_for_size(&self) -> bool {
        self.appended_text.len() >= STORAGE_FLUSH_BYTES
    }

    fn deadline(&self) -> Option<tokio::time::Instant> {
        self.flush_deadline.filter(|_| self.is_dirty())
    }

    fn timestamp(&mut self) -> Result<TimestampMillis, ProviderCommandError> {
        let now = TimestampMillis::now().map_err(|_| {
            ProviderCommandError::new("STORAGE_UNAVAILABLE", "local storage is unavailable")
        })?;
        if now > self.last_at_ms {
            self.last_at_ms = now;
        }
        Ok(self.last_at_ms)
    }

    fn checkpoint(&mut self, through_seq: u64) -> Result<ResponseCheckpoint, ProviderCommandError> {
        let at_ms = self.timestamp()?;
        Ok(ResponseCheckpoint {
            request_state_id: self.request_state_id.clone(),
            expected_last_seq: self.last_persisted_seq,
            through_seq,
            appended_text: std::mem::take(&mut self.appended_text),
            provider_response_id: self.provider_response_id.take(),
            usage: self.has_usage.then_some(self.usage),
            at_ms,
        })
    }

    fn restore_checkpoint_payload(&mut self, checkpoint: &ResponseCheckpoint) {
        if !checkpoint.appended_text.is_empty() {
            let mut restored = checkpoint.appended_text.clone();
            restored.push_str(&self.appended_text);
            self.appended_text = restored;
        }
        if self.provider_response_id.is_none() {
            self.provider_response_id = checkpoint.provider_response_id.clone();
        }
    }

    fn enter_terminal_sequence(&mut self, terminal_seq: u64) -> Result<(), ProviderCommandError> {
        if terminal_seq == self.through_seq.saturating_add(1) {
            self.through_seq = terminal_seq;
            return Ok(());
        }
        if terminal_seq == self.through_seq && self.last_persisted_seq < terminal_seq {
            return Ok(());
        }
        Err(ProviderCommandError::internal(
            "STORAGE_SEQUENCE_INVALID",
            "terminal persistence sequence is invalid",
        ))
    }

    async fn flush(&mut self, storage: &StorageState) -> Result<(), ProviderCommandError> {
        if !self.is_dirty() {
            self.flush_deadline = None;
            return Ok(());
        }
        let through_seq = self.through_seq;
        let checkpoint = self.checkpoint(through_seq)?;
        let operation_checkpoint = checkpoint.clone();
        let result = storage
            .run(move |store| store.checkpoint_response(operation_checkpoint))
            .await;
        match result {
            Ok(progress) if progress.last_seq == through_seq => {
                self.last_persisted_seq = through_seq;
                self.flush_deadline = None;
                Ok(())
            }
            Ok(_) => {
                self.restore_checkpoint_payload(&checkpoint);
                Err(ProviderCommandError::internal(
                    "STORAGE_WRITE_FAILED",
                    "local storage checkpoint was inconsistent",
                ))
            }
            Err(error) => {
                self.restore_checkpoint_payload(&checkpoint);
                Err(ProviderCommandError::from_storage(error))
            }
        }
    }

    async fn finish(
        &mut self,
        storage: &StorageState,
        terminal_seq: u64,
        outcome: TerminalOutcome,
    ) -> Result<(), ProviderCommandError> {
        self.enter_terminal_sequence(terminal_seq)?;
        let checkpoint = self.checkpoint(terminal_seq)?;
        let operation = TerminalCheckpoint {
            checkpoint: checkpoint.clone(),
            outcome,
        };
        match storage.run(move |store| store.finish_turn(operation)).await {
            Ok(progress) if progress.last_seq == terminal_seq => {
                self.last_persisted_seq = terminal_seq;
                self.flush_deadline = None;
                Ok(())
            }
            Ok(_) => {
                self.restore_checkpoint_payload(&checkpoint);
                Err(ProviderCommandError::internal(
                    "STORAGE_WRITE_FAILED",
                    "local storage terminal checkpoint was inconsistent",
                ))
            }
            Err(error) => {
                self.restore_checkpoint_payload(&checkpoint);
                Err(ProviderCommandError::from_storage(error))
            }
        }
    }

    async fn fail_after_terminal_error(
        &mut self,
        storage: &StorageState,
        terminal_seq: u64,
    ) -> Result<StorageRequestStatus, ProviderCommandError> {
        if terminal_seq != self.through_seq || self.last_persisted_seq >= terminal_seq {
            return Err(ProviderCommandError::internal(
                "STORAGE_SEQUENCE_INVALID",
                "terminal recovery sequence is invalid",
            ));
        }
        let checkpoint = self.checkpoint(terminal_seq)?;
        let operation_checkpoint = checkpoint.clone();
        let known_last_seq = self.last_persisted_seq;
        let operation = storage
            .run(move |store| {
                let state = store.get_request_state(&operation_checkpoint.request_state_id)?;
                if state.status != StorageRequestStatus::Running {
                    return Ok((state.status, state.last_seq));
                }
                if state.last_seq >= terminal_seq {
                    return Err(lorepia_storage::StorageError::SequenceMismatch {
                        expected: state.last_seq,
                        actual: terminal_seq,
                    });
                }

                let mut reconciled = operation_checkpoint;
                if state.last_seq != known_last_seq {
                    // An ambiguous earlier write may already own the buffered
                    // payload. Close the request without risking duplicate text.
                    reconciled.appended_text.clear();
                    reconciled.provider_response_id = None;
                    reconciled.usage = None;
                }
                reconciled.expected_last_seq = state.last_seq;
                store
                    .fail_turn(reconciled, StorageFailureCode::Internal)
                    .map(|progress| (progress.status, progress.last_seq))
            })
            .await;

        match operation {
            Ok((status, last_seq)) if status != StorageRequestStatus::Running => {
                self.last_persisted_seq = last_seq;
                self.appended_text.clear();
                self.provider_response_id = None;
                self.flush_deadline = None;
                Ok(status)
            }
            Ok(_) => {
                self.restore_checkpoint_payload(&checkpoint);
                Err(ProviderCommandError::internal(
                    "STORAGE_WRITE_FAILED",
                    "local storage terminal recovery was inconsistent",
                ))
            }
            Err(error) => {
                self.restore_checkpoint_payload(&checkpoint);
                Err(ProviderCommandError::from_storage(error))
            }
        }
    }
}

struct StreamBridgeInput {
    request_id: String,
    state: Arc<StreamRequestState>,
    request: ProviderRequest,
    endpoint: EndpointSelection,
    credential: ProviderCredential,
    on_event: Channel<ProviderChannelEvent>,
    registry: ProviderStreamRegistry,
    storage: StorageState,
    started_turn: StartedTurn,
    started_at_ms: TimestampMillis,
}

async fn run_stream_bridge(input: StreamBridgeInput) {
    let StreamBridgeInput {
        request_id,
        state,
        request,
        endpoint,
        credential,
        on_event: channel,
        registry,
        storage,
        started_turn,
        started_at_ms,
    } = input;
    let mut persistence = StreamPersistence::new(started_turn, started_at_ms);
    let (event_tx, mut event_rx) = mpsc::channel(1);
    let runtime = ProviderRuntime::new();
    let cancellation = state.cancellation.clone();
    // This narrow command builds the first-chat request natively. It uses the
    // classic compiler only because no prompt preset is bound in this slice.
    // Preset-bound sessions remain disabled until native-owned preset IDs and
    // binding state can route them through ProviderRuntime::run_prompt_stream.
    let run = runtime.run_classic_stream(
        request,
        endpoint,
        credential,
        cancellation.clone(),
        event_tx,
    );
    tokio::pin!(run);

    let result = loop {
        let flush_deadline = persistence.deadline();
        tokio::select! {
            biased;
            event = event_rx.recv() => {
                match event {
                    Some(event) => {
                        if let Err(error) = forward_runtime_event(
                            &request_id,
                            &state,
                            &channel,
                            &mut persistence,
                            &storage,
                            event,
                        ).await {
                            state.cancellation.cancel();
                            break Err(error);
                        }
                    }
                    None => break run.await.map_err(ProviderCommandError::from_runtime),
                }
            }
            result = &mut run => {
                let mut forwarding_error = None;
                while let Some(event) = event_rx.recv().await {
                    if let Err(error) = forward_runtime_event(
                        &request_id,
                        &state,
                        &channel,
                        &mut persistence,
                        &storage,
                        event,
                    ).await {
                        forwarding_error = Some(error);
                        break;
                    }
                }
                break forwarding_error.map_or_else(
                    || result.map_err(ProviderCommandError::from_runtime),
                    Err,
                );
            }
            _ = wait_for_storage_flush(flush_deadline) => {
                if let Err(error) = persistence.flush(&storage).await {
                    state.cancellation.cancel();
                    break Err(error);
                }
            }
        }
    };

    let cancelled = state.machine.lock().await.cancel_requested;
    let terminal = if cancelled {
        TerminalKind::Cancelled
    } else {
        match result {
            Ok(ProviderRunOutcome::Cancelled) => TerminalKind::Cancelled,
            Ok(ProviderRunOutcome::Completed { reason, usage }) => {
                TerminalKind::Completed { reason, usage }
            }
            Err(error) => TerminalKind::Failed { error },
        }
    };

    if send_terminal(
        &request_id,
        &state,
        &channel,
        terminal,
        &mut persistence,
        &storage,
    )
    .await
    .is_err()
    {
        registry.remove_if_same(&request_id, &state);
        return;
    }
    schedule_terminal_cleanup(request_id, state, registry);
}

async fn wait_for_storage_flush(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

async fn forward_runtime_event(
    request_id: &str,
    state: &Arc<StreamRequestState>,
    channel: &Channel<ProviderChannelEvent>,
    persistence: &mut StreamPersistence,
    storage: &StorageState,
    event: RuntimeStreamEvent,
) -> Result<(), ProviderCommandError> {
    match event {
        RuntimeStreamEvent::ProviderResponseId { id } => {
            if id.len() > MAX_PROVIDER_RESPONSE_ID_BYTES {
                return Err(ProviderCommandError::new(
                    "PROVIDER_RESPONSE_ID_TOO_LARGE",
                    "provider response identifier exceeded the runtime limit",
                ));
            }
            let persisted_id = id.clone();
            let seq = send_non_terminal(request_id, state, channel, |request_id, seq| {
                ProviderChannelEvent::ProviderResponseId {
                    request_id,
                    seq,
                    id,
                }
            })
            .await?;
            persistence.record(seq, None, Some(&persisted_id), None)?;
        }
        RuntimeStreamEvent::TextDelta { text } => {
            forward_text_fragments(
                request_id,
                state,
                channel,
                persistence,
                storage,
                text,
                DeltaKind::Text,
            )
            .await?;
        }
        RuntimeStreamEvent::ReasoningDelta { text } => {
            forward_text_fragments(
                request_id,
                state,
                channel,
                persistence,
                storage,
                text,
                DeltaKind::Reasoning,
            )
            .await?;
        }
        RuntimeStreamEvent::RefusalDelta { text } => {
            forward_text_fragments(
                request_id,
                state,
                channel,
                persistence,
                storage,
                text,
                DeltaKind::Refusal,
            )
            .await?;
        }
        RuntimeStreamEvent::Usage { usage } => {
            let persisted_usage = usage.clone();
            let seq = send_non_terminal(request_id, state, channel, |request_id, seq| {
                ProviderChannelEvent::Usage {
                    request_id,
                    seq,
                    usage,
                }
            })
            .await?;
            persistence.record(seq, None, None, Some(&persisted_usage))?;
        }
    }
    if persistence.should_flush_for_size() {
        persistence.flush(storage).await?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum DeltaKind {
    Text,
    Reasoning,
    Refusal,
}

async fn forward_text_fragments(
    request_id: &str,
    state: &Arc<StreamRequestState>,
    channel: &Channel<ProviderChannelEvent>,
    persistence: &mut StreamPersistence,
    storage: &StorageState,
    text: String,
    kind: DeltaKind,
) -> Result<(), ProviderCommandError> {
    for fragment in split_utf8(&text, MAX_DELTA_FRAGMENT_BYTES) {
        let persisted_fragment = fragment.clone();
        let seq = send_non_terminal(request_id, state, channel, |request_id, seq| match kind {
            DeltaKind::Text => ProviderChannelEvent::TextDelta {
                request_id,
                seq,
                text: fragment,
            },
            DeltaKind::Reasoning => ProviderChannelEvent::ReasoningDelta {
                request_id,
                seq,
                text: fragment,
            },
            DeltaKind::Refusal => ProviderChannelEvent::RefusalDelta {
                request_id,
                seq,
                text: fragment,
            },
        })
        .await?;
        let visible = match kind {
            DeltaKind::Text | DeltaKind::Refusal => Some(persisted_fragment.as_str()),
            DeltaKind::Reasoning => None,
        };
        persistence.record(seq, visible, None, None)?;
        if persistence.should_flush_for_size() {
            persistence.flush(storage).await?;
        }
    }
    Ok(())
}

async fn send_non_terminal(
    request_id: &str,
    state: &Arc<StreamRequestState>,
    channel: &Channel<ProviderChannelEvent>,
    make_event: impl FnOnce(String, u64) -> ProviderChannelEvent,
) -> Result<u64, ProviderCommandError> {
    let deadline = tokio::time::Instant::now() + ACK_TIMEOUT;
    let mut make_event = Some(make_event);
    loop {
        let notified = state.notify.notified();
        {
            let mut machine = state.machine.lock().await;
            if machine.cancel_requested {
                return Err(ProviderCommandError::new(
                    "STREAM_CANCELLED",
                    "provider stream was cancelled",
                ));
            }
            if machine.terminal.is_some() || machine.terminal_committing {
                return Err(ProviderCommandError::internal(
                    "STREAM_ALREADY_TERMINAL",
                    "provider stream already reached a terminal state",
                ));
            }
            if machine.in_flight() < MAX_IN_FLIGHT {
                let seq = machine.last_sent_seq.saturating_add(1);
                let factory = make_event.take().ok_or_else(|| {
                    ProviderCommandError::internal(
                        "STREAM_INTERNAL_STATE",
                        "provider event factory was already consumed",
                    )
                })?;
                let event = factory(request_id.to_owned(), seq);
                send_direct(channel, event)?;
                machine.last_sent_seq = seq;
                return Ok(seq);
            }
        }
        tokio::select! {
            _ = state.cancellation.cancelled() => {
                return Err(ProviderCommandError::new(
                    "STREAM_CANCELLED",
                    "provider stream was cancelled",
                ));
            }
            _ = notified => {}
            _ = tokio::time::sleep_until(deadline) => {
                return Err(ProviderCommandError::new(
                    "STREAM_ACK_TIMEOUT",
                    "provider stream acknowledgements timed out",
                ));
            }
        }
    }
}

enum TerminalKind {
    Completed {
        reason: Option<CompletionReason>,
        usage: Option<TokenUsage>,
    },
    Cancelled,
    Failed {
        error: ProviderCommandError,
    },
}

async fn send_terminal(
    request_id: &str,
    state: &Arc<StreamRequestState>,
    channel: &Channel<ProviderChannelEvent>,
    mut terminal: TerminalKind,
    persistence: &mut StreamPersistence,
    storage: &StorageState,
) -> Result<(), ProviderCommandError> {
    let seq = {
        let mut machine = state.machine.lock().await;
        if machine.terminal.is_some() {
            return Ok(());
        }
        if machine.terminal_committing {
            return Err(ProviderCommandError::internal(
                "STREAM_TERMINAL_COMMITTING",
                "provider stream terminal state is already committing",
            ));
        }
        if machine.cancel_requested {
            terminal = TerminalKind::Cancelled;
        }
        machine.terminal_committing = true;
        machine.last_sent_seq.saturating_add(1)
    };

    if let TerminalKind::Completed {
        usage: Some(usage), ..
    } = &terminal
    {
        persistence.merge_usage(usage);
    }
    let outcome = match &terminal {
        TerminalKind::Completed { .. } => TerminalOutcome::Completed,
        TerminalKind::Cancelled => TerminalOutcome::Cancelled,
        TerminalKind::Failed { error } => TerminalOutcome::Failed(storage_failure_code(error)),
    };
    if let Err(commit_error) = finish_terminal_with_retry(persistence, storage, seq, outcome).await
    {
        let recovered = recover_failed_terminal_with_retry(persistence, storage, seq).await;
        match recovered {
            Ok(StorageRequestStatus::Completed) if outcome == TerminalOutcome::Completed => {}
            Ok(StorageRequestStatus::Cancelled) => terminal = TerminalKind::Cancelled,
            Ok(StorageRequestStatus::Failed | StorageRequestStatus::Interrupted) => {
                terminal = TerminalKind::Failed {
                    error: commit_error,
                };
            }
            Ok(StorageRequestStatus::Completed | StorageRequestStatus::Running) => {
                terminal = TerminalKind::Failed {
                    error: ProviderCommandError::internal(
                        "STORAGE_TERMINAL_INCONSISTENT",
                        "local storage terminal state was inconsistent",
                    ),
                };
            }
            Err(error) => {
                let mut machine = state.machine.lock().await;
                machine.terminal_committing = false;
                state.notify.notify_waiters();
                return Err(error);
            }
        }
    }

    let (event, receipt) = match terminal {
        TerminalKind::Completed { reason, usage } => (
            ProviderChannelEvent::Completed {
                request_id: request_id.to_owned(),
                seq,
                reason: reason.clone(),
                usage: usage.clone(),
            },
            TerminalReceipt::Completed { seq, reason, usage },
        ),
        TerminalKind::Cancelled => (
            ProviderChannelEvent::Cancelled {
                request_id: request_id.to_owned(),
                seq,
            },
            TerminalReceipt::Cancelled { seq },
        ),
        TerminalKind::Failed { error } => (
            ProviderChannelEvent::Failed {
                request_id: request_id.to_owned(),
                seq,
                error: error.clone(),
            },
            TerminalReceipt::Failed { seq, error },
        ),
    };
    // Terminal delivery uses one reserved slot so cancellation and failure can
    // finish even when all regular in-flight slots are waiting for ACKs.
    let mut machine = state.machine.lock().await;
    if let Err(error) = send_direct(channel, event) {
        machine.terminal_committing = false;
        state.notify.notify_waiters();
        return Err(error);
    }
    machine.last_sent_seq = seq;
    machine.terminal = Some(receipt);
    machine.terminal_committing = false;
    state.notify.notify_waiters();
    Ok(())
}

async fn finish_terminal_with_retry(
    persistence: &mut StreamPersistence,
    storage: &StorageState,
    seq: u64,
    outcome: TerminalOutcome,
) -> Result<(), ProviderCommandError> {
    let mut result = persistence.finish(storage, seq, outcome).await;
    for delay in TERMINAL_STORAGE_RETRY_DELAYS {
        if result.is_ok() {
            break;
        }
        tokio::time::sleep(delay).await;
        result = persistence.finish(storage, seq, outcome).await;
    }
    result
}

async fn recover_failed_terminal_with_retry(
    persistence: &mut StreamPersistence,
    storage: &StorageState,
    seq: u64,
) -> Result<StorageRequestStatus, ProviderCommandError> {
    let mut result = persistence.fail_after_terminal_error(storage, seq).await;
    for delay in TERMINAL_STORAGE_RETRY_DELAYS {
        if result.is_ok() {
            break;
        }
        tokio::time::sleep(delay).await;
        result = persistence.fail_after_terminal_error(storage, seq).await;
    }
    result
}

fn storage_failure_code(error: &ProviderCommandError) -> StorageFailureCode {
    if error.http_status == Some(401) || error.http_status == Some(403) {
        return StorageFailureCode::AuthenticationFailed;
    }
    if error.http_status == Some(429) {
        return StorageFailureCode::RateLimited;
    }
    match error.code.as_str() {
        "DNS_TIMEOUT" | "DNS_RESOLUTION_FAILED" | "DNS_NO_ADDRESSES" => {
            StorageFailureCode::NetworkUnavailable
        }
        "OVERALL_TIMEOUT"
        | "RESPONSE_HEADER_TIMEOUT"
        | "STREAM_IDLE_TIMEOUT"
        | "STREAM_ACK_TIMEOUT"
        | "EXACT_TOKEN_COUNT_TIMEOUT" => StorageFailureCode::Timeout,
        "STREAM_VISIBLE_OUTPUT_TOO_LARGE"
        | "STREAM_EVENT_TOO_LARGE"
        | "PROVIDER_RESPONSE_ID_TOO_LARGE" => StorageFailureCode::ResponseTooLarge,
        "INVALID_STREAM_EVENT"
        | "STREAM_INTERNAL_STATE"
        | "STREAM_ALREADY_TERMINAL"
        | "STORAGE_SEQUENCE_INVALID" => StorageFailureCode::ProtocolViolation,
        "CREDENTIAL_NOT_CONFIGURED" | "CREDENTIAL_UNSUPPORTED" | "CREDENTIAL_INVALID_ENCODING" => {
            StorageFailureCode::AuthenticationFailed
        }
        "PROVIDER_HTTP_ERROR" | "PROVIDER_FAILED" => StorageFailureCode::ProviderRejected,
        _ => StorageFailureCode::Internal,
    }
}

fn send_direct(
    channel: &Channel<ProviderChannelEvent>,
    event: ProviderChannelEvent,
) -> Result<(), ProviderCommandError> {
    ensure_direct_serializable(&event)?;
    channel.send(event).map_err(|_| {
        ProviderCommandError::new(
            "STREAM_CHANNEL_CLOSED",
            "provider stream event channel is closed",
        )
    })
}

fn ensure_direct_serializable(value: &impl Serialize) -> Result<(), ProviderCommandError> {
    let length = serde_json::to_vec(value)
        .map_err(|_| {
            ProviderCommandError::internal(
                "STREAM_EVENT_SERIALIZATION_FAILED",
                "provider stream event could not be serialized",
            )
        })?
        .len();
    if length > DIRECT_CHANNEL_BUDGET_BYTES {
        return Err(ProviderCommandError::new(
            "STREAM_EVENT_TOO_LARGE",
            "provider stream event exceeded the direct IPC budget",
        ));
    }
    Ok(())
}

fn split_utf8(text: &str, max_bytes: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut fragments = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = start.saturating_add(max_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = text[start..]
                .char_indices()
                .nth(1)
                .map_or(text.len(), |(offset, _)| start + offset);
        }
        fragments.push(text[start..end].to_owned());
        start = end;
    }
    fragments
}

fn schedule_terminal_cleanup(
    request_id: String,
    state: Arc<StreamRequestState>,
    registry: ProviderStreamRegistry,
) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(TERMINAL_RETENTION).await;
        registry.remove_if_same(&request_id, &state);
    });
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started_turn() -> StartedTurn {
        StartedTurn {
            request_state_id: RequestStateId::parse("1".repeat(32)).unwrap(),
            user_message_id: lorepia_storage::MessageId::parse("2".repeat(32)).unwrap(),
            assistant_message_id: lorepia_storage::MessageId::parse("3".repeat(32)).unwrap(),
            user_ordinal: 1,
            assistant_ordinal: 2,
            last_seq: 0,
        }
    }

    #[test]
    fn worst_case_delta_fragment_stays_on_direct_channel_path() {
        let text = "\u{0001}".repeat(MAX_DELTA_FRAGMENT_BYTES);
        for fragment in split_utf8(&text, MAX_DELTA_FRAGMENT_BYTES) {
            let event = ProviderChannelEvent::TextDelta {
                request_id: format!("provider-{}", "f".repeat(32)),
                seq: u64::MAX,
                text: fragment,
            };
            assert!(serde_json::to_vec(&event).unwrap().len() <= DIRECT_CHANNEL_BUDGET_BYTES);
        }
    }

    #[test]
    fn utf8_splitting_preserves_text_exactly() {
        let text = format!("{}{}{}", "a".repeat(511), "🦀", "나".repeat(400));
        let fragments = split_utf8(&text, MAX_DELTA_FRAGMENT_BYTES);
        assert_eq!(fragments.concat(), text);
        assert!(
            fragments
                .iter()
                .all(|part| part.len() <= MAX_DELTA_FRAGMENT_BYTES)
        );
    }

    #[test]
    fn control_tokens_are_bound_to_one_request() {
        let state = StreamRequestState::new("0123456789abcdef".to_owned());
        assert!(state.authenticates("0123456789abcdef"));
        assert!(!state.authenticates("0123456789abcdee"));
        assert!(!state.authenticates("short"));
    }

    #[test]
    fn terminal_receipts_also_fit_the_direct_budget() {
        let snapshot = ProviderStreamSnapshot {
            request_id: format!("provider-{}", "f".repeat(32)),
            last_sent_seq: u64::MAX,
            acknowledged_through: Some(u64::MAX),
            in_flight: 0,
            cancel_requested: false,
            terminal: Some(TerminalReceipt::Failed {
                seq: u64::MAX,
                error: ProviderCommandError::new("X".repeat(64), "Y".repeat(512)),
            }),
        };
        ensure_direct_serializable(&snapshot).unwrap();
    }

    #[test]
    fn first_chat_request_is_native_owned_and_minimal() {
        let request = build_first_chat_request(
            FirstChatProfile {
                provider_id: ProviderId::Anthropic,
                model_id: "claude-test".to_owned(),
            },
            "  안녕하세요  ".to_owned(),
        )
        .unwrap();

        assert_eq!(request.provider, ProviderId::Anthropic);
        assert_eq!(request.model_id, "claude-test");
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, MessageRole::System);
        assert_eq!(request.messages[0].content, FIRST_CHAT_SYSTEM_PROMPT);
        assert_eq!(
            request.messages[1],
            ChatMessage::new(MessageRole::User, "안녕하세요")
        );
        assert_eq!(
            request.generation.max_output_tokens,
            Some(FIRST_CHAT_MAX_OUTPUT_TOKENS)
        );
        assert!(request.additional_parameters.is_empty());
        assert!(request.tokenizer_override.is_none());
    }

    #[test]
    fn first_chat_rejects_vertex_invalid_models_and_unbounded_messages() {
        let build = |provider_id, model_id: &str, message: String| {
            build_first_chat_request(
                FirstChatProfile {
                    provider_id,
                    model_id: model_id.to_owned(),
                },
                message,
            )
        };

        assert!(build(ProviderId::GoogleVertexAi, "model", "hello".into()).is_err());
        assert!(build(ProviderId::GoogleGemini, "models/escape", "hello".into()).is_err());
        assert!(build(ProviderId::OpenAi, "bad\nmodel", "hello".into()).is_err());
        assert!(
            build(
                ProviderId::OpenAi,
                "model",
                "a".repeat(FIRST_CHAT_MAX_INPUT_BYTES + 1),
            )
            .is_err()
        );
    }

    #[test]
    fn stream_persistence_batches_only_visible_text_and_merges_partial_usage() {
        let started_at = TimestampMillis::new(10).unwrap();
        let mut persistence = StreamPersistence::new(started_turn(), started_at);
        persistence.record(1, Some("visible"), None, None).unwrap();
        persistence.record(2, None, None, None).unwrap();
        persistence
            .record(
                3,
                Some(" refusal"),
                Some("response-id"),
                Some(&TokenUsage {
                    input_tokens: Some(7),
                    output_tokens: None,
                    reasoning_tokens: Some(2),
                    cached_input_tokens: None,
                    total_tokens: Some(9),
                }),
            )
            .unwrap();
        persistence.merge_usage(&TokenUsage {
            input_tokens: None,
            output_tokens: Some(5),
            reasoning_tokens: None,
            cached_input_tokens: Some(1),
            total_tokens: Some(15),
        });

        assert_eq!(persistence.through_seq, 3);
        assert_eq!(persistence.appended_text, "visible refusal");
        assert_eq!(
            persistence.provider_response_id.as_deref(),
            Some("response-id")
        );
        assert_eq!(persistence.usage.input_tokens, 7);
        assert_eq!(persistence.usage.output_tokens, 5);
        assert_eq!(persistence.usage.reasoning_tokens, 2);
        assert_eq!(persistence.usage.cached_input_tokens, 1);
        assert!(persistence.is_dirty());
    }

    #[test]
    fn terminal_checkpoint_can_be_retried_without_losing_buffered_text() {
        let started_at = TimestampMillis::new(10).unwrap();
        let mut persistence = StreamPersistence::new(started_turn(), started_at);
        persistence.record(1, Some("visible"), None, None).unwrap();

        persistence.enter_terminal_sequence(2).unwrap();
        let first = persistence.checkpoint(2).unwrap();
        persistence.restore_checkpoint_payload(&first);

        persistence.enter_terminal_sequence(2).unwrap();
        let retry = persistence.checkpoint(2).unwrap();
        assert_eq!(retry.expected_last_seq, 0);
        assert_eq!(retry.through_seq, 2);
        assert_eq!(retry.appended_text, "visible");
        assert!(persistence.enter_terminal_sequence(1).is_err());
    }

    #[test]
    fn storage_failure_mapping_never_needs_provider_error_text() {
        let authentication = ProviderCommandError {
            code: "PROVIDER_FAILED".to_owned(),
            message: "provider controlled body".to_owned(),
            http_status: Some(401),
            retriable: false,
        };
        assert_eq!(
            storage_failure_code(&authentication),
            StorageFailureCode::AuthenticationFailed
        );
        let timeout = ProviderCommandError::new("STREAM_ACK_TIMEOUT", "bounded");
        assert_eq!(storage_failure_code(&timeout), StorageFailureCode::Timeout);
    }
}
