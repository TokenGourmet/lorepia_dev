use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use lorepia_credential_vault::{CredentialVaultError, CredentialVaultErrorCode};
use lorepia_provider_runtime::{
    CompletionReason, EndpointSelection, ProviderCredential, ProviderRunOutcome, ProviderRuntime,
    ProviderStreamEvent as RuntimeStreamEvent, RuntimeError, TokenUsage,
};
use lorepia_providers::{ProviderId, ProviderRequest};
use serde::Serialize;
use subtle::ConstantTimeEq;
use tauri::{State, ipc::Channel};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::credential_commands::{CredentialVaultState, run_vault_operation};

const MAX_ACTIVE_STREAMS: usize = 128;
const MAX_IN_FLIGHT: u64 = 4;
const ACK_TIMEOUT: Duration = Duration::from_secs(30);
const TERMINAL_RETENTION: Duration = Duration::from_secs(5 * 60);
const DIRECT_CHANNEL_BUDGET_BYTES: usize = 4_096;
const MAX_DELTA_FRAGMENT_BYTES: usize = 512;
const MAX_PROVIDER_RESPONSE_ID_BYTES: usize = 256;

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
}

impl StreamMachine {
    const fn after_started() -> Self {
        Self {
            last_sent_seq: 0,
            acknowledged_through: None,
            cancel_requested: false,
            terminal: None,
            terminal_snapshot_returned: false,
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
    request: ProviderRequest,
    endpoint: EndpointSelection,
    on_event: Channel<ProviderChannelEvent>,
    vault: State<'_, CredentialVaultState>,
    registry: State<'_, ProviderStreamRegistry>,
) -> Result<StartProviderStreamResponse, ProviderCommandError> {
    let credential = load_provider_credential(&request, &endpoint, vault.vault()).await?;
    let (request_id, state) = registry.insert_new()?;
    let started = ProviderChannelEvent::Started {
        request_id: request_id.clone(),
        seq: 0,
        max_in_flight: MAX_IN_FLIGHT,
    };
    if let Err(error) = send_direct(&on_event, started) {
        registry.remove_if_same(&request_id, &state);
        return Err(error);
    }

    let control_token = state.control_token.clone();
    let request_id_for_task = request_id.clone();
    let registry_for_task = registry.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_stream_bridge(
            request_id_for_task,
            state,
            request,
            endpoint,
            credential,
            on_event,
            registry_for_task,
        )
        .await;
    });
    Ok(StartProviderStreamResponse {
        request_id,
        control_token,
    })
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
        if machine.cancel_requested || machine.terminal.is_some() {
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

async fn run_stream_bridge(
    request_id: String,
    state: Arc<StreamRequestState>,
    request: ProviderRequest,
    endpoint: EndpointSelection,
    credential: ProviderCredential,
    channel: Channel<ProviderChannelEvent>,
    registry: ProviderStreamRegistry,
) {
    let (event_tx, mut event_rx) = mpsc::channel(1);
    let runtime = ProviderRuntime::new();
    let cancellation = state.cancellation.clone();
    let run = runtime.run_stream(
        request,
        endpoint,
        credential,
        cancellation.clone(),
        event_tx,
    );
    tokio::pin!(run);

    let result = loop {
        tokio::select! {
            biased;
            event = event_rx.recv() => {
                match event {
                    Some(event) => {
                        if let Err(error) = forward_runtime_event(
                            &request_id,
                            &state,
                            &channel,
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

    if send_terminal(&request_id, &state, &channel, terminal)
        .await
        .is_err()
    {
        registry.remove_if_same(&request_id, &state);
        return;
    }
    schedule_terminal_cleanup(request_id, state, registry);
}

async fn forward_runtime_event(
    request_id: &str,
    state: &Arc<StreamRequestState>,
    channel: &Channel<ProviderChannelEvent>,
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
            send_non_terminal(request_id, state, channel, |request_id, seq| {
                ProviderChannelEvent::ProviderResponseId {
                    request_id,
                    seq,
                    id,
                }
            })
            .await
        }
        RuntimeStreamEvent::TextDelta { text } => {
            forward_text_fragments(request_id, state, channel, text, DeltaKind::Text).await
        }
        RuntimeStreamEvent::ReasoningDelta { text } => {
            forward_text_fragments(request_id, state, channel, text, DeltaKind::Reasoning).await
        }
        RuntimeStreamEvent::RefusalDelta { text } => {
            forward_text_fragments(request_id, state, channel, text, DeltaKind::Refusal).await
        }
        RuntimeStreamEvent::Usage { usage } => {
            send_non_terminal(request_id, state, channel, |request_id, seq| {
                ProviderChannelEvent::Usage {
                    request_id,
                    seq,
                    usage,
                }
            })
            .await
        }
    }
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
    text: String,
    kind: DeltaKind,
) -> Result<(), ProviderCommandError> {
    for fragment in split_utf8(&text, MAX_DELTA_FRAGMENT_BYTES) {
        send_non_terminal(request_id, state, channel, |request_id, seq| match kind {
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
    }
    Ok(())
}

async fn send_non_terminal(
    request_id: &str,
    state: &Arc<StreamRequestState>,
    channel: &Channel<ProviderChannelEvent>,
    make_event: impl FnOnce(String, u64) -> ProviderChannelEvent,
) -> Result<(), ProviderCommandError> {
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
            if machine.terminal.is_some() {
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
                return Ok(());
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
    terminal: TerminalKind,
) -> Result<(), ProviderCommandError> {
    let mut machine = state.machine.lock().await;
    if machine.terminal.is_some() {
        return Ok(());
    }
    let seq = machine.last_sent_seq.saturating_add(1);
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
    send_direct(channel, event)?;
    machine.last_sent_seq = seq;
    machine.terminal = Some(receipt);
    state.notify.notify_waiters();
    Ok(())
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
}
