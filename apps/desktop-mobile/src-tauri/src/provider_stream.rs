use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use lorepia_credential_vault::{CredentialVaultError, CredentialVaultErrorCode};
use lorepia_provider_runtime::{
    CompletionReason, EndpointSelection, ProviderCredential, ProviderRunOutcome, ProviderRuntime,
    ProviderStreamEvent as RuntimeStreamEvent, RuntimeError, RuntimeErrorKind, TokenUsage,
};
use lorepia_providers::{
    AnthropicOptions, ChatMessage, DeepSeekOptions, GenerationOptions, GoogleOptions, MessageRole,
    OllamaCloudOptions, OpenAiOptions, ProviderId, ProviderOptions, ProviderRequest,
    compile_request,
};
use lorepia_storage::{
    BeginTurn, ChatId as StorageChatId, CumulativeAck, DeliveryCheckpoint,
    Message as StoredMessage, MessageRole as StoredMessageRole,
    MessageStatus as StoredMessageStatus, ModelId as StorageModelId,
    ProviderId as StorageProviderId, ProviderSelection as StorageProviderSelection,
    RequestFailureCode as StorageFailureCode, RequestStateId,
    RequestStatus as StorageRequestStatus, ResponseCheckpoint, StartedTurn, StreamGeneration,
    StreamOwnerLabel, TerminalCheckpoint, TerminalOutcome, TimestampMillis,
    TokenUsage as StorageTokenUsage,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tauri::{State, WebviewWindow, ipc::Channel};
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
const ACK_DURABILITY_BARRIER_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_COMMIT_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(5);
const OWNER_RESET_TIMEOUT: Duration = Duration::from_secs(10);
const TERMINAL_RETENTION: Duration = Duration::from_secs(5 * 60);
const DIRECT_CHANNEL_BUDGET_BYTES: usize = 4_096;
const MAX_DELTA_FRAGMENT_BYTES: usize = 512;
/// Largest integer that JavaScript can represent exactly on the IPC wire.
const MAX_WIRE_SEQUENCE: u64 = 9_007_199_254_740_991;
/// Keep the final safe sequence available for exactly one terminal receipt.
const MAX_NON_TERMINAL_SEQUENCE: u64 = MAX_WIRE_SEQUENCE - 1;
const PER_STREAM_IN_FLIGHT_RESERVATION_BYTES: usize =
    DIRECT_CHANNEL_BUDGET_BYTES * (MAX_IN_FLIGHT as usize + 1);
const GLOBAL_IN_FLIGHT_RESERVATION_BYTES: usize =
    PER_STREAM_IN_FLIGHT_RESERVATION_BYTES * MAX_ACTIVE_STREAMS;
const MAX_PROVIDER_RESPONSE_ID_BYTES: usize = 256;
const CHAT_MAX_INPUT_BYTES: usize = 64 * 1024;
const CHAT_MAX_OUTPUT_TOKENS: u32 = 512;
const CHAT_HISTORY_LOAD_LIMIT: u16 = 64;
const CHAT_HISTORY_MAX_MESSAGES: usize = CHAT_HISTORY_LOAD_LIMIT as usize;
/// Product-owned UTF-8 content budget for system, retained history, and the
/// current user message. This is deliberately not described as a token or
/// provider context-window guarantee.
const CHAT_CONTEXT_MAX_UTF8_BYTES: usize = 256 * 1024;
const STORAGE_FLUSH_BYTES: usize = 4 * 1024;
const STORAGE_FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const TERMINAL_STORAGE_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(25), Duration::from_millis(100)];
const TERMINAL_RECOVERY_MAX_DELAY: Duration = Duration::from_secs(30);
const CREDENTIAL_PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(10);
const CHAT_SYSTEM_PROMPT: &str = "You are Seraphine, the librarian of a moonlit archive. Stay in character, answer the user's latest message naturally in the user's language, and never claim access to tools, memories, or facts that are not included in this request.";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderCommandError {
    code: String,
    message: String,
    http_status: Option<u16>,
    retriable: bool,
    #[serde(skip)]
    runtime_kind: Option<RuntimeErrorKind>,
}

impl ProviderCommandError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: truncate_utf8(code.into(), 64),
            message: truncate_utf8(message.into(), 512),
            http_status: None,
            retriable: false,
            runtime_kind: None,
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
            runtime_kind: Some(error.kind()),
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
    last_durable_seq: u64,
    persisted_acked_through: Option<u64>,
    flush_requested_through: Option<u64>,
    ack_deadline: Option<tokio::time::Instant>,
    pending_send_seq: Option<u64>,
    lease_failure: Option<ProviderCommandError>,
    terminal: Option<TerminalReceipt>,
    terminal_snapshot_returned: bool,
    terminal_committing: bool,
}

impl StreamMachine {
    fn after_started() -> Self {
        Self {
            last_sent_seq: 0,
            acknowledged_through: None,
            last_durable_seq: 0,
            persisted_acked_through: None,
            flush_requested_through: None,
            ack_deadline: Some(tokio::time::Instant::now() + ACK_TIMEOUT),
            pending_send_seq: None,
            lease_failure: None,
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

    fn record_ack_progress(&mut self, seq: u64) {
        let progressed = self
            .acknowledged_through
            .is_none_or(|acknowledged| seq > acknowledged);
        self.acknowledged_through = Some(seq);
        if self.in_flight() == 0 {
            self.ack_deadline = None;
        } else if progressed {
            self.ack_deadline = Some(tokio::time::Instant::now() + ACK_TIMEOUT);
        }
    }

    fn note_ack_activity(&mut self, seq: u64) {
        if self
            .acknowledged_through
            .is_none_or(|acknowledged| seq > acknowledged)
        {
            self.ack_deadline = Some(tokio::time::Instant::now() + ACK_TIMEOUT);
        }
    }

    fn request_flush_through(&mut self, seq: u64) {
        if seq > self.last_durable_seq {
            self.flush_requested_through = Some(
                self.flush_requested_through
                    .map_or(seq, |requested| requested.max(seq)),
            );
        }
    }

    fn publish_durable(&mut self, seq: u64) {
        self.last_durable_seq = self.last_durable_seq.max(seq);
        if self
            .flush_requested_through
            .is_some_and(|requested| requested <= self.last_durable_seq)
        {
            self.flush_requested_through = None;
        }
    }
}

struct StreamRequestState {
    owner_label: StreamOwnerLabel,
    stream_generation: StreamGeneration,
    request_state_id: Mutex<Option<RequestStateId>>,
    control_token: String,
    cancellation: CancellationToken,
    cancel_requested: AtomicBool,
    forced_failure: AtomicBool,
    ack_gate: AsyncMutex<()>,
    machine: AsyncMutex<StreamMachine>,
    notify: Notify,
    flush_notify: Notify,
}

impl StreamRequestState {
    fn new(
        owner_label: StreamOwnerLabel,
        stream_generation: StreamGeneration,
        control_token: String,
    ) -> Self {
        Self {
            owner_label,
            stream_generation,
            request_state_id: Mutex::new(None),
            control_token,
            cancellation: CancellationToken::new(),
            cancel_requested: AtomicBool::new(false),
            forced_failure: AtomicBool::new(false),
            ack_gate: AsyncMutex::new(()),
            machine: AsyncMutex::new(StreamMachine::after_started()),
            notify: Notify::new(),
            flush_notify: Notify::new(),
        }
    }

    fn authenticates(&self, supplied: &str) -> bool {
        let expected = self.control_token.as_bytes();
        let supplied = supplied.as_bytes();
        expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
    }

    fn request_cancel(&self) -> bool {
        if self.forced_failure.load(Ordering::Acquire) {
            return false;
        }
        let accepted = !self.cancel_requested.swap(true, Ordering::AcqRel);
        if accepted {
            self.cancellation.cancel();
            self.notify.notify_waiters();
        }
        accepted
    }

    fn bind_started_turn(&self, started: &StartedTurn) -> Result<(), ProviderCommandError> {
        if started.owner_label != self.owner_label
            || started.stream_generation != self.stream_generation
            || started.last_delivered_seq != 0
            || started.last_durable_seq != 0
            || started.last_acked_seq.is_some()
        {
            return Err(ProviderCommandError::internal(
                "STREAM_STORAGE_IDENTITY_MISMATCH",
                "local stream identity did not match the request owner",
            ));
        }
        let mut request_state_id = self.request_state_id.lock().map_err(|_| {
            ProviderCommandError::internal(
                "STREAM_REGISTRY_FAILED",
                "stream registry is unavailable",
            )
        })?;
        if request_state_id.is_some() {
            return Err(ProviderCommandError::internal(
                "STREAM_STORAGE_IDENTITY_MISMATCH",
                "local stream identity was already bound",
            ));
        }
        *request_state_id = Some(started.request_state_id.clone());
        Ok(())
    }

    fn request_state_id(&self) -> Result<RequestStateId, ProviderCommandError> {
        self.request_state_id
            .lock()
            .map_err(|_| {
                ProviderCommandError::internal(
                    "STREAM_REGISTRY_FAILED",
                    "stream registry is unavailable",
                )
            })?
            .clone()
            .ok_or_else(|| {
                ProviderCommandError::internal(
                    "STREAM_STORAGE_IDENTITY_MISSING",
                    "local stream identity was not initialized",
                )
            })
    }
}

#[derive(Default)]
struct ProviderStreamRegistryInner {
    requests: HashMap<String, Arc<StreamRequestState>>,
    reserved_in_flight_bytes: usize,
}

#[derive(Clone, Default)]
pub(crate) struct ProviderStreamRegistry {
    inner: Arc<Mutex<ProviderStreamRegistryInner>>,
}

impl ProviderStreamRegistry {
    fn insert_new(
        &self,
        owner_label: StreamOwnerLabel,
    ) -> Result<(String, Arc<StreamRequestState>), ProviderCommandError> {
        let mut registry = self.inner.lock().map_err(|_| {
            ProviderCommandError::internal(
                "STREAM_REGISTRY_FAILED",
                "stream registry is unavailable",
            )
        })?;
        if registry.requests.len() >= MAX_ACTIVE_STREAMS {
            return Err(ProviderCommandError::new(
                "TOO_MANY_ACTIVE_STREAMS",
                "too many provider streams are active",
            ));
        }
        let next_reserved = registry
            .reserved_in_flight_bytes
            .checked_add(PER_STREAM_IN_FLIGHT_RESERVATION_BYTES)
            .ok_or_else(|| {
                ProviderCommandError::internal(
                    "STREAM_CAPACITY_OVERFLOW",
                    "stream byte reservation overflowed",
                )
            })?;
        if next_reserved > GLOBAL_IN_FLIGHT_RESERVATION_BYTES {
            return Err(ProviderCommandError::new(
                "STREAM_BYTE_BUDGET_EXHAUSTED",
                "provider stream byte budget is exhausted",
            ));
        }
        loop {
            let request_id = format!("provider-{}", Uuid::new_v4().simple());
            if registry.requests.contains_key(&request_id) {
                continue;
            }
            let token = Uuid::new_v4().simple().to_string();
            let state = Arc::new(StreamRequestState::new(
                owner_label.clone(),
                StreamGeneration::new(),
                token,
            ));
            registry
                .requests
                .insert(request_id.clone(), Arc::clone(&state));
            registry.reserved_in_flight_bytes = next_reserved;
            return Ok((request_id, state));
        }
    }

    fn authenticated(
        &self,
        request_id: &str,
        control_token: &str,
    ) -> Result<Arc<StreamRequestState>, ProviderCommandError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| {
                ProviderCommandError::internal(
                    "STREAM_REGISTRY_FAILED",
                    "stream registry is unavailable",
                )
            })?
            .requests
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
        let Ok(mut registry) = self.inner.lock() else {
            return;
        };
        if registry
            .requests
            .get(request_id)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            registry.requests.remove(request_id);
            registry.reserved_in_flight_bytes = registry
                .reserved_in_flight_bytes
                .checked_sub(PER_STREAM_IN_FLIGHT_RESERVATION_BYTES)
                .expect("stream byte reservation invariant violated");
        }
    }

    pub(crate) fn cancel_owner(&self, owner_label: &str) -> usize {
        let Ok(requests) = self.requests_for_owner(owner_label) else {
            return 0;
        };
        let mut cancelled = 0;
        for (_, state) in requests {
            cancelled += usize::from(state.request_cancel());
        }
        cancelled
    }

    fn requests_for_owner(
        &self,
        owner_label: &str,
    ) -> Result<Vec<(String, Arc<StreamRequestState>)>, ProviderCommandError> {
        let registry = self.inner.lock().map_err(|_| {
            ProviderCommandError::internal(
                "STREAM_REGISTRY_FAILED",
                "stream registry is unavailable",
            )
        })?;
        Ok(registry
            .requests
            .iter()
            .filter(|(_, state)| state.owner_label.as_str() == owner_label)
            .map(|(request_id, state)| (request_id.clone(), Arc::clone(state)))
            .collect())
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
pub(crate) struct ChatProfile {
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
pub(crate) struct ResetProviderStreamOwnerResponse {
    cancelled: u64,
    terminalized: u64,
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
// The IPC payload plus Tauri-injected window and managed states are intentionally
// kept as distinct arguments so the command boundary remains explicit.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_provider_stream(
    window: WebviewWindow,
    chat_id: String,
    profile: ChatProfile,
    user_message: String,
    on_event: Channel<ProviderChannelEvent>,
    vault: State<'_, CredentialVaultState>,
    registry: State<'_, ProviderStreamRegistry>,
    storage: State<'_, StorageState>,
) -> Result<StartProviderStreamResponse, ProviderCommandError> {
    let chat_id = StorageChatId::parse(chat_id).map_err(|_| {
        ProviderCommandError::new("FIRST_CHAT_ID_INVALID", "chat identifier is invalid")
    })?;
    let storage_for_history = storage.inner().clone();
    let chat_id_for_history = chat_id.clone();
    let history = storage_for_history
        .run_read(move |store| {
            store.load_recent_messages(&chat_id_for_history, None, CHAT_HISTORY_LOAD_LIMIT)
        })
        .await
        .map_err(ProviderCommandError::from_storage)?;
    let request = build_chat_request(profile, user_message, &history.messages)?;
    let selection = storage_provider_selection(&request)?;
    let persisted_user_text = request
        .messages
        .last()
        .filter(|message| message.role == MessageRole::User)
        .map(|message| message.content.clone())
        .ok_or_else(|| {
            ProviderCommandError::internal(
                "FIRST_CHAT_INTERNAL_STATE",
                "chat request did not end with the current user message",
            )
        })?;
    let endpoint = EndpointSelection::Official;
    let owner_label = StreamOwnerLabel::parse(window.label().to_owned()).map_err(|_| {
        ProviderCommandError::internal(
            "STREAM_OWNER_INVALID",
            "native stream owner label is invalid",
        )
    })?;
    let (request_id, state) = registry.insert_new(owner_label)?;
    let started_at_ms = TimestampMillis::now().map_err(|_| {
        ProviderCommandError::new("STORAGE_UNAVAILABLE", "local storage is unavailable")
    })?;
    let storage_for_turn = storage.inner().clone();
    let owner_label_for_turn = state.owner_label.clone();
    let generation_for_turn = state.stream_generation.clone();
    let started_turn_result = storage_for_turn
        .run(move |store| {
            store.begin_turn(BeginTurn {
                chat_id,
                selection,
                owner_label: owner_label_for_turn,
                stream_generation: generation_for_turn,
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
    if let Err(error) = state.bind_started_turn(&started_turn) {
        retire_rejected_start(
            storage_for_turn.clone(),
            started_turn.clone(),
            started_at_ms,
            request_id.clone(),
            Arc::clone(&state),
            registry.inner().clone(),
        )
        .await;
        return Err(error);
    }
    // Persist the submitted user turn before consulting the OS credential
    // store. A locked or unavailable keychain can no longer erase accepted
    // input; the paired assistant row is deterministically failed instead.
    let credential = match load_provider_credential(&request, &endpoint, vault.vault()).await {
        Ok(credential) => credential,
        Err(error) => {
            retire_rejected_start(
                storage_for_turn.clone(),
                started_turn.clone(),
                started_at_ms,
                request_id.clone(),
                Arc::clone(&state),
                registry.inner().clone(),
            )
            .await;
            return Err(error);
        }
    };
    let started = ProviderChannelEvent::Started {
        request_id: request_id.clone(),
        seq: 0,
        max_in_flight: MAX_IN_FLIGHT,
    };
    if let Err(error) = send_direct(&on_event, started) {
        retire_rejected_start(
            storage_for_turn.clone(),
            started_turn.clone(),
            started_at_ms,
            request_id.clone(),
            Arc::clone(&state),
            registry.inner().clone(),
        )
        .await;
        return Err(error);
    }
    state.machine.lock().await.ack_deadline = Some(tokio::time::Instant::now() + ACK_TIMEOUT);

    let control_token = state.control_token.clone();
    let request_id_for_task = request_id.clone();
    let registry_for_task = registry.inner().clone();
    let storage_for_task = storage.inner().clone();
    tauri::async_runtime::spawn(run_ack_watchdog(Arc::clone(&state)));
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
) -> Result<(), ProviderCommandError> {
    let mut result = persist_rejected_start(storage, started, started_at_ms).await;
    for delay in TERMINAL_STORAGE_RETRY_DELAYS {
        if result.is_ok() {
            break;
        }
        tokio::time::sleep(delay).await;
        result = persist_rejected_start(storage, started, started_at_ms).await;
    }
    result
}

async fn persist_rejected_start(
    storage: &StorageState,
    started: &StartedTurn,
    at_ms: TimestampMillis,
) -> Result<(), ProviderCommandError> {
    let started = started.clone();
    storage
        .run(move |store| {
            let mut current = store.get_request_state(&started.request_state_id)?;
            if current.owner_label != started.owner_label
                || current.stream_generation != started.stream_generation
            {
                return Err(lorepia_storage::StorageError::Conflict {
                    entity: "stream identity",
                });
            }
            if current.status != StorageRequestStatus::Running {
                return Ok(());
            }

            let terminal_seq = started.last_durable_seq.saturating_add(1);
            if current.last_delivered_seq == started.last_delivered_seq {
                store.record_response_delivery(DeliveryCheckpoint {
                    request_state_id: started.request_state_id.clone(),
                    owner_label: started.owner_label.clone(),
                    stream_generation: started.stream_generation.clone(),
                    expected_last_delivered_seq: started.last_delivered_seq,
                    through_seq: terminal_seq,
                    at_ms,
                })?;
                current = store.get_request_state(&started.request_state_id)?;
            }
            if current.status != StorageRequestStatus::Running {
                return Ok(());
            }
            if current.last_delivered_seq != terminal_seq
                || current.last_durable_seq != started.last_durable_seq
            {
                return Err(lorepia_storage::StorageError::SequenceMismatch {
                    expected: terminal_seq,
                    actual: current.last_delivered_seq,
                });
            }
            store.fail_turn(
                ResponseCheckpoint {
                    request_state_id: started.request_state_id,
                    owner_label: started.owner_label,
                    stream_generation: started.stream_generation,
                    expected_last_durable_seq: started.last_durable_seq,
                    through_seq: terminal_seq,
                    appended_text: String::new(),
                    provider_response_id: None,
                    usage: None,
                    at_ms,
                },
                StorageFailureCode::Internal,
            )?;
            Ok(())
        })
        .await
        .map_err(ProviderCommandError::from_storage)
}

#[allow(clippy::too_many_arguments)]
async fn retire_rejected_start(
    storage: StorageState,
    started: StartedTurn,
    at_ms: TimestampMillis,
    request_id: String,
    state: Arc<StreamRequestState>,
    registry: ProviderStreamRegistry,
) {
    if fail_started_turn(&storage, &started, at_ms).await.is_ok() {
        registry.remove_if_same(&request_id, &state);
        return;
    }

    // A transient disk-full/WAL failure must not strand this chat in
    // `running`. Keep native ownership after the rejected invoke and retry
    // until the terminal row is durable; app restart is the second recovery
    // boundary if the process exits first.
    tauri::async_runtime::spawn(async move {
        let mut delay = Duration::from_secs(1);
        loop {
            tokio::time::sleep(delay).await;
            if fail_started_turn(&storage, &started, at_ms).await.is_ok() {
                registry.remove_if_same(&request_id, &state);
                return;
            }
            delay = delay.saturating_mul(2).min(TERMINAL_RECOVERY_MAX_DELAY);
        }
    });
}

fn build_chat_request(
    profile: ChatProfile,
    user_message: String,
    stored_history: &[StoredMessage],
) -> Result<ProviderRequest, ProviderCommandError> {
    let model_id = profile.model_id.trim().to_owned();
    if model_id != profile.model_id {
        return Err(ProviderCommandError::new(
            "FIRST_CHAT_PROFILE_INVALID",
            "model identifier must use its canonical form",
        ));
    }
    let content = user_message.trim().to_owned();
    if content.is_empty() || content.len() > CHAT_MAX_INPUT_BYTES || content.contains('\0') {
        return Err(ProviderCommandError::new(
            "FIRST_CHAT_MESSAGE_INVALID",
            "chat message is empty or exceeds the product limit",
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
    let history_byte_budget = CHAT_CONTEXT_MAX_UTF8_BYTES
        .checked_sub(CHAT_SYSTEM_PROMPT.len())
        .and_then(|remaining| remaining.checked_sub(content.len()))
        .ok_or_else(|| {
            ProviderCommandError::new(
                "FIRST_CHAT_MESSAGE_INVALID",
                "chat message exceeds the native context byte budget",
            )
        })?;
    let retained_history = select_completed_history(
        stored_history,
        CHAT_HISTORY_MAX_MESSAGES,
        history_byte_budget,
    );
    let mut messages = Vec::with_capacity(retained_history.len() + 2);
    messages.push(ChatMessage::new(MessageRole::System, CHAT_SYSTEM_PROMPT));
    messages.extend(retained_history);
    // The active path is loaded before `begin_turn`, so the current input is
    // appended exactly once and can never be duplicated from durable history.
    messages.push(ChatMessage::new(MessageRole::User, content));

    let request = ProviderRequest {
        provider: profile.provider_id,
        model_id,
        messages,
        generation: GenerationOptions {
            max_output_tokens: Some(CHAT_MAX_OUTPUT_TOKENS),
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

/// Retains a newest-first suffix of closed turns without truncating message
/// bodies. An unfinished assistant row is paired with the immediately
/// preceding user row and both are excluded, so failed/cancelled generation
/// attempts cannot silently become provider context on retry.
fn select_completed_history(
    stored_history: &[StoredMessage],
    max_messages: usize,
    max_utf8_bytes: usize,
) -> Vec<ChatMessage> {
    let max_pairs = max_messages / 2;
    let mut newest_pairs = Vec::<(ChatMessage, ChatMessage)>::new();
    let mut retained_bytes = 0usize;
    let mut cursor = stored_history.len();

    while cursor > 0 && newest_pairs.len() < max_pairs {
        if cursor < 2 {
            break;
        }
        let assistant = &stored_history[cursor - 1];
        let user = &stored_history[cursor - 2];
        let is_turn_pair =
            user.role == StoredMessageRole::User && assistant.role == StoredMessageRole::Assistant;
        if !is_turn_pair {
            cursor -= 1;
            continue;
        }
        cursor -= 2;

        let is_closed = user.status == StoredMessageStatus::Complete
            && assistant.status == StoredMessageStatus::Complete
            && !user.text.trim().is_empty()
            && !assistant.text.trim().is_empty();
        if !is_closed {
            continue;
        }

        let pair_bytes = match user.text.len().checked_add(assistant.text.len()) {
            Some(bytes) => bytes,
            None => break,
        };
        let projected = match retained_bytes.checked_add(pair_bytes) {
            Some(bytes) => bytes,
            None => break,
        };
        if projected > max_utf8_bytes {
            // A larger newest eligible turn must not be replaced with stale
            // older context merely because the older text happens to fit.
            break;
        }
        retained_bytes = projected;
        newest_pairs.push((
            ChatMessage::new(MessageRole::User, user.text.clone()),
            ChatMessage::new(MessageRole::Assistant, assistant.text.clone()),
        ));
    }

    newest_pairs.reverse();
    newest_pairs
        .into_iter()
        .flat_map(|(user, assistant)| [user, assistant])
        .collect()
}

#[tauri::command]
pub(crate) async fn ack_provider_stream(
    request_id: String,
    control_token: String,
    seq: u64,
    registry: State<'_, ProviderStreamRegistry>,
    storage: State<'_, StorageState>,
) -> Result<AckProviderStreamResponse, ProviderCommandError> {
    let state = registry.authenticated(&request_id, &control_token)?;
    let _ack_guard = state.ack_gate.lock().await;

    // A direct Channel callback may invoke ACK on another task before the
    // sender has reacquired the machine lock and committed `last_sent_seq`.
    // Wait for that exact reserved send to commit or roll back instead of
    // rejecting a legitimate zero-latency ACK as out of range.
    wait_for_send_commit(&state, seq).await?;

    let needs_durability_barrier = {
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
        if machine.acknowledged_through == Some(seq) {
            let response = AckProviderStreamResponse {
                request_id: request_id.clone(),
                acknowledged_through: seq,
                in_flight: machine.in_flight(),
            };
            let should_remove = machine.can_evict();
            drop(machine);
            if should_remove {
                registry.remove_if_same(&request_id, &state);
            }
            return Ok(response);
        }
        if let Some(terminal) = machine.terminal.as_ref() {
            if seq > terminal.seq() {
                return Err(ProviderCommandError::new(
                    "INVALID_ACK",
                    "acknowledgement sequence is outside the delivered range",
                ));
            }
            false
        } else {
            machine.note_ack_activity(seq);
            machine.request_flush_through(seq);
            true
        }
    };

    // Sequence zero is the Started event. The storage journal represents that
    // baseline as `None`; positive cumulative ACKs are always persisted.
    if seq == 0 {
        let (response, should_remove) = {
            let mut machine = state.machine.lock().await;
            machine.record_ack_progress(seq);
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
        return Ok(response);
    }

    if needs_durability_barrier {
        state.flush_notify.notify_one();
        let barrier_deadline = tokio::time::Instant::now() + ACK_DURABILITY_BARRIER_TIMEOUT;
        loop {
            let notified = state.notify.notified();
            let barrier_satisfied = {
                let machine = state.machine.lock().await;
                if let Some(error) = machine.lease_failure.clone() {
                    return Err(error);
                }
                if machine.terminal.is_some() {
                    true
                } else if machine.terminal_committing {
                    false
                } else {
                    machine.last_durable_seq >= seq
                }
            };
            if barrier_satisfied {
                break;
            }
            tokio::select! {
                _ = notified => {}
                _ = state.cancellation.cancelled() => {
                    return Err(ProviderCommandError::new(
                        "STREAM_CANCELLED",
                        "provider stream was cancelled",
                    ));
                }
                _ = tokio::time::sleep_until(barrier_deadline) => {
                    let error = ProviderCommandError::new(
                        "STREAM_ACK_DURABILITY_TIMEOUT",
                        "provider stream acknowledgement durability timed out",
                    );
                    force_stream_failure(&state, error.clone()).await;
                    return Err(error);
                }
            }
        }
    }

    if let Err(error) = persist_cumulative_ack(&storage, &state, seq).await {
        force_stream_failure(&state, error.clone()).await;
        return Err(error);
    }

    let (response, should_remove) = {
        let mut machine = state.machine.lock().await;
        machine.record_ack_progress(seq);
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

async fn wait_for_send_commit(
    state: &Arc<StreamRequestState>,
    seq: u64,
) -> Result<(), ProviderCommandError> {
    let deadline = tokio::time::Instant::now() + SEND_COMMIT_OBSERVATION_TIMEOUT;
    loop {
        let notified = state.notify.notified();
        let pending = state.machine.lock().await.pending_send_seq == Some(seq);
        if !pending {
            return Ok(());
        }
        tokio::select! {
            _ = notified => {}
            _ = state.cancellation.cancelled() => {
                return Err(ProviderCommandError::new(
                    "STREAM_CANCELLED",
                    "provider stream was cancelled",
                ));
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(ProviderCommandError::internal(
                    "STREAM_SEND_COMMIT_TIMEOUT",
                    "provider stream delivery did not reach a coherent state",
                ));
            }
        }
    }
}

async fn wait_for_pending_send_resolution(
    state: &Arc<StreamRequestState>,
) -> Result<(), ProviderCommandError> {
    let deadline = tokio::time::Instant::now() + SEND_COMMIT_OBSERVATION_TIMEOUT;
    loop {
        let notified = state.notify.notified();
        if state.machine.lock().await.pending_send_seq.is_none() {
            return Ok(());
        }
        tokio::select! {
            _ = notified => {}
            _ = tokio::time::sleep_until(deadline) => {
                return Err(ProviderCommandError::internal(
                    "STREAM_SEND_COMMIT_TIMEOUT",
                    "provider stream delivery did not reach a coherent state",
                ));
            }
        }
    }
}

async fn persist_cumulative_ack(
    storage: &StorageState,
    state: &Arc<StreamRequestState>,
    seq: u64,
) -> Result<(), ProviderCommandError> {
    let request_state_id = state.request_state_id()?;
    let expected_request_state_id = request_state_id.clone();
    let owner_label = state.owner_label.clone();
    let stream_generation = state.stream_generation.clone();
    let expected_last_acked_seq = state.machine.lock().await.persisted_acked_through;
    let at_ms = TimestampMillis::now().map_err(|_| {
        ProviderCommandError::new("STORAGE_UNAVAILABLE", "local storage is unavailable")
    })?;
    let acknowledgement = CumulativeAck {
        request_state_id,
        owner_label: owner_label.clone(),
        stream_generation: stream_generation.clone(),
        expected_last_acked_seq,
        through_seq: seq,
        at_ms,
    };
    let progress = storage
        .run(move |store| store.acknowledge_response(acknowledgement))
        .await
        .map_err(ProviderCommandError::from_storage)?;
    if progress.request_state_id != expected_request_state_id
        || progress.owner_label != owner_label
        || progress.stream_generation != stream_generation
        || progress.last_acked_seq != Some(seq)
        || progress.last_durable_seq < seq
        || progress.last_durable_seq > progress.last_delivered_seq
    {
        return Err(ProviderCommandError::internal(
            "STREAM_STORAGE_IDENTITY_MISMATCH",
            "local acknowledgement progress was inconsistent",
        ));
    }

    let mut machine = state.machine.lock().await;
    if machine.persisted_acked_through != expected_last_acked_seq {
        return Err(ProviderCommandError::internal(
            "STREAM_INTERNAL_STATE",
            "provider acknowledgement state changed concurrently",
        ));
    }
    machine.persisted_acked_through = Some(seq);
    Ok(())
}

async fn force_stream_failure(state: &Arc<StreamRequestState>, error: ProviderCommandError) {
    {
        let mut machine = state.machine.lock().await;
        if machine.terminal.is_none()
            && !machine.terminal_committing
            && machine.lease_failure.is_none()
        {
            machine.lease_failure = Some(error);
            state.forced_failure.store(true, Ordering::Release);
        }
    }
    state.cancellation.cancel();
    state.notify.notify_waiters();
    state.flush_notify.notify_waiters();
}

#[tauri::command]
pub(crate) async fn cancel_provider_stream(
    request_id: String,
    control_token: String,
    registry: State<'_, ProviderStreamRegistry>,
) -> Result<CancelProviderStreamResponse, ProviderCommandError> {
    let state = registry.authenticated(&request_id, &control_token)?;
    let accepted = {
        let machine = state.machine.lock().await;
        if state.cancel_requested.load(Ordering::Acquire)
            || machine.terminal.is_some()
            || machine.terminal_committing
            || machine.lease_failure.is_some()
        {
            false
        } else {
            state.request_cancel()
        }
    };
    Ok(CancelProviderStreamResponse {
        request_id,
        accepted,
    })
}

#[tauri::command]
pub(crate) async fn reset_provider_stream_owner(
    window: WebviewWindow,
    registry: State<'_, ProviderStreamRegistry>,
) -> Result<ResetProviderStreamOwnerResponse, ProviderCommandError> {
    let owner_label = StreamOwnerLabel::parse(window.label().to_owned()).map_err(|_| {
        ProviderCommandError::internal(
            "STREAM_OWNER_INVALID",
            "native stream owner label is invalid",
        )
    })?;
    reset_owner_streams(owner_label.as_str(), registry.inner()).await
}

async fn reset_owner_streams(
    owner_label: &str,
    registry: &ProviderStreamRegistry,
) -> Result<ResetProviderStreamOwnerResponse, ProviderCommandError> {
    let requests = registry.requests_for_owner(owner_label)?;
    let mut cancelled = 0usize;
    for (_, state) in &requests {
        let machine = state.machine.lock().await;
        if machine.terminal.is_none()
            && !machine.terminal_committing
            && machine.lease_failure.is_none()
            && state.request_cancel()
        {
            cancelled += 1;
        }
    }
    let deadline = tokio::time::Instant::now() + OWNER_RESET_TIMEOUT;
    let mut terminalized = 0usize;

    for (request_id, state) in requests {
        loop {
            let notified = state.notify.notified();
            if state.machine.lock().await.terminal.is_some() {
                registry.remove_if_same(&request_id, &state);
                terminalized += 1;
                break;
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(ProviderCommandError::new(
                        "STREAM_OWNER_RESET_TIMEOUT",
                        "previous provider streams did not terminate in time",
                    ));
                }
            }
        }
    }

    Ok(ResetProviderStreamOwnerResponse {
        cancelled: u64::try_from(cancelled).unwrap_or(u64::MAX),
        terminalized: u64::try_from(terminalized).unwrap_or(u64::MAX),
    })
}

#[tauri::command]
pub(crate) async fn get_provider_stream_snapshot(
    request_id: String,
    control_token: String,
    registry: State<'_, ProviderStreamRegistry>,
) -> Result<ProviderStreamSnapshot, ProviderCommandError> {
    let state = registry.authenticated(&request_id, &control_token)?;
    // A direct Channel callback can re-enter Tauri before the sender
    // reacquires the machine lock and publishes its terminal receipt.
    // Never expose that transient reservation as a coherent snapshot.
    wait_for_pending_send_resolution(&state).await?;
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
            cancel_requested: state.cancel_requested.load(Ordering::Acquire),
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
    // This operation is read-only, so abandoning the join wait after the
    // deadline cannot commit a late credential mutation. The platform call may
    // finish on its blocking worker, but the request deterministically fails.
    let loaded = tokio::time::timeout(
        CREDENTIAL_PREFLIGHT_TIMEOUT,
        run_vault_operation(move || vault.load_api_key_for_native_use(provider)),
    )
    .await
    .map_err(|_| {
        ProviderCommandError::new(
            "CREDENTIAL_PREFLIGHT_TIMEOUT",
            "credential store access timed out",
        )
    })?
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
    owner_label: StreamOwnerLabel,
    stream_generation: StreamGeneration,
    last_delivered_seq: u64,
    last_durable_seq: u64,
    through_seq: u64,
    appended_text: String,
    visible_text_bytes: usize,
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
            owner_label: started.owner_label,
            stream_generation: started.stream_generation,
            last_delivered_seq: started.last_delivered_seq,
            last_durable_seq: started.last_durable_seq,
            through_seq: started.last_durable_seq,
            appended_text: String::new(),
            visible_text_bytes: 0,
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
            self.validate_visible_fragment(text)?;
            self.appended_text.push_str(text);
            self.visible_text_bytes = self
                .visible_text_bytes
                .checked_add(text.len())
                .ok_or_else(response_too_large)?;
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

    fn validate_visible_fragment(&self, text: &str) -> Result<(), ProviderCommandError> {
        if self
            .visible_text_bytes
            .checked_add(text.len())
            .is_none_or(|bytes| bytes > lorepia_storage::MAX_MESSAGE_BYTES)
        {
            return Err(response_too_large());
        }
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
        self.through_seq > self.last_durable_seq
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
            owner_label: self.owner_label.clone(),
            stream_generation: self.stream_generation.clone(),
            expected_last_durable_seq: self.last_durable_seq,
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
        if terminal_seq == self.through_seq && self.last_durable_seq < terminal_seq {
            return Ok(());
        }
        Err(ProviderCommandError::internal(
            "STORAGE_SEQUENCE_INVALID",
            "terminal persistence sequence is invalid",
        ))
    }

    async fn record_delivery(
        &mut self,
        storage: &StorageState,
        seq: u64,
    ) -> Result<(), ProviderCommandError> {
        if seq != self.last_delivered_seq.saturating_add(1) {
            return Err(ProviderCommandError::internal(
                "STORAGE_SEQUENCE_INVALID",
                "stream delivery sequence is invalid",
            ));
        }
        let checkpoint = DeliveryCheckpoint {
            request_state_id: self.request_state_id.clone(),
            owner_label: self.owner_label.clone(),
            stream_generation: self.stream_generation.clone(),
            expected_last_delivered_seq: self.last_delivered_seq,
            through_seq: seq,
            at_ms: self.timestamp()?,
        };
        let operation = checkpoint.clone();
        let progress = storage
            .run(move |store| store.record_response_delivery(operation))
            .await
            .map_err(ProviderCommandError::from_storage)?;
        if progress.request_state_id != self.request_state_id
            || progress.owner_label != self.owner_label
            || progress.stream_generation != self.stream_generation
            || progress.last_delivered_seq != seq
            || progress.last_durable_seq != self.last_durable_seq
            || progress
                .last_acked_seq
                .is_some_and(|acked| acked > progress.last_durable_seq)
        {
            return Err(ProviderCommandError::internal(
                "STREAM_STORAGE_IDENTITY_MISMATCH",
                "local delivery progress was inconsistent",
            ));
        }
        self.last_delivered_seq = seq;
        Ok(())
    }

    async fn flush(
        &mut self,
        storage: &StorageState,
        state: &Arc<StreamRequestState>,
    ) -> Result<(), ProviderCommandError> {
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
            Ok(progress)
                if progress.request_state_id == self.request_state_id
                    && progress.last_delivered_seq == self.last_delivered_seq
                    && progress.last_durable_seq == through_seq
                    && progress
                        .last_acked_seq
                        .is_none_or(|acked| acked <= progress.last_durable_seq) =>
            {
                self.last_durable_seq = through_seq;
                self.flush_deadline = None;
                let mut machine = state.machine.lock().await;
                machine.publish_durable(through_seq);
                drop(machine);
                state.notify.notify_waiters();
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
        state: &Arc<StreamRequestState>,
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
            Ok(progress)
                if progress.request_state_id == self.request_state_id
                    && progress.last_delivered_seq == terminal_seq
                    && progress.last_durable_seq == terminal_seq
                    && progress
                        .last_acked_seq
                        .is_none_or(|acked| acked <= progress.last_durable_seq)
                    && progress.status != StorageRequestStatus::Running =>
            {
                self.last_durable_seq = terminal_seq;
                self.flush_deadline = None;
                let mut machine = state.machine.lock().await;
                machine.publish_durable(terminal_seq);
                drop(machine);
                state.notify.notify_waiters();
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
        request: &Arc<StreamRequestState>,
        terminal_seq: u64,
    ) -> Result<StorageRequestStatus, ProviderCommandError> {
        if terminal_seq != self.through_seq || self.last_durable_seq >= terminal_seq {
            return Err(ProviderCommandError::internal(
                "STORAGE_SEQUENCE_INVALID",
                "terminal recovery sequence is invalid",
            ));
        }
        let checkpoint = self.checkpoint(terminal_seq)?;
        let operation_checkpoint = checkpoint.clone();
        let known_last_durable_seq = self.last_durable_seq;
        let operation = storage
            .run(move |store| {
                let state = store.get_request_state(&operation_checkpoint.request_state_id)?;
                if state.owner_label != operation_checkpoint.owner_label
                    || state.stream_generation != operation_checkpoint.stream_generation
                {
                    return Err(lorepia_storage::StorageError::Conflict {
                        entity: "stream identity",
                    });
                }
                if state.status != StorageRequestStatus::Running {
                    return Ok((state.status, state.last_durable_seq));
                }
                if state.last_delivered_seq < terminal_seq
                    || state.last_durable_seq != known_last_durable_seq
                {
                    return Err(lorepia_storage::StorageError::SequenceMismatch {
                        expected: state.last_durable_seq,
                        actual: known_last_durable_seq,
                    });
                }

                let mut reconciled = operation_checkpoint;
                reconciled.expected_last_durable_seq = state.last_durable_seq;
                store
                    .fail_turn(reconciled, StorageFailureCode::Internal)
                    .map(|progress| (progress.status, progress.last_durable_seq))
            })
            .await;

        match operation {
            Ok((status, last_durable_seq)) if status != StorageRequestStatus::Running => {
                self.last_durable_seq = last_durable_seq;
                self.appended_text.clear();
                self.provider_response_id = None;
                self.flush_deadline = None;
                let mut machine = request.machine.lock().await;
                machine.publish_durable(last_durable_seq);
                drop(machine);
                request.notify.notify_waiters();
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

    let mut result = loop {
        let flush_deadline = persistence.deadline();
        tokio::select! {
            biased;
            _ = state.flush_notify.notified() => {
                if let Err(error) = persistence.flush(&storage, &state).await {
                    state.cancellation.cancel();
                    break Err(error);
                }
            }
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
                if let Err(error) = persistence.flush(&storage, &state).await {
                    state.cancellation.cancel();
                    break Err(error);
                }
            }
        }
    };

    if let Err(error) = persistence.flush(&storage, &state).await {
        state.cancellation.cancel();
        result = Err(error);
    }

    let (lease_failure, cancelled) = {
        let machine = state.machine.lock().await;
        (
            machine.lease_failure.clone(),
            state.cancel_requested.load(Ordering::Acquire),
        )
    };
    let terminal = if let Some(error) = lease_failure {
        TerminalKind::Failed { error }
    } else if cancelled {
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
        terminal.clone(),
        &mut persistence,
        &storage,
    )
    .await
    .is_err()
    {
        // Keep the authenticated request and its buffered persistence state
        // alive until a transient disk/WAL failure clears. Dropping them here
        // would leave a `running` row that wedges the chat until app restart.
        tauri::async_runtime::spawn(recover_terminal_until_durable(
            request_id,
            state,
            channel,
            terminal,
            persistence,
            storage,
            registry,
        ));
        return;
    }
    schedule_terminal_cleanup(request_id, state, registry);
}

async fn run_ack_watchdog(state: Arc<StreamRequestState>) {
    loop {
        let notified = state.notify.notified();
        let deadline = {
            let machine = state.machine.lock().await;
            if machine.terminal.is_some()
                || machine.terminal_committing
                || machine.lease_failure.is_some()
            {
                return;
            }
            machine.ack_deadline
        };

        let Some(deadline) = deadline else {
            tokio::select! {
                _ = state.cancellation.cancelled() => return,
                _ = notified => continue,
            }
        };

        tokio::select! {
            _ = state.cancellation.cancelled() => return,
            _ = notified => continue,
            _ = tokio::time::sleep_until(deadline) => {}
        }

        let timed_out = {
            let mut machine = state.machine.lock().await;
            if machine.terminal.is_some()
                || machine.terminal_committing
                || machine.lease_failure.is_some()
                || machine.in_flight() == 0
                || machine
                    .ack_deadline
                    .is_none_or(|current| current > tokio::time::Instant::now())
            {
                false
            } else {
                machine.lease_failure = Some(ProviderCommandError::new(
                    "STREAM_ACK_TIMEOUT",
                    "provider stream acknowledgements timed out",
                ));
                state.forced_failure.store(true, Ordering::Release);
                true
            }
        };
        if timed_out {
            state.cancellation.cancel();
            state.notify.notify_waiters();
            return;
        }
    }
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
            persistence.record_delivery(storage, seq).await?;
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
            persistence.record_delivery(storage, seq).await?;
            persistence.record(seq, None, None, Some(&persisted_usage))?;
        }
    }
    flush_if_requested_or_pressured(persistence, storage, state).await?;
    Ok(())
}

async fn flush_if_requested_or_pressured(
    persistence: &mut StreamPersistence,
    storage: &StorageState,
    state: &Arc<StreamRequestState>,
) -> Result<(), ProviderCommandError> {
    let (flush_requested, flow_control_pressure) = {
        let machine = state.machine.lock().await;
        (
            machine
                .flush_requested_through
                .is_some_and(|requested| requested <= persistence.through_seq),
            machine.in_flight() >= MAX_IN_FLIGHT.saturating_sub(1),
        )
    };
    if persistence.should_flush_for_size() || flush_requested || flow_control_pressure {
        persistence.flush(storage, state).await?;
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
    if text.is_empty() {
        return Err(ProviderCommandError::new(
            "INVALID_STREAM_EVENT",
            "provider stream text delta was empty",
        ));
    }
    for fragment in split_utf8(&text, MAX_DELTA_FRAGMENT_BYTES) {
        if matches!(kind, DeltaKind::Text | DeltaKind::Refusal) {
            // Reject before Channel delivery so the WebView can never observe
            // bytes that the durable message row cannot commit.
            persistence.validate_visible_fragment(fragment)?;
        }
        let event_text = fragment.to_owned();
        let seq = send_non_terminal(request_id, state, channel, |request_id, seq| match kind {
            DeltaKind::Text => ProviderChannelEvent::TextDelta {
                request_id,
                seq,
                text: event_text,
            },
            DeltaKind::Reasoning => ProviderChannelEvent::ReasoningDelta {
                request_id,
                seq,
                text: event_text,
            },
            DeltaKind::Refusal => ProviderChannelEvent::RefusalDelta {
                request_id,
                seq,
                text: event_text,
            },
        })
        .await?;
        persistence.record_delivery(storage, seq).await?;
        let visible = match kind {
            DeltaKind::Text | DeltaKind::Refusal => Some(fragment),
            DeltaKind::Reasoning => None,
        };
        persistence.record(seq, visible, None, None)?;
        flush_if_requested_or_pressured(persistence, storage, state).await?;
    }
    Ok(())
}

fn response_too_large() -> ProviderCommandError {
    ProviderCommandError::new(
        "STREAM_VISIBLE_OUTPUT_TOO_LARGE",
        "provider stream visible output exceeded the product limit",
    )
}

fn next_wire_sequence(last_sent_seq: u64, maximum: u64) -> Result<u64, ProviderCommandError> {
    last_sent_seq
        .checked_add(1)
        .filter(|next| *next <= maximum)
        .ok_or_else(|| {
            ProviderCommandError::new(
                "STREAM_SEQUENCE_EXHAUSTED",
                "provider stream exhausted its JavaScript-safe sequence budget",
            )
        })
}

async fn send_non_terminal(
    request_id: &str,
    state: &Arc<StreamRequestState>,
    channel: &Channel<ProviderChannelEvent>,
    make_event: impl FnOnce(String, u64) -> ProviderChannelEvent,
) -> Result<u64, ProviderCommandError> {
    let mut make_event = Some(make_event);
    loop {
        let notified = state.notify.notified();
        let reserved_seq = {
            let mut machine = state.machine.lock().await;
            // The ACK watchdog publishes its failure while holding this same
            // lock and only cancels the token after releasing it.  Check the
            // published failure first so no sender can reserve one more event
            // in that intentional hand-off window.
            if let Some(error) = machine.lease_failure.clone() {
                return Err(error);
            }
            if state.cancel_requested.load(Ordering::Acquire) || state.cancellation.is_cancelled() {
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
            if machine.pending_send_seq.is_none() && machine.in_flight() < MAX_IN_FLIGHT {
                let seq = next_wire_sequence(machine.last_sent_seq, MAX_NON_TERMINAL_SEQUENCE)?;
                machine.pending_send_seq = Some(seq);
                Some(seq)
            } else {
                None
            }
        };

        if let Some(seq) = reserved_seq {
            let factory = make_event.take().ok_or_else(|| {
                ProviderCommandError::internal(
                    "STREAM_INTERNAL_STATE",
                    "provider event factory was already consumed",
                )
            })?;
            let event = factory(request_id.to_owned(), seq);
            let send_result = send_direct(channel, event);

            let mut machine = state.machine.lock().await;
            if machine.pending_send_seq != Some(seq) {
                drop(machine);
                state.notify.notify_waiters();
                return Err(ProviderCommandError::internal(
                    "STREAM_INTERNAL_STATE",
                    "provider event reservation was lost",
                ));
            }
            machine.pending_send_seq = None;
            match send_result {
                Ok(()) => {
                    let had_outstanding = machine.in_flight() > 0;
                    machine.last_sent_seq = seq;
                    if !had_outstanding || machine.ack_deadline.is_none() {
                        machine.ack_deadline = Some(tokio::time::Instant::now() + ACK_TIMEOUT);
                    }
                    drop(machine);
                    state.notify.notify_waiters();
                    return Ok(seq);
                }
                Err(error) => {
                    drop(machine);
                    state.notify.notify_waiters();
                    return Err(error);
                }
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
        }
    }
}

#[derive(Clone)]
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
    let ack_guard = state.ack_gate.lock().await;
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
        if state.cancel_requested.load(Ordering::Acquire) {
            terminal = TerminalKind::Cancelled;
        }
        if machine.pending_send_seq.is_some() {
            return Err(ProviderCommandError::internal(
                "STREAM_INTERNAL_STATE",
                "provider stream still had a pending event send",
            ));
        }
        let seq = next_wire_sequence(machine.last_sent_seq, MAX_WIRE_SEQUENCE)?;
        machine.terminal_committing = true;
        seq
    };

    if persistence.last_delivered_seq < seq {
        if let Err(error) = persistence.record_delivery(storage, seq).await {
            let mut machine = state.machine.lock().await;
            machine.terminal_committing = false;
            drop(machine);
            state.notify.notify_waiters();
            return Err(error);
        }
    } else if persistence.last_delivered_seq != seq {
        let mut machine = state.machine.lock().await;
        machine.terminal_committing = false;
        drop(machine);
        state.notify.notify_waiters();
        return Err(ProviderCommandError::internal(
            "STORAGE_SEQUENCE_INVALID",
            "terminal delivery sequence is inconsistent",
        ));
    }

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
    if let Err(commit_error) =
        finish_terminal_with_retry(persistence, storage, state, seq, outcome).await
    {
        let recovered = recover_failed_terminal_with_retry(persistence, storage, state, seq).await;
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
    {
        let mut machine = state.machine.lock().await;
        if machine.pending_send_seq.is_some() {
            machine.terminal_committing = false;
            drop(machine);
            state.notify.notify_waiters();
            return Err(ProviderCommandError::internal(
                "STREAM_INTERNAL_STATE",
                "provider stream acquired a conflicting terminal send reservation",
            ));
        }
        machine.pending_send_seq = Some(seq);
    }
    drop(ack_guard);

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
    let send_result = send_direct(channel, event);
    let mut machine = state.machine.lock().await;
    if machine.pending_send_seq != Some(seq) {
        machine.terminal_committing = false;
        state.notify.notify_waiters();
        return Err(ProviderCommandError::internal(
            "STREAM_INTERNAL_STATE",
            "provider terminal send reservation was lost",
        ));
    }
    machine.pending_send_seq = None;
    // Storage is already terminal and is the source of truth. Publish the
    // receipt even if the Channel closed so an authenticated recovery poll can
    // synthesize the missing terminal event from the snapshot.
    machine.last_sent_seq = seq;
    machine.terminal = Some(receipt);
    machine.terminal_committing = false;
    state.notify.notify_waiters();
    let _ = send_result;
    Ok(())
}

async fn recover_terminal_until_durable(
    request_id: String,
    state: Arc<StreamRequestState>,
    channel: Channel<ProviderChannelEvent>,
    terminal: TerminalKind,
    mut persistence: StreamPersistence,
    storage: StorageState,
    registry: ProviderStreamRegistry,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        tokio::time::sleep(delay).await;
        if send_terminal(
            &request_id,
            &state,
            &channel,
            terminal.clone(),
            &mut persistence,
            &storage,
        )
        .await
        .is_ok()
        {
            schedule_terminal_cleanup(request_id, state, registry);
            return;
        }
        delay = delay.saturating_mul(2).min(TERMINAL_RECOVERY_MAX_DELAY);
    }
}

async fn finish_terminal_with_retry(
    persistence: &mut StreamPersistence,
    storage: &StorageState,
    state: &Arc<StreamRequestState>,
    seq: u64,
    outcome: TerminalOutcome,
) -> Result<(), ProviderCommandError> {
    let mut result = persistence.finish(storage, state, seq, outcome).await;
    for delay in TERMINAL_STORAGE_RETRY_DELAYS {
        if result.is_ok() {
            break;
        }
        tokio::time::sleep(delay).await;
        result = persistence.finish(storage, state, seq, outcome).await;
    }
    result
}

async fn recover_failed_terminal_with_retry(
    persistence: &mut StreamPersistence,
    storage: &StorageState,
    state: &Arc<StreamRequestState>,
    seq: u64,
) -> Result<StorageRequestStatus, ProviderCommandError> {
    let mut result = persistence
        .fail_after_terminal_error(storage, state, seq)
        .await;
    for delay in TERMINAL_STORAGE_RETRY_DELAYS {
        if result.is_ok() {
            break;
        }
        tokio::time::sleep(delay).await;
        result = persistence
            .fail_after_terminal_error(storage, state, seq)
            .await;
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
    if error.http_status.is_some() {
        return StorageFailureCode::ProviderRejected;
    }
    if let Some(kind) = error.runtime_kind {
        return match kind {
            RuntimeErrorKind::DnsResolution | RuntimeErrorKind::Http => {
                StorageFailureCode::NetworkUnavailable
            }
            RuntimeErrorKind::CredentialMismatch | RuntimeErrorKind::InvalidCredential => {
                StorageFailureCode::AuthenticationFailed
            }
            RuntimeErrorKind::HttpStatus | RuntimeErrorKind::Provider => {
                StorageFailureCode::ProviderRejected
            }
            RuntimeErrorKind::Timeout => StorageFailureCode::Timeout,
            RuntimeErrorKind::StreamTooLarge => StorageFailureCode::ResponseTooLarge,
            RuntimeErrorKind::UnexpectedContentType | RuntimeErrorKind::StreamProtocol => {
                StorageFailureCode::ProtocolViolation
            }
            RuntimeErrorKind::InvalidRequest
            | RuntimeErrorKind::InvalidEndpoint
            | RuntimeErrorKind::UnsafeEndpoint
            | RuntimeErrorKind::Cancelled
            | RuntimeErrorKind::ConsumerClosed => StorageFailureCode::Internal,
        };
    }
    match error.code.as_str() {
        "DNS_TIMEOUT" | "DNS_RESOLUTION_FAILED" | "DNS_NO_ADDRESSES" => {
            StorageFailureCode::NetworkUnavailable
        }
        "OVERALL_TIMEOUT"
        | "RESPONSE_HEADER_TIMEOUT"
        | "STREAM_IDLE_TIMEOUT"
        | "STREAM_ACK_TIMEOUT"
        | "STREAM_ACK_DURABILITY_TIMEOUT"
        | "EXACT_TOKEN_COUNT_TIMEOUT" => StorageFailureCode::Timeout,
        "STREAM_VISIBLE_OUTPUT_TOO_LARGE"
        | "STREAM_EVENT_TOO_LARGE"
        | "PROVIDER_RESPONSE_ID_TOO_LARGE" => StorageFailureCode::ResponseTooLarge,
        "INVALID_STREAM_EVENT"
        | "STREAM_INTERNAL_STATE"
        | "STREAM_ALREADY_TERMINAL"
        | "STREAM_SEQUENCE_EXHAUSTED"
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

struct Utf8Fragments<'a> {
    remaining: &'a str,
    max_bytes: usize,
}

impl<'a> Iterator for Utf8Fragments<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        let mut end = self.max_bytes.min(self.remaining.len());
        while end > 0 && !self.remaining.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            end = self
                .remaining
                .char_indices()
                .nth(1)
                .map_or(self.remaining.len(), |(offset, _)| offset);
        }
        let (fragment, remaining) = self.remaining.split_at(end);
        self.remaining = remaining;
        Some(fragment)
    }
}

fn split_utf8(text: &str, max_bytes: usize) -> Utf8Fragments<'_> {
    Utf8Fragments {
        remaining: text,
        max_bytes: max_bytes.max(1),
    }
}

fn schedule_terminal_cleanup(
    request_id: String,
    state: Arc<StreamRequestState>,
    registry: ProviderStreamRegistry,
) {
    tokio::spawn(async move {
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
    use lorepia_storage::{CharacterId, CreateChat};

    fn owner_label(value: &str) -> StreamOwnerLabel {
        StreamOwnerLabel::parse(value).unwrap()
    }

    fn request_state(owner_label: &str) -> Arc<StreamRequestState> {
        Arc::new(StreamRequestState::new(
            self::owner_label(owner_label),
            StreamGeneration::new(),
            "0123456789abcdef".to_owned(),
        ))
    }

    fn started_turn() -> StartedTurn {
        StartedTurn {
            request_state_id: RequestStateId::parse("1".repeat(32)).unwrap(),
            user_message_id: lorepia_storage::MessageId::parse("2".repeat(32)).unwrap(),
            assistant_message_id: lorepia_storage::MessageId::parse("3".repeat(32)).unwrap(),
            user_ordinal: 1,
            assistant_ordinal: 2,
            owner_label: owner_label("main"),
            stream_generation: StreamGeneration::new(),
            last_delivered_seq: 0,
            last_durable_seq: 0,
            last_acked_seq: None,
        }
    }

    fn stored_message(
        ordinal: u64,
        role: StoredMessageRole,
        status: StoredMessageStatus,
        text: impl Into<String>,
    ) -> StoredMessage {
        let at_ms = TimestampMillis::new(i64::try_from(ordinal).unwrap()).unwrap();
        StoredMessage {
            id: lorepia_storage::MessageId::parse(format!("{ordinal:032x}")).unwrap(),
            chat_id: StorageChatId::parse("c".repeat(32)).unwrap(),
            parent_id: (ordinal > 1).then(|| {
                lorepia_storage::MessageId::parse(format!("{:032x}", ordinal - 1)).unwrap()
            }),
            sibling_ord: 1,
            depth: ordinal - 1,
            ordinal,
            role,
            status,
            text: text.into(),
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
            completed_at_ms: (status == StoredMessageStatus::Complete).then_some(at_ms),
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum ModelAction {
        Deliver,
        RejectDelivery,
        AcknowledgeLatest,
        Cancel,
        Fail,
    }

    const MODEL_ACTIONS: [ModelAction; 5] = [
        ModelAction::Deliver,
        ModelAction::RejectDelivery,
        ModelAction::AcknowledgeLatest,
        ModelAction::Cancel,
        ModelAction::Fail,
    ];

    #[derive(Debug)]
    struct ReferenceStream {
        last_sent_seq: u64,
        acknowledged_through: Option<u64>,
        cancel_requested: bool,
        forced_failure: bool,
    }

    impl ReferenceStream {
        fn after_started_and_acked() -> Self {
            Self {
                last_sent_seq: 0,
                acknowledged_through: Some(0),
                cancel_requested: false,
                forced_failure: false,
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

        fn expected_send_error(&self) -> Option<&'static str> {
            if self.forced_failure {
                Some("MODEL_FORCED_FAILURE")
            } else if self.cancel_requested {
                Some("STREAM_CANCELLED")
            } else {
                None
            }
        }
    }

    async fn assert_reduced_model_sequence(sequence: &[ModelAction]) {
        let state = request_state("model-owner");
        state.machine.lock().await.record_ack_progress(0);
        let accepted_channel = Channel::new(|_| Ok(()));
        let rejected_channel = Channel::new(|_| Err(tauri::Error::FailedToReceiveMessage));
        let mut reference = ReferenceStream::after_started_and_acked();

        for action in sequence {
            match action {
                ModelAction::Deliver => {
                    if let Some(expected_error) = reference.expected_send_error() {
                        let error = send_non_terminal(
                            "provider-model",
                            &state,
                            &accepted_channel,
                            |request_id, seq| ProviderChannelEvent::TextDelta {
                                request_id,
                                seq,
                                text: "model-delta".to_owned(),
                            },
                        )
                        .await
                        .unwrap_err();
                        assert_eq!(error.code, expected_error, "sequence: {sequence:?}");
                    } else {
                        assert!(
                            reference.in_flight() < MAX_IN_FLIGHT,
                            "the reduced depth must not block on backpressure: {sequence:?}"
                        );
                        let seq = send_non_terminal(
                            "provider-model",
                            &state,
                            &accepted_channel,
                            |request_id, seq| ProviderChannelEvent::TextDelta {
                                request_id,
                                seq,
                                text: "model-delta".to_owned(),
                            },
                        )
                        .await
                        .unwrap();
                        reference.last_sent_seq += 1;
                        assert_eq!(seq, reference.last_sent_seq, "sequence: {sequence:?}");
                    }
                }
                ModelAction::RejectDelivery => {
                    let before = reference.last_sent_seq;
                    let error = send_non_terminal(
                        "provider-model",
                        &state,
                        &rejected_channel,
                        |request_id, seq| ProviderChannelEvent::TextDelta {
                            request_id,
                            seq,
                            text: "rejected-model-delta".to_owned(),
                        },
                    )
                    .await
                    .unwrap_err();
                    let expected = reference
                        .expected_send_error()
                        .unwrap_or("STREAM_CHANNEL_CLOSED");
                    assert_eq!(error.code, expected, "sequence: {sequence:?}");
                    assert_eq!(reference.last_sent_seq, before);
                }
                ModelAction::AcknowledgeLatest => {
                    if reference.expected_send_error().is_none() {
                        state
                            .machine
                            .lock()
                            .await
                            .record_ack_progress(reference.last_sent_seq);
                        reference.acknowledged_through = Some(reference.last_sent_seq);
                        state.notify.notify_waiters();
                    }
                }
                ModelAction::Cancel => {
                    let expected = !reference.cancel_requested && !reference.forced_failure;
                    assert_eq!(state.request_cancel(), expected, "sequence: {sequence:?}");
                    reference.cancel_requested |= expected;
                }
                ModelAction::Fail => {
                    force_stream_failure(
                        &state,
                        ProviderCommandError::new(
                            "MODEL_FORCED_FAILURE",
                            "deterministic model failure",
                        ),
                    )
                    .await;
                    reference.forced_failure = true;
                }
            }

            let machine = state.machine.lock().await;
            assert_eq!(
                machine.last_sent_seq, reference.last_sent_seq,
                "sequence: {sequence:?}"
            );
            assert_eq!(
                machine.acknowledged_through, reference.acknowledged_through,
                "sequence: {sequence:?}"
            );
            assert_eq!(
                machine.in_flight(),
                reference.in_flight(),
                "sequence: {sequence:?}"
            );
            assert!(
                machine.in_flight() <= MAX_IN_FLIGHT,
                "sequence: {sequence:?}"
            );
            assert_eq!(machine.pending_send_seq, None, "sequence: {sequence:?}");
            assert!(machine.terminal.is_none(), "sequence: {sequence:?}");
            assert!(!machine.terminal_committing, "sequence: {sequence:?}");
            assert_eq!(
                machine.lease_failure.is_some(),
                reference.forced_failure,
                "sequence: {sequence:?}"
            );
            drop(machine);
            assert_eq!(
                state.cancel_requested.load(Ordering::Acquire),
                reference.cancel_requested,
                "sequence: {sequence:?}"
            );
            assert_eq!(
                state.forced_failure.load(Ordering::Acquire),
                reference.forced_failure,
                "sequence: {sequence:?}"
            );
            assert_eq!(
                state.cancellation.is_cancelled(),
                reference.cancel_requested || reference.forced_failure,
                "sequence: {sequence:?}"
            );
        }
    }

    #[tokio::test]
    async fn reduced_deterministic_action_corpus_preserves_stream_invariants() {
        // Exhaust every action sequence through depth four (781 total cases).
        // The larger 100k+ corpus belongs in scheduled stress runs; this
        // reduced corpus remains deterministic and fast enough for every PR.
        for depth in 0..=4_u32 {
            for mut ordinal in 0..MODEL_ACTIONS.len().pow(depth) {
                let mut sequence = Vec::with_capacity(depth as usize);
                for _ in 0..depth {
                    sequence.push(MODEL_ACTIONS[ordinal % MODEL_ACTIONS.len()]);
                    ordinal /= MODEL_ACTIONS.len();
                }
                assert_reduced_model_sequence(&sequence).await;
            }
        }
    }

    #[test]
    fn worst_case_delta_fragment_stays_on_direct_channel_path() {
        let text = "\u{0001}".repeat(MAX_DELTA_FRAGMENT_BYTES);
        for fragment in split_utf8(&text, MAX_DELTA_FRAGMENT_BYTES) {
            let event = ProviderChannelEvent::TextDelta {
                request_id: format!("provider-{}", "f".repeat(32)),
                seq: MAX_WIRE_SEQUENCE,
                text: fragment.to_owned(),
            };
            assert!(serde_json::to_vec(&event).unwrap().len() <= DIRECT_CHANNEL_BUDGET_BYTES);
        }
    }

    #[test]
    fn utf8_splitting_preserves_text_exactly() {
        let text = format!("{}{}{}", "a".repeat(511), "🦀", "나".repeat(400));
        let fragments = split_utf8(&text, MAX_DELTA_FRAGMENT_BYTES).collect::<Vec<_>>();
        assert_eq!(fragments.concat(), text);
        assert!(
            fragments
                .iter()
                .all(|part| part.len() <= MAX_DELTA_FRAGMENT_BYTES)
        );
    }

    #[tokio::test]
    async fn empty_runtime_text_events_are_rejected_without_delivery_or_state_change() {
        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));
        let state = request_state("main");
        let mut persistence =
            StreamPersistence::new(started_turn(), TimestampMillis::now().unwrap());
        let channel_called = Arc::new(AtomicBool::new(false));
        let channel_called_for_send = Arc::clone(&channel_called);
        let channel = Channel::new(move |_| {
            channel_called_for_send.store(true, Ordering::Release);
            Ok(())
        });

        for kind in [DeltaKind::Text, DeltaKind::Reasoning, DeltaKind::Refusal] {
            let error = forward_text_fragments(
                "provider-test",
                &state,
                &channel,
                &mut persistence,
                &storage,
                String::new(),
                kind,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "INVALID_STREAM_EVENT");
        }

        assert!(!channel_called.load(Ordering::Acquire));
        assert_eq!(persistence.through_seq, 0);
        assert_eq!(persistence.last_delivered_seq, 0);
        assert!(persistence.appended_text.is_empty());
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, 0);
        assert_eq!(machine.pending_send_seq, None);
    }

    #[test]
    fn oversized_direct_event_is_rejected_before_channel_callback() {
        let channel_called = Arc::new(AtomicBool::new(false));
        let channel_called_for_send = Arc::clone(&channel_called);
        let channel = Channel::new(move |_| {
            channel_called_for_send.store(true, Ordering::Release);
            Ok(())
        });
        let event = ProviderChannelEvent::TextDelta {
            request_id: format!("provider-{}", "f".repeat(32)),
            seq: 1,
            text: "x".repeat(DIRECT_CHANNEL_BUDGET_BYTES),
        };
        assert!(serde_json::to_vec(&event).unwrap().len() > DIRECT_CHANNEL_BUDGET_BYTES);

        let error = send_direct(&channel, event).unwrap_err();
        assert_eq!(error.code, "STREAM_EVENT_TOO_LARGE");
        assert!(!channel_called.load(Ordering::Acquire));
    }

    #[test]
    fn snapshot_schema_exposes_control_plane_state_only() {
        let snapshot = ProviderStreamSnapshot {
            request_id: "provider-test".to_owned(),
            last_sent_seq: 2,
            acknowledged_through: Some(1),
            in_flight: 1,
            cancel_requested: false,
            terminal: Some(TerminalReceipt::Cancelled { seq: 2 }),
        };
        let value = serde_json::to_value(snapshot).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 6);
        for required in [
            "requestId",
            "lastSentSeq",
            "acknowledgedThrough",
            "inFlight",
            "cancelRequested",
            "terminal",
        ] {
            assert!(object.contains_key(required));
        }
        for forbidden in ["text", "content", "partialText", "rawText", "snapshotText"] {
            assert!(!object.contains_key(forbidden));
        }
        ensure_direct_serializable(&value).unwrap();
    }

    #[test]
    fn control_tokens_are_bound_to_one_request() {
        let state = StreamRequestState::new(
            owner_label("main"),
            StreamGeneration::new(),
            "0123456789abcdef".to_owned(),
        );
        assert!(state.authenticates("0123456789abcdef"));
        assert!(!state.authenticates("0123456789abcdee"));
        assert!(!state.authenticates("short"));
    }

    #[test]
    fn storage_identity_rejects_stale_owner_generation_and_bind_replay() {
        let state = request_state("main");
        assert!(state.request_state_id().is_err());
        let matching = StartedTurn {
            request_state_id: RequestStateId::new(),
            user_message_id: lorepia_storage::MessageId::new(),
            assistant_message_id: lorepia_storage::MessageId::new(),
            user_ordinal: 1,
            assistant_ordinal: 2,
            owner_label: state.owner_label.clone(),
            stream_generation: state.stream_generation.clone(),
            last_delivered_seq: 0,
            last_durable_seq: 0,
            last_acked_seq: None,
        };

        let mut stale_owner = matching.clone();
        stale_owner.owner_label = owner_label("other");
        assert_eq!(
            state.bind_started_turn(&stale_owner).unwrap_err().code,
            "STREAM_STORAGE_IDENTITY_MISMATCH"
        );

        let mut stale_generation = matching.clone();
        stale_generation.stream_generation = StreamGeneration::new();
        assert_eq!(
            state.bind_started_turn(&stale_generation).unwrap_err().code,
            "STREAM_STORAGE_IDENTITY_MISMATCH"
        );

        state.bind_started_turn(&matching).unwrap();
        assert_eq!(state.request_state_id().unwrap(), matching.request_state_id);
        assert_eq!(
            state.bind_started_turn(&matching).unwrap_err().code,
            "STREAM_STORAGE_IDENTITY_MISMATCH"
        );
    }

    #[tokio::test]
    async fn terminal_ack_is_durable_and_stale_identity_or_replay_cannot_advance_it() {
        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));
        let owner = owner_label("main");
        let generation = StreamGeneration::new();
        let owner_for_turn = owner.clone();
        let generation_for_turn = generation.clone();
        let started = storage
            .run(move |store| {
                let at_ms = TimestampMillis::now()?;
                let chat = store.create_chat(CreateChat {
                    character_id: CharacterId::parse("character-test")?,
                    title: "terminal ACK test".to_owned(),
                    at_ms,
                })?;
                store.begin_turn(BeginTurn {
                    chat_id: chat.id,
                    selection: StorageProviderSelection {
                        provider_id: StorageProviderId::OpenAi,
                        model_id: StorageModelId::parse("gpt-test")?,
                    },
                    owner_label: owner_for_turn,
                    stream_generation: generation_for_turn,
                    user_text: "hello".to_owned(),
                    started_at_ms: at_ms,
                })
            })
            .await
            .unwrap();

        let state = Arc::new(StreamRequestState::new(
            owner.clone(),
            generation.clone(),
            "terminal-token".to_owned(),
        ));
        state.bind_started_turn(&started).unwrap();

        let started_for_finish = started.clone();
        storage
            .run(move |store| {
                let at_ms = TimestampMillis::now()?;
                store.record_response_delivery(DeliveryCheckpoint {
                    request_state_id: started_for_finish.request_state_id.clone(),
                    owner_label: started_for_finish.owner_label.clone(),
                    stream_generation: started_for_finish.stream_generation.clone(),
                    expected_last_delivered_seq: 0,
                    through_seq: 1,
                    at_ms,
                })?;
                store.finish_turn(TerminalCheckpoint {
                    checkpoint: ResponseCheckpoint {
                        request_state_id: started_for_finish.request_state_id.clone(),
                        owner_label: started_for_finish.owner_label.clone(),
                        stream_generation: started_for_finish.stream_generation.clone(),
                        expected_last_durable_seq: 0,
                        through_seq: 1,
                        appended_text: "answer".to_owned(),
                        provider_response_id: None,
                        usage: None,
                        at_ms,
                    },
                    outcome: TerminalOutcome::Completed,
                })
            })
            .await
            .unwrap();
        {
            let mut machine = state.machine.lock().await;
            machine.last_sent_seq = 1;
            machine.last_durable_seq = 1;
            machine.terminal = Some(TerminalReceipt::Completed {
                seq: 1,
                reason: None,
                usage: None,
            });
            machine.ack_deadline = None;
        }

        for stale in [
            Arc::new(StreamRequestState::new(
                owner_label("other"),
                generation.clone(),
                "stale-owner".to_owned(),
            )),
            Arc::new(StreamRequestState::new(
                owner.clone(),
                StreamGeneration::new(),
                "stale-generation".to_owned(),
            )),
        ] {
            *stale.request_state_id.lock().unwrap() = Some(started.request_state_id.clone());
            let error = persist_cumulative_ack(&storage, &stale, 1)
                .await
                .unwrap_err();
            assert_eq!(error.code, "STORAGE_WRITE_FAILED");
            assert_eq!(stale.machine.lock().await.persisted_acked_through, None);
        }

        persist_cumulative_ack(&storage, &state, 1).await.unwrap();
        assert_eq!(state.machine.lock().await.persisted_acked_through, Some(1));

        let replay = persist_cumulative_ack(&storage, &state, 1)
            .await
            .unwrap_err();
        assert_eq!(replay.code, "STORAGE_INPUT_INVALID");
        assert_eq!(state.machine.lock().await.persisted_acked_through, Some(1));

        let request_state_id = started.request_state_id.clone();
        let persisted = storage
            .run(move |store| store.get_request_state(&request_state_id))
            .await
            .unwrap();
        assert_eq!(persisted.status, StorageRequestStatus::Completed);
        assert_eq!(persisted.last_delivered_seq, 1);
        assert_eq!(persisted.last_durable_seq, 1);
        assert_eq!(persisted.last_acked_seq, Some(1));
    }

    #[tokio::test]
    async fn rejected_start_recovery_resumes_after_delivery_only_crash_window() {
        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));
        let owner = owner_label("main");
        let generation = StreamGeneration::new();
        let owner_for_turn = owner.clone();
        let generation_for_turn = generation.clone();
        let started = storage
            .run(move |store| {
                let at_ms = TimestampMillis::now()?;
                let chat = store.create_chat(CreateChat {
                    character_id: CharacterId::parse("rejected-start-test")?,
                    title: "rejected start".to_owned(),
                    at_ms,
                })?;
                store.begin_turn(BeginTurn {
                    chat_id: chat.id,
                    selection: StorageProviderSelection {
                        provider_id: StorageProviderId::OpenAi,
                        model_id: StorageModelId::parse("gpt-test")?,
                    },
                    owner_label: owner_for_turn,
                    stream_generation: generation_for_turn,
                    user_text: "persist me".to_owned(),
                    started_at_ms: at_ms,
                })
            })
            .await
            .unwrap();

        let delivery_only = started.clone();
        storage
            .run(move |store| {
                store.record_response_delivery(DeliveryCheckpoint {
                    request_state_id: delivery_only.request_state_id,
                    owner_label: delivery_only.owner_label,
                    stream_generation: delivery_only.stream_generation,
                    expected_last_delivered_seq: 0,
                    through_seq: 1,
                    at_ms: TimestampMillis::now()?,
                })
            })
            .await
            .unwrap();

        let at_ms = TimestampMillis::now().unwrap();
        persist_rejected_start(&storage, &started, at_ms)
            .await
            .unwrap();
        // A repeated recovery after the terminal commit is a no-op rather than
        // a second terminal transition.
        persist_rejected_start(&storage, &started, at_ms)
            .await
            .unwrap();

        let request_state_id = started.request_state_id.clone();
        let persisted = storage
            .run(move |store| store.get_request_state(&request_state_id))
            .await
            .unwrap();
        assert_eq!(persisted.status, StorageRequestStatus::Failed);
        assert_eq!(persisted.last_delivered_seq, 1);
        assert_eq!(persisted.last_durable_seq, 1);
    }

    #[test]
    fn terminal_receipts_also_fit_the_direct_budget() {
        let snapshot = ProviderStreamSnapshot {
            request_id: format!("provider-{}", "f".repeat(32)),
            last_sent_seq: MAX_WIRE_SEQUENCE,
            acknowledged_through: Some(MAX_WIRE_SEQUENCE),
            in_flight: 0,
            cancel_requested: false,
            terminal: Some(TerminalReceipt::Failed {
                seq: MAX_WIRE_SEQUENCE,
                error: ProviderCommandError::new("X".repeat(64), "Y".repeat(512)),
            }),
        };
        ensure_direct_serializable(&snapshot).unwrap();
    }

    #[test]
    fn registry_enforces_count_and_global_reserved_byte_budget() {
        let registry = ProviderStreamRegistry::default();
        let mut inserted = Vec::new();
        for _ in 0..MAX_ACTIVE_STREAMS {
            inserted.push(registry.insert_new(owner_label("main")).unwrap());
        }

        let inner = registry.inner.lock().unwrap();
        assert_eq!(inner.requests.len(), MAX_ACTIVE_STREAMS);
        assert_eq!(
            inner.reserved_in_flight_bytes,
            GLOBAL_IN_FLIGHT_RESERVATION_BYTES
        );
        drop(inner);

        let rejected = registry.insert_new(owner_label("main")).err().unwrap();
        assert_eq!(rejected.code, "TOO_MANY_ACTIVE_STREAMS");

        let (removed_id, removed_state) = inserted.pop().unwrap();
        registry.remove_if_same(&removed_id, &removed_state);
        assert_eq!(
            registry.inner.lock().unwrap().reserved_in_flight_bytes,
            GLOBAL_IN_FLIGHT_RESERVATION_BYTES - PER_STREAM_IN_FLIGHT_RESERVATION_BYTES
        );

        registry.insert_new(owner_label("main")).unwrap();
        let inner = registry.inner.lock().unwrap();
        assert_eq!(inner.requests.len(), MAX_ACTIVE_STREAMS);
        assert_eq!(
            inner.reserved_in_flight_bytes,
            GLOBAL_IN_FLIGHT_RESERVATION_BYTES
        );
    }

    #[test]
    fn reserved_byte_budget_is_an_independent_admission_gate() {
        let registry = ProviderStreamRegistry::default();
        registry.inner.lock().unwrap().reserved_in_flight_bytes =
            GLOBAL_IN_FLIGHT_RESERVATION_BYTES;

        let rejected = registry.insert_new(owner_label("main")).err().unwrap();
        assert_eq!(rejected.code, "STREAM_BYTE_BUDGET_EXHAUSTED");
        assert!(registry.inner.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn repeated_registry_cleanup_has_no_count_or_byte_reservation_drift() {
        let registry = ProviderStreamRegistry::default();
        for iteration in 0..1_024 {
            let (request_id, state) = registry.insert_new(owner_label("main")).unwrap();
            if iteration % 17 == 0 {
                let stale_state = request_state("main");
                registry.remove_if_same(&request_id, &stale_state);
                let inner = registry.inner.lock().unwrap();
                assert_eq!(inner.requests.len(), 1);
                assert_eq!(
                    inner.reserved_in_flight_bytes,
                    PER_STREAM_IN_FLIGHT_RESERVATION_BYTES
                );
            }
            registry.remove_if_same(&request_id, &state);
            let inner = registry.inner.lock().unwrap();
            assert!(inner.requests.is_empty());
            assert_eq!(inner.reserved_in_flight_bytes, 0);
        }
    }

    #[test]
    fn owner_cleanup_cancels_only_requests_bound_to_destroyed_window() {
        let registry = ProviderStreamRegistry::default();
        let (_, main_one) = registry.insert_new(owner_label("main")).unwrap();
        let (_, main_two) = registry.insert_new(owner_label("main")).unwrap();
        let (_, settings) = registry.insert_new(owner_label("settings")).unwrap();

        assert_eq!(registry.cancel_owner("main"), 2);
        assert!(main_one.cancellation.is_cancelled());
        assert!(main_two.cancellation.is_cancelled());
        assert!(!settings.cancellation.is_cancelled());
        assert_eq!(registry.cancel_owner("main"), 0);
    }

    #[tokio::test]
    async fn owner_reset_cancels_waits_for_terminal_and_releases_admission() {
        let registry = ProviderStreamRegistry::default();
        let (_, state) = registry.insert_new(owner_label("main")).unwrap();
        let state_for_terminal = Arc::clone(&state);
        tokio::spawn(async move {
            state_for_terminal.cancellation.cancelled().await;
            {
                let mut machine = state_for_terminal.machine.lock().await;
                machine.last_sent_seq = 1;
                machine.terminal = Some(TerminalReceipt::Cancelled { seq: 1 });
                machine.ack_deadline = None;
            }
            state_for_terminal.notify.notify_waiters();
        });

        let response = reset_owner_streams("main", &registry).await.unwrap();
        assert_eq!(response.cancelled, 1);
        assert_eq!(response.terminalized, 1);
        assert!(registry.inner.lock().unwrap().requests.is_empty());
        assert_eq!(registry.inner.lock().unwrap().reserved_in_flight_bytes, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn owner_reset_fails_closed_when_terminal_storage_never_finishes() {
        let registry = ProviderStreamRegistry::default();
        let (_, state) = registry.insert_new(owner_label("main")).unwrap();
        let reset_registry = registry.clone();
        let reset = tokio::spawn(async move { reset_owner_streams("main", &reset_registry).await });
        tokio::task::yield_now().await;
        assert!(state.cancellation.is_cancelled());

        tokio::time::advance(OWNER_RESET_TIMEOUT).await;
        tokio::task::yield_now().await;
        let error = reset.await.unwrap().unwrap_err();
        assert_eq!(error.code, "STREAM_OWNER_RESET_TIMEOUT");
        assert_eq!(registry.inner.lock().unwrap().requests.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn never_ack_times_out_even_before_the_count_window_is_full() {
        let state = request_state("main");
        let watchdog = tokio::spawn(run_ack_watchdog(Arc::clone(&state)));
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(29)).await;
        tokio::task::yield_now().await;
        assert!(!state.cancellation.is_cancelled());

        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        watchdog.await.unwrap();
        let machine = state.machine.lock().await;
        assert!(state.cancellation.is_cancelled());
        assert_eq!(
            machine
                .lease_failure
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("STREAM_ACK_TIMEOUT")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn ack_progress_renews_lease_but_duplicate_ack_does_not() {
        let state = request_state("main");
        state.machine.lock().await.last_sent_seq = 1;
        let watchdog = tokio::spawn(run_ack_watchdog(Arc::clone(&state)));
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(20)).await;
        state.machine.lock().await.record_ack_progress(0);
        state.notify.notify_waiters();
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(20)).await;
        state.machine.lock().await.record_ack_progress(0);
        state.notify.notify_waiters();
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(9)).await;
        tokio::task::yield_now().await;
        assert!(!state.cancellation.is_cancelled());

        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        watchdog.await.unwrap();
        assert!(state.cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn channel_send_runs_without_machine_lock_and_failure_rolls_back() {
        let state = request_state("main");
        let state_for_send = Arc::clone(&state);
        let lock_was_free = Arc::new(AtomicBool::new(false));
        let lock_was_free_for_send = Arc::clone(&lock_was_free);
        let channel = Channel::new(move |_| {
            lock_was_free_for_send
                .store(state_for_send.machine.try_lock().is_ok(), Ordering::Release);
            Err(tauri::Error::FailedToReceiveMessage)
        });
        let initial_deadline = state.machine.lock().await.ack_deadline;

        let result = send_non_terminal("provider-test", &state, &channel, |request_id, seq| {
            ProviderChannelEvent::TextDelta {
                request_id,
                seq,
                text: "delta".to_owned(),
            }
        })
        .await;

        assert_eq!(result.unwrap_err().code, "STREAM_CHANNEL_CLOSED");
        assert!(lock_was_free.load(Ordering::Acquire));
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, 0);
        assert_eq!(machine.pending_send_seq, None);
        assert_eq!(machine.ack_deadline, initial_deadline);
    }

    #[tokio::test]
    async fn ack_lease_failure_blocks_send_before_cancellation_token_handoff() {
        let state = request_state("main");
        {
            let mut machine = state.machine.lock().await;
            machine.lease_failure = Some(ProviderCommandError::new(
                "STREAM_ACK_TIMEOUT",
                "provider stream acknowledgements timed out",
            ));
            state.forced_failure.store(true, Ordering::Release);
        }
        assert!(!state.cancellation.is_cancelled());

        let channel_was_called = Arc::new(AtomicBool::new(false));
        let channel_was_called_for_send = Arc::clone(&channel_was_called);
        let channel = Channel::new(move |_| {
            channel_was_called_for_send.store(true, Ordering::Release);
            Ok(())
        });

        let error = send_non_terminal("provider-test", &state, &channel, |request_id, seq| {
            ProviderChannelEvent::TextDelta {
                request_id,
                seq,
                text: "must-not-send".to_owned(),
            }
        })
        .await
        .unwrap_err();

        assert_eq!(error.code, "STREAM_ACK_TIMEOUT");
        assert!(!channel_was_called.load(Ordering::Acquire));
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, 0);
        assert_eq!(machine.pending_send_seq, None);
    }

    #[tokio::test]
    async fn successful_send_commits_state_after_unlocked_channel_delivery() {
        let state = request_state("main");
        state.machine.lock().await.record_ack_progress(0);
        let state_for_send = Arc::clone(&state);
        let lock_was_free = Arc::new(AtomicBool::new(false));
        let lock_was_free_for_send = Arc::clone(&lock_was_free);
        let channel = Channel::new(move |_| {
            lock_was_free_for_send
                .store(state_for_send.machine.try_lock().is_ok(), Ordering::Release);
            Ok(())
        });

        let seq = send_non_terminal("provider-test", &state, &channel, |request_id, seq| {
            ProviderChannelEvent::TextDelta {
                request_id,
                seq,
                text: "delta".to_owned(),
            }
        })
        .await
        .unwrap();

        assert_eq!(seq, 1);
        assert!(lock_was_free.load(Ordering::Acquire));
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, 1);
        assert_eq!(machine.pending_send_seq, None);
        assert!(machine.ack_deadline.is_some());
    }

    #[tokio::test]
    async fn concurrent_senders_keep_unique_sequence_and_bounded_window() {
        use tauri::ipc::InvokeResponseBody;
        use tokio::sync::Barrier;

        let state = request_state("main");
        state.machine.lock().await.record_ack_progress(0);
        let (delivered_tx, mut delivered_rx) = mpsc::unbounded_channel();
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            let event: serde_json::Value = serde_json::from_str(&json)?;
            delivered_tx
                .send(event["seq"].as_u64().unwrap())
                .map_err(|_| tauri::Error::FailedToReceiveMessage)
        });
        let sender_count = usize::try_from(MAX_IN_FLIGHT).unwrap() + 1;
        let start = Arc::new(Barrier::new(sender_count + 1));
        let mut senders = Vec::with_capacity(sender_count);
        for _ in 0..sender_count {
            let state_for_send = Arc::clone(&state);
            let channel_for_send = channel.clone();
            let start_for_send = Arc::clone(&start);
            senders.push(tokio::spawn(async move {
                start_for_send.wait().await;
                send_non_terminal(
                    "provider-concurrent",
                    &state_for_send,
                    &channel_for_send,
                    |request_id, seq| ProviderChannelEvent::TextDelta {
                        request_id,
                        seq,
                        text: "delta".to_owned(),
                    },
                )
                .await
            }));
        }
        start.wait().await;

        let mut delivered = Vec::with_capacity(sender_count);
        for _ in 0..MAX_IN_FLIGHT {
            delivered.push(
                tokio::time::timeout(Duration::from_secs(1), delivered_rx.recv())
                    .await
                    .expect("the bounded window stalled")
                    .expect("the channel callback closed"),
            );
        }
        assert!(matches!(
            delivered_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        {
            let machine = state.machine.lock().await;
            assert_eq!(machine.in_flight(), MAX_IN_FLIGHT);
            assert_eq!(machine.pending_send_seq, None);
        }

        state.machine.lock().await.record_ack_progress(1);
        state.notify.notify_waiters();
        delivered.push(
            tokio::time::timeout(Duration::from_secs(1), delivered_rx.recv())
                .await
                .expect("the sender did not resume after cumulative ACK")
                .expect("the channel callback closed"),
        );

        for sender in senders {
            tokio::time::timeout(Duration::from_secs(1), sender)
                .await
                .expect("concurrent sender did not finish")
                .unwrap()
                .unwrap();
        }
        delivered.sort_unstable();
        assert_eq!(delivered, vec![1, 2, 3, 4, 5]);
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, 5);
        assert_eq!(machine.acknowledged_through, Some(1));
        assert_eq!(machine.in_flight(), MAX_IN_FLIGHT);
        assert_eq!(machine.pending_send_seq, None);
    }

    #[test]
    fn wire_sequence_stops_at_javascript_safe_integer_without_wrapping() {
        assert_eq!(
            next_wire_sequence(MAX_WIRE_SEQUENCE - 1, MAX_WIRE_SEQUENCE).unwrap(),
            MAX_WIRE_SEQUENCE
        );
        assert_eq!(
            next_wire_sequence(MAX_NON_TERMINAL_SEQUENCE - 1, MAX_NON_TERMINAL_SEQUENCE).unwrap(),
            MAX_NON_TERMINAL_SEQUENCE
        );
        assert_eq!(
            next_wire_sequence(MAX_NON_TERMINAL_SEQUENCE, MAX_WIRE_SEQUENCE).unwrap(),
            MAX_WIRE_SEQUENCE
        );
        for exhausted in [MAX_WIRE_SEQUENCE, u64::MAX] {
            let error = next_wire_sequence(exhausted, MAX_WIRE_SEQUENCE).unwrap_err();
            assert_eq!(error.code, "STREAM_SEQUENCE_EXHAUSTED");
        }
        assert_eq!(
            next_wire_sequence(MAX_NON_TERMINAL_SEQUENCE, MAX_NON_TERMINAL_SEQUENCE)
                .unwrap_err()
                .code,
            "STREAM_SEQUENCE_EXHAUSTED"
        );
    }

    #[tokio::test]
    async fn exhausted_non_terminal_send_does_not_call_factory_or_channel_or_mutate_state() {
        let state = request_state("main");
        {
            let mut machine = state.machine.lock().await;
            machine.last_sent_seq = MAX_NON_TERMINAL_SEQUENCE;
            machine.acknowledged_through = Some(MAX_NON_TERMINAL_SEQUENCE);
            machine.ack_deadline = None;
        }

        let factory_called = Arc::new(AtomicBool::new(false));
        let channel_called = Arc::new(AtomicBool::new(false));
        let untouched_text = Arc::new(Mutex::new("unchanged".to_owned()));
        let channel_called_for_send = Arc::clone(&channel_called);
        let channel = Channel::new(move |_| {
            channel_called_for_send.store(true, Ordering::Release);
            Ok(())
        });
        let factory_called_for_send = Arc::clone(&factory_called);
        let text_for_send = Arc::clone(&untouched_text);

        let error = send_non_terminal("provider-test", &state, &channel, move |request_id, seq| {
            factory_called_for_send.store(true, Ordering::Release);
            text_for_send.lock().unwrap().push_str("-mutated");
            ProviderChannelEvent::TextDelta {
                request_id,
                seq,
                text: "must-not-send".to_owned(),
            }
        })
        .await
        .unwrap_err();

        assert_eq!(error.code, "STREAM_SEQUENCE_EXHAUSTED");
        assert!(!factory_called.load(Ordering::Acquire));
        assert!(!channel_called.load(Ordering::Acquire));
        assert_eq!(untouched_text.lock().unwrap().as_str(), "unchanged");
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, MAX_NON_TERMINAL_SEQUENCE);
        assert_eq!(
            machine.acknowledged_through,
            Some(MAX_NON_TERMINAL_SEQUENCE)
        );
        assert_eq!(machine.pending_send_seq, None);
        assert!(machine.terminal.is_none());
        assert!(!machine.terminal_committing);
        assert!(machine.lease_failure.is_none());
        assert!(machine.ack_deadline.is_none());
    }

    #[tokio::test]
    async fn last_non_terminal_sequence_is_emitted_once_and_never_duplicated() {
        use tauri::ipc::InvokeResponseBody;

        let state = request_state("main");
        {
            let mut machine = state.machine.lock().await;
            machine.last_sent_seq = MAX_NON_TERMINAL_SEQUENCE - 1;
            machine.acknowledged_through = Some(MAX_NON_TERMINAL_SEQUENCE - 1);
            machine.ack_deadline = None;
        }
        let received = Arc::new(Mutex::new(Vec::<u64>::new()));
        let received_for_send = Arc::clone(&received);
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            let event: serde_json::Value = serde_json::from_str(&json)?;
            received_for_send
                .lock()
                .unwrap()
                .push(event["seq"].as_u64().unwrap());
            Ok(())
        });

        let seq = send_non_terminal("provider-test", &state, &channel, |request_id, seq| {
            ProviderChannelEvent::TextDelta {
                request_id,
                seq,
                text: "last".to_owned(),
            }
        })
        .await
        .unwrap();
        assert_eq!(seq, MAX_NON_TERMINAL_SEQUENCE);

        let duplicate_factory_called = Arc::new(AtomicBool::new(false));
        let duplicate_factory_called_for_send = Arc::clone(&duplicate_factory_called);
        let error = send_non_terminal("provider-test", &state, &channel, move |request_id, seq| {
            duplicate_factory_called_for_send.store(true, Ordering::Release);
            ProviderChannelEvent::TextDelta {
                request_id,
                seq,
                text: "duplicate".to_owned(),
            }
        })
        .await
        .unwrap_err();

        assert_eq!(error.code, "STREAM_SEQUENCE_EXHAUSTED");
        assert!(!duplicate_factory_called.load(Ordering::Acquire));
        assert_eq!(*received.lock().unwrap(), vec![MAX_NON_TERMINAL_SEQUENCE]);
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, MAX_NON_TERMINAL_SEQUENCE);
        assert_eq!(machine.pending_send_seq, None);
    }

    #[tokio::test]
    async fn terminal_uses_the_reserved_javascript_safe_sequence_after_the_last_delta() {
        use tauri::ipc::InvokeResponseBody;

        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));
        let owner = owner_label("main");
        let generation = StreamGeneration::new();
        let owner_for_turn = owner.clone();
        let generation_for_turn = generation.clone();
        let started = storage
            .run(move |store| {
                let at_ms = TimestampMillis::now()?;
                let chat = store.create_chat(CreateChat {
                    character_id: CharacterId::parse("terminal-sequence-test")?,
                    title: "terminal sequence boundary".to_owned(),
                    at_ms,
                })?;
                store.begin_turn(BeginTurn {
                    chat_id: chat.id,
                    selection: StorageProviderSelection {
                        provider_id: StorageProviderId::OpenAi,
                        model_id: StorageModelId::parse("gpt-test")?,
                    },
                    owner_label: owner_for_turn,
                    stream_generation: generation_for_turn,
                    user_text: "hello".to_owned(),
                    started_at_ms: at_ms,
                })
            })
            .await
            .unwrap();

        // Put the durable test fixture immediately before the reserved terminal
        // slot. The production store enforces the same JavaScript-safe upper
        // bound, so this exercises send_terminal rather than only its helper.
        let connection =
            rusqlite::Connection::open(directory.path().join("lorepia.sqlite3")).unwrap();
        assert_eq!(
            connection
                .execute(
                    "UPDATE request_state
                     SET last_delivered_seq = ?2,
                         last_durable_seq = ?2,
                         last_acked_seq = ?2
                     WHERE id = ?1 AND status = 'running'",
                    rusqlite::params![
                        started.request_state_id.as_str(),
                        i64::try_from(MAX_NON_TERMINAL_SEQUENCE).unwrap()
                    ],
                )
                .unwrap(),
            1
        );
        drop(connection);

        let state = Arc::new(StreamRequestState::new(
            owner,
            generation,
            "terminal-sequence-token".to_owned(),
        ));
        state.bind_started_turn(&started).unwrap();
        {
            let mut machine = state.machine.lock().await;
            machine.last_sent_seq = MAX_NON_TERMINAL_SEQUENCE;
            machine.acknowledged_through = Some(MAX_NON_TERMINAL_SEQUENCE);
            machine.persisted_acked_through = Some(MAX_NON_TERMINAL_SEQUENCE);
            machine.ack_deadline = None;
        }

        let mut persistence_started = started.clone();
        persistence_started.last_delivered_seq = MAX_NON_TERMINAL_SEQUENCE;
        persistence_started.last_durable_seq = MAX_NON_TERMINAL_SEQUENCE;
        persistence_started.last_acked_seq = Some(MAX_NON_TERMINAL_SEQUENCE);
        let mut persistence =
            StreamPersistence::new(persistence_started, TimestampMillis::now().unwrap());
        let received = Arc::new(Mutex::new(Vec::<u64>::new()));
        let received_for_send = Arc::clone(&received);
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            let event: serde_json::Value = serde_json::from_str(&json)?;
            received_for_send
                .lock()
                .unwrap()
                .push(event["seq"].as_u64().unwrap());
            Ok(())
        });

        send_terminal(
            "provider-test",
            &state,
            &channel,
            TerminalKind::Cancelled,
            &mut persistence,
            &storage,
        )
        .await
        .unwrap();

        assert_eq!(*received.lock().unwrap(), vec![MAX_WIRE_SEQUENCE]);
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, MAX_WIRE_SEQUENCE);
        assert!(matches!(
            machine.terminal,
            Some(TerminalReceipt::Cancelled {
                seq: MAX_WIRE_SEQUENCE
            })
        ));
        drop(machine);
        let request_state_id = started.request_state_id;
        let persisted = storage
            .run(move |store| store.get_request_state(&request_state_id))
            .await
            .unwrap();
        assert_eq!(persisted.status, StorageRequestStatus::Cancelled);
        assert_eq!(persisted.last_delivered_seq, MAX_WIRE_SEQUENCE);
        assert_eq!(persisted.last_durable_seq, MAX_WIRE_SEQUENCE);
    }

    #[tokio::test]
    async fn exhausted_terminal_send_does_not_call_channel_or_mutate_state_or_text() {
        let state = request_state("main");
        {
            let mut machine = state.machine.lock().await;
            machine.last_sent_seq = MAX_WIRE_SEQUENCE;
            machine.acknowledged_through = Some(MAX_WIRE_SEQUENCE);
            machine.ack_deadline = None;
        }
        let started = started_turn();
        let started_at_ms = TimestampMillis::now().unwrap();
        let mut persistence = StreamPersistence::new(started, started_at_ms);
        persistence.appended_text = "unchanged".to_owned();
        persistence.visible_text_bytes = persistence.appended_text.len();
        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));
        let channel_called = Arc::new(AtomicBool::new(false));
        let channel_called_for_send = Arc::clone(&channel_called);
        let channel = Channel::new(move |_| {
            channel_called_for_send.store(true, Ordering::Release);
            Ok(())
        });

        let error = send_terminal(
            "provider-test",
            &state,
            &channel,
            TerminalKind::Cancelled,
            &mut persistence,
            &storage,
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, "STREAM_SEQUENCE_EXHAUSTED");
        assert!(!channel_called.load(Ordering::Acquire));
        assert_eq!(persistence.appended_text, "unchanged");
        assert_eq!(persistence.visible_text_bytes, "unchanged".len());
        assert_eq!(persistence.through_seq, 0);
        assert_eq!(persistence.last_delivered_seq, 0);
        assert_eq!(persistence.last_durable_seq, 0);
        let machine = state.machine.lock().await;
        assert_eq!(machine.last_sent_seq, MAX_WIRE_SEQUENCE);
        assert_eq!(machine.acknowledged_through, Some(MAX_WIRE_SEQUENCE));
        assert_eq!(machine.pending_send_seq, None);
        assert!(machine.terminal.is_none());
        assert!(!machine.terminal_committing);
        assert!(machine.ack_deadline.is_none());
    }

    #[tokio::test]
    async fn zero_latency_ack_waits_for_unlocked_send_commit() {
        let state = request_state("main");
        state.machine.lock().await.pending_send_seq = Some(1);

        let state_for_ack = Arc::clone(&state);
        let acknowledgement = tokio::spawn(async move {
            wait_for_send_commit(&state_for_ack, 1).await.unwrap();
            state_for_ack.machine.lock().await.last_sent_seq
        });
        tokio::task::yield_now().await;
        assert!(!acknowledgement.is_finished());

        {
            let mut machine = state.machine.lock().await;
            machine.pending_send_seq = None;
            machine.last_sent_seq = 1;
        }
        state.notify.notify_waiters();

        assert_eq!(acknowledgement.await.unwrap(), 1);
    }

    #[tokio::test]
    async fn zero_latency_snapshot_waits_for_terminal_send_commit() {
        let state = request_state("main");
        {
            let mut machine = state.machine.lock().await;
            machine.pending_send_seq = Some(1);
            machine.terminal_committing = true;
        }

        let state_for_snapshot = Arc::clone(&state);
        let snapshot = tokio::spawn(async move {
            wait_for_pending_send_resolution(&state_for_snapshot)
                .await
                .unwrap();
            let machine = state_for_snapshot.machine.lock().await;
            (
                machine.last_sent_seq,
                matches!(
                    machine.terminal,
                    Some(TerminalReceipt::Cancelled { seq: 1 })
                ),
            )
        });
        tokio::task::yield_now().await;
        assert!(!snapshot.is_finished());

        {
            let mut machine = state.machine.lock().await;
            machine.pending_send_seq = None;
            machine.last_sent_seq = 1;
            machine.terminal = Some(TerminalReceipt::Cancelled { seq: 1 });
            machine.terminal_committing = false;
        }
        state.notify.notify_waiters();

        assert_eq!(snapshot.await.unwrap(), (1, true));
    }

    #[tokio::test(start_paused = true)]
    async fn terminal_fallback_retains_request_until_five_minute_ttl() {
        let registry = ProviderStreamRegistry::default();
        let (request_id, state) = registry.insert_new(owner_label("main")).unwrap();
        let token = state.control_token.clone();
        schedule_terminal_cleanup(request_id.clone(), Arc::clone(&state), registry.clone());
        tokio::task::yield_now().await;

        tokio::time::advance(TERMINAL_RETENTION - Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(registry.authenticated(&request_id, &token).is_ok());

        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(registry.authenticated(&request_id, &token).is_err());
        assert_eq!(registry.inner.lock().unwrap().reserved_in_flight_bytes, 0);
    }

    #[test]
    fn chat_request_is_native_owned_and_minimal_without_history() {
        let request = build_chat_request(
            ChatProfile {
                provider_id: ProviderId::Anthropic,
                model_id: "claude-test".to_owned(),
            },
            "  안녕하세요  ".to_owned(),
            &[],
        )
        .unwrap();

        assert_eq!(request.provider, ProviderId::Anthropic);
        assert_eq!(request.model_id, "claude-test");
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, MessageRole::System);
        assert_eq!(request.messages[0].content, CHAT_SYSTEM_PROMPT);
        assert_eq!(
            request.messages[1],
            ChatMessage::new(MessageRole::User, "안녕하세요")
        );
        assert_eq!(
            request.generation.max_output_tokens,
            Some(CHAT_MAX_OUTPUT_TOKENS)
        );
        assert!(request.additional_parameters.is_empty());
        assert!(request.tokenizer_override.is_none());
    }

    #[test]
    fn chat_request_rejects_vertex_invalid_models_and_unbounded_messages() {
        let build = |provider_id, model_id: &str, message: String| {
            build_chat_request(
                ChatProfile {
                    provider_id,
                    model_id: model_id.to_owned(),
                },
                message,
                &[],
            )
        };

        assert!(build(ProviderId::GoogleVertexAi, "model", "hello".into()).is_err());
        assert!(build(ProviderId::GoogleGemini, "models/escape", "hello".into()).is_err());
        assert!(build(ProviderId::OpenAi, "bad\nmodel", "hello".into()).is_err());
        assert!(
            build(
                ProviderId::OpenAi,
                "model",
                "a".repeat(CHAT_MAX_INPUT_BYTES + 1),
            )
            .is_err()
        );
    }

    #[test]
    fn chat_request_includes_completed_history_and_appends_current_user_once() {
        let history = vec![
            stored_message(
                1,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                "첫 질문",
            ),
            stored_message(
                2,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Complete,
                "첫 답변",
            ),
            stored_message(
                3,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                "둘째 질문",
            ),
            stored_message(
                4,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Complete,
                "둘째 답변",
            ),
        ];
        let request = build_chat_request(
            ChatProfile {
                provider_id: ProviderId::GoogleGemini,
                model_id: "gemini-test".to_owned(),
            },
            "  지금 질문  ".to_owned(),
            &history,
        )
        .unwrap();

        assert_eq!(
            request
                .messages
                .iter()
                .map(|message| (message.role, message.content.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (MessageRole::System, CHAT_SYSTEM_PROMPT),
                (MessageRole::User, "첫 질문"),
                (MessageRole::Assistant, "첫 답변"),
                (MessageRole::User, "둘째 질문"),
                (MessageRole::Assistant, "둘째 답변"),
                (MessageRole::User, "지금 질문"),
            ]
        );
        assert_eq!(
            request
                .messages
                .iter()
                .filter(|message| message.content == "지금 질문")
                .count(),
            1
        );
        assert!(
            request
                .messages
                .iter()
                .map(|message| message.content.len())
                .sum::<usize>()
                <= CHAT_CONTEXT_MAX_UTF8_BYTES
        );
        compile_request(&request).expect("bounded Gemini history must compile");
    }

    #[test]
    fn incomplete_assistant_turns_and_their_users_are_excluded() {
        let history = vec![
            stored_message(
                1,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                "완료 질문",
            ),
            stored_message(
                2,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Complete,
                "완료 답변",
            ),
            stored_message(
                3,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                "취소된 질문",
            ),
            stored_message(
                4,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Partial,
                "부분 답변",
            ),
            stored_message(
                5,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                "실패한 질문",
            ),
            stored_message(
                6,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Failed,
                "실패 전 일부",
            ),
        ];

        let selected = select_completed_history(&history, usize::MAX, usize::MAX);
        assert_eq!(
            selected,
            vec![
                ChatMessage::new(MessageRole::User, "완료 질문"),
                ChatMessage::new(MessageRole::Assistant, "완료 답변"),
            ]
        );
    }

    #[test]
    fn history_limits_prefer_the_newest_complete_turns() {
        let mut history = Vec::new();
        for turn in 1..=4 {
            history.push(stored_message(
                turn * 2 - 1,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                format!("user-{turn}"),
            ));
            history.push(stored_message(
                turn * 2,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Complete,
                format!("assistant-{turn}"),
            ));
        }

        let selected = select_completed_history(&history, 4, usize::MAX);
        assert_eq!(
            selected
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            vec!["user-3", "assistant-3", "user-4", "assistant-4"]
        );
    }

    #[test]
    fn history_budget_counts_utf8_bytes_without_truncating_or_stale_fallback() {
        let history = vec![
            stored_message(
                1,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                "old",
            ),
            stored_message(
                2,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Complete,
                "answer",
            ),
            stored_message(
                3,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                "최신",
            ),
            stored_message(
                4,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Complete,
                "응답",
            ),
        ];
        let newest_bytes = "최신".len() + "응답".len();

        let exact = select_completed_history(&history, usize::MAX, newest_bytes);
        assert_eq!(
            exact,
            vec![
                ChatMessage::new(MessageRole::User, "최신"),
                ChatMessage::new(MessageRole::Assistant, "응답"),
            ]
        );
        assert_eq!(
            exact
                .iter()
                .map(|message| message.content.len())
                .sum::<usize>(),
            newest_bytes
        );
        assert!(
            select_completed_history(&history, usize::MAX, newest_bytes - 1).is_empty(),
            "older context must not replace an oversized newest eligible turn"
        );
    }

    #[test]
    fn bounded_multiturn_request_compiles_for_each_enabled_provider() {
        let history = vec![
            stored_message(
                1,
                StoredMessageRole::User,
                StoredMessageStatus::Complete,
                "prior question",
            ),
            stored_message(
                2,
                StoredMessageRole::Assistant,
                StoredMessageStatus::Complete,
                "prior answer",
            ),
        ];
        let enabled = [
            (ProviderId::OpenAi, "gpt-test"),
            (ProviderId::Anthropic, "claude-test"),
            (ProviderId::DeepSeek, "deepseek-test"),
            (ProviderId::OllamaCloud, "ollama-test"),
            (ProviderId::GoogleGemini, "gemini-test"),
        ];

        for (provider_id, model_id) in enabled {
            let request = build_chat_request(
                ChatProfile {
                    provider_id,
                    model_id: model_id.to_owned(),
                },
                "current question".to_owned(),
                &history,
            )
            .unwrap();
            compile_request(&request).unwrap_or_else(|error| {
                panic!("{provider_id:?} bounded multi-turn request did not compile: {error}")
            });
        }
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
    fn visible_output_limit_accepts_exact_boundary_and_rejects_next_byte() {
        let started_at = TimestampMillis::new(10).unwrap();
        let mut persistence = StreamPersistence::new(started_turn(), started_at);
        let prefix = "a".repeat(lorepia_storage::MAX_MESSAGE_BYTES - 1);
        persistence
            .record(1, Some(&prefix), None, None)
            .expect("max minus one");
        persistence
            .record(2, Some("b"), None, None)
            .expect("exact max");
        assert_eq!(
            persistence.visible_text_bytes,
            lorepia_storage::MAX_MESSAGE_BYTES
        );

        let error = persistence
            .record(3, Some("c"), None, None)
            .expect_err("max plus one");
        assert_eq!(error.code, "STREAM_VISIBLE_OUTPUT_TOO_LARGE");
        assert_eq!(persistence.through_seq, 2);
        assert_eq!(
            persistence.appended_text.len(),
            lorepia_storage::MAX_MESSAGE_BYTES
        );
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
        assert_eq!(retry.expected_last_durable_seq, 0);
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
            runtime_kind: None,
        };
        assert_eq!(
            storage_failure_code(&authentication),
            StorageFailureCode::AuthenticationFailed
        );
        let timeout = ProviderCommandError::new("STREAM_ACK_TIMEOUT", "bounded");
        assert_eq!(storage_failure_code(&timeout), StorageFailureCode::Timeout);
    }

    #[test]
    fn runtime_error_kinds_map_to_durable_failure_categories() {
        let cases = [
            (
                RuntimeErrorKind::DnsResolution,
                StorageFailureCode::NetworkUnavailable,
            ),
            (
                RuntimeErrorKind::Http,
                StorageFailureCode::NetworkUnavailable,
            ),
            (
                RuntimeErrorKind::HttpStatus,
                StorageFailureCode::ProviderRejected,
            ),
            (
                RuntimeErrorKind::Provider,
                StorageFailureCode::ProviderRejected,
            ),
            (RuntimeErrorKind::Timeout, StorageFailureCode::Timeout),
            (
                RuntimeErrorKind::StreamTooLarge,
                StorageFailureCode::ResponseTooLarge,
            ),
            (
                RuntimeErrorKind::StreamProtocol,
                StorageFailureCode::ProtocolViolation,
            ),
            (
                RuntimeErrorKind::InvalidCredential,
                StorageFailureCode::AuthenticationFailed,
            ),
        ];
        for (runtime_kind, expected) in cases {
            let mut error = ProviderCommandError::new("provider-specific-code", "bounded");
            error.runtime_kind = Some(runtime_kind);
            assert_eq!(storage_failure_code(&error), expected, "{runtime_kind:?}");
        }

        let mut not_found = ProviderCommandError::new("OPENAI_HTTP_ERROR", "bounded");
        not_found.http_status = Some(404);
        not_found.runtime_kind = Some(RuntimeErrorKind::HttpStatus);
        assert_eq!(
            storage_failure_code(&not_found),
            StorageFailureCode::ProviderRejected
        );
    }
}
