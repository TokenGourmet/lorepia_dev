mod host_broker;

include!("app_commands.rs");

macro_rules! generate_lorepia_handler {
    ($($command:ident),+ $(,)?) => {
        tauri::generate_handler![$($command),+]
    };
}

macro_rules! lorepia_command_names {
    ($($command:ident),+ $(,)?) => {
        &[$(stringify!($command)),+]
    };
}

const APP_COMMAND_NAMES: &[&str] = with_lorepia_app_commands!(lorepia_command_names);
const COMMAND_SURFACE_VERSION: u32 = 3;

use host_broker::{
    AuthorizedAction, BrokerAction, BrokerError, HostBroker, RegistrationOutcome,
    RegistrationPolicy, RotationOutcome, SystemMonotonicClock,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, State};
use tokio::sync::{watch, Mutex as AsyncMutex, Notify};

const MIN_BATCH_WINDOW_MS: u64 = 16;
const MAX_BATCH_WINDOW_MS: u64 = 50;
const DEFAULT_BATCH_WINDOW_MS: u64 = 24;
const DEFAULT_MAX_IN_FLIGHT: usize = 4;
const DEFAULT_CHUNK_INTERVAL_MS: u64 = 8;
const DEFAULT_ACK_TIMEOUT_MS: u64 = 1_000;
const MAX_REQUESTS: usize = 128;
const TERMINAL_RETENTION_TTL: Duration = Duration::from_secs(5 * 60);
const NO_TERMINAL_SEQ: u64 = u64::MAX;
const MAX_SEQUENCE: u64 = 9_007_199_254_740_991;
const STREAM_REQUEST_ID_PREFIX: &str = "m1-channel-";
const STREAM_REQUEST_ID_HEX_BYTES: usize = 16;
const MAX_PLUGIN_HTML_BYTES: usize = host_broker::MAX_SANITIZE_HTML_BYTES;
// Tauri 2.11 sends JSON Channel payloads smaller than 8192 bytes directly by
// evaluating them in the destination WebView. Larger payloads are placed in a
// process-global, numerically indexed fetch queue whose fetch command bypasses
// normal ACL resolution. Keep every LorePia Channel event on the direct path so
// an untrusted same-process WebView cannot race that queue.
const TAURI_CHANNEL_JSON_DIRECT_THRESHOLD_BYTES: usize = 8_192;
const LOREPIA_DIRECT_JSON_BUDGET_BYTES: usize = 4_096;
const MAX_CHANNEL_REQUEST_ID: &str = "m1-channel-ffffffffffffffff";
const _: () = assert!(LOREPIA_DIRECT_JSON_BUDGET_BYTES < TAURI_CHANNEL_JSON_DIRECT_THRESHOLD_BYTES);
const _: () = assert!(
    MAX_CHANNEL_REQUEST_ID.len() == STREAM_REQUEST_ID_PREFIX.len() + STREAM_REQUEST_ID_HEX_BYTES
);

static HOST_BROKER_PROBE_CALLS: AtomicU64 = AtomicU64::new(0);
static HOST_BROKER_SANITIZE_CALLS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StreamFailure {
    code: String,
    message: String,
}

impl StreamFailure {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: truncate_utf8(code.into(), 64),
            message: truncate_utf8(message.into(), 512),
        }
    }
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
enum StreamEvent {
    Started {
        request_id: String,
        seq: u64,
        batch_window_ms: u64,
        max_in_flight: usize,
    },
    Delta {
        request_id: String,
        seq: u64,
        text: String,
    },
    Completed {
        request_id: String,
        seq: u64,
    },
    Cancelled {
        request_id: String,
        seq: u64,
    },
    Failed {
        request_id: String,
        seq: u64,
        error: StreamFailure,
    },
}

fn serialized_json_len(value: &impl Serialize) -> Option<usize> {
    serde_json::to_vec(value).ok().map(|encoded| encoded.len())
}

fn serialized_channel_event_len(event: &StreamEvent) -> Option<usize> {
    serialized_json_len(event)
}

fn json_value_uses_direct_path(value: &impl Serialize) -> bool {
    serialized_json_len(value).is_some_and(|len| len <= LOREPIA_DIRECT_JSON_BUDGET_BYTES)
}

fn channel_event_uses_direct_path(event: &StreamEvent) -> bool {
    json_value_uses_direct_path(event)
}

fn direct_delta_fits(text: &str) -> bool {
    channel_event_uses_direct_path(&StreamEvent::Delta {
        request_id: MAX_CHANNEL_REQUEST_ID.to_owned(),
        seq: MAX_SEQUENCE,
        text: text.to_owned(),
    })
}

fn send_direct_channel_event(
    channel: &Channel<StreamEvent>,
    event: StreamEvent,
) -> Result<(), String> {
    let Some(encoded_len) = serialized_channel_event_len(&event) else {
        return Err("failed to serialize Channel event".to_owned());
    };
    if encoded_len > LOREPIA_DIRECT_JSON_BUDGET_BYTES {
        return Err(format!(
            "Channel event is {encoded_len} bytes; LorePia direct-execute budget is {LOREPIA_DIRECT_JSON_BUDGET_BYTES} bytes"
        ));
    }
    channel.send(event).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StreamConfig {
    batch_window_ms: Option<u64>,
    max_in_flight: Option<usize>,
    chunk_interval_ms: Option<u64>,
    chunks: Option<Vec<String>>,
    fail_after_chunks: Option<usize>,
    ack_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct ValidatedConfig {
    batch_window_ms: u64,
    max_in_flight: usize,
    chunk_interval_ms: u64,
    chunks: Vec<String>,
    fail_after_chunks: Option<usize>,
    ack_timeout_ms: u64,
}

impl TryFrom<StreamConfig> for ValidatedConfig {
    type Error = CommandError;

    fn try_from(value: StreamConfig) -> Result<Self, Self::Error> {
        let batch_window_ms = value.batch_window_ms.unwrap_or(DEFAULT_BATCH_WINDOW_MS);
        if !(MIN_BATCH_WINDOW_MS..=MAX_BATCH_WINDOW_MS).contains(&batch_window_ms) {
            return Err(CommandError::invalid_config(format!(
                "batchWindowMs must be between {MIN_BATCH_WINDOW_MS} and {MAX_BATCH_WINDOW_MS}"
            )));
        }

        let max_in_flight = value.max_in_flight.unwrap_or(DEFAULT_MAX_IN_FLIGHT);
        if !(2..=64).contains(&max_in_flight) {
            return Err(CommandError::invalid_config(
                "maxInFlight must be between 2 and 64",
            ));
        }

        let chunk_interval_ms = value.chunk_interval_ms.unwrap_or(DEFAULT_CHUNK_INTERVAL_MS);
        if !(1..=1_000).contains(&chunk_interval_ms) {
            return Err(CommandError::invalid_config(
                "chunkIntervalMs must be between 1 and 1000",
            ));
        }

        let chunks = value.chunks.unwrap_or_else(default_chunks);
        if chunks.is_empty() || chunks.len() > 4_096 {
            return Err(CommandError::invalid_config(
                "chunks must contain between 1 and 4096 entries",
            ));
        }
        if chunks.iter().any(String::is_empty) {
            return Err(CommandError::invalid_config(
                "chunks must not contain empty strings",
            ));
        }
        if chunks.iter().any(|chunk| chunk.len() > 16_384) {
            return Err(CommandError::invalid_config(
                "each chunk must be at most 16384 bytes",
            ));
        }
        let total_bytes = chunks
            .iter()
            .try_fold(0usize, |total, chunk| total.checked_add(chunk.len()))
            .ok_or_else(|| CommandError::invalid_config("chunks byte length overflow"))?;
        if total_bytes > 1_048_576 {
            return Err(CommandError::invalid_config(
                "chunks must total at most 1048576 bytes",
            ));
        }

        if value
            .fail_after_chunks
            .is_some_and(|count| count > chunks.len())
        {
            return Err(CommandError::invalid_config(
                "failAfterChunks cannot exceed chunks.length",
            ));
        }

        let ack_timeout_ms = value.ack_timeout_ms.unwrap_or(DEFAULT_ACK_TIMEOUT_MS);
        if !(10..=60_000).contains(&ack_timeout_ms) {
            return Err(CommandError::invalid_config(
                "ackTimeoutMs must be between 10 and 60000",
            ));
        }

        Ok(Self {
            batch_window_ms,
            max_in_flight,
            chunk_interval_ms,
            chunks,
            fail_after_chunks: value.fail_after_chunks,
            ack_timeout_ms,
        })
    }
}

fn default_chunks() -> Vec<String> {
    [
        "LorePia ",
        "Channel ",
        "spike는 ",
        "순서가 ",
        "보장된 ",
        "delta와 ",
        "ACK 기반 ",
        "backpressure, ",
        "취소 후 ",
        "부분 텍스트 ",
        "스냅샷을 ",
        "검증합니다.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum StreamStatus {
    Queued,
    Streaming,
    Completed,
    Cancelled,
    Failed,
}

impl StreamStatus {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug)]
struct StreamMachine {
    request_id: String,
    status: StreamStatus,
    last_seq: Option<u64>,
    last_acked_seq: Option<u64>,
    text: String,
    error: Option<StreamFailure>,
    batch_window_ms: u64,
    effective_batch_window_ms: u64,
    max_in_flight: usize,
    cancel_requested: bool,
}

impl StreamMachine {
    fn new(request_id: String, config: &ValidatedConfig) -> Self {
        Self {
            request_id,
            status: StreamStatus::Queued,
            last_seq: None,
            last_acked_seq: None,
            text: String::new(),
            error: None,
            batch_window_ms: config.batch_window_ms,
            effective_batch_window_ms: config.batch_window_ms,
            max_in_flight: config.max_in_flight,
            cancel_requested: false,
        }
    }

    fn in_flight(&self) -> usize {
        let emitted = self.last_seq.map_or(0, |seq| seq + 1);
        let acknowledged = self.last_acked_seq.map_or(0, |seq| seq + 1);
        usize::try_from(emitted.saturating_sub(acknowledged)).unwrap_or(usize::MAX)
    }

    fn has_data_capacity(&self) -> bool {
        // One slot is permanently reserved for the terminal event. This lets a
        // cancellation terminate even when the frontend stops ACKing entirely.
        self.in_flight() < self.max_in_flight - 1
    }

    fn has_terminal_capacity(&self) -> bool {
        self.in_flight() < self.max_in_flight
    }

    fn advance_sequence(&mut self) -> Result<u64, CommandError> {
        let next = match self.last_seq {
            Some(last) => last
                .checked_add(1)
                .ok_or_else(CommandError::sequence_exhausted)?,
            None => 0,
        };
        if next > MAX_SEQUENCE {
            return Err(CommandError::sequence_exhausted());
        }
        self.last_seq = Some(next);
        Ok(next)
    }

    fn start(&mut self) -> Result<StreamEvent, CommandError> {
        if self.status != StreamStatus::Queued {
            return Err(CommandError::invalid_state("stream already started"));
        }
        let seq = self.advance_sequence()?;
        self.status = StreamStatus::Streaming;
        Ok(StreamEvent::Started {
            request_id: self.request_id.clone(),
            seq,
            batch_window_ms: self.batch_window_ms,
            max_in_flight: self.max_in_flight,
        })
    }

    fn acknowledge(&mut self, seq: u64) -> Result<u64, CommandError> {
        let last_seq = self.last_seq.ok_or_else(|| {
            CommandError::invalid_ack("cannot acknowledge before the first event is emitted")
        })?;
        if seq > last_seq {
            return Err(CommandError::invalid_ack(format!(
                "seq {seq} has not been emitted; last emitted seq is {last_seq}"
            )));
        }
        let acknowledged_through = self.last_acked_seq.map_or(seq, |last| last.max(seq));
        self.last_acked_seq = Some(acknowledged_through);
        Ok(acknowledged_through)
    }

    fn apply_pressure(&mut self) {
        self.effective_batch_window_ms = MAX_BATCH_WINDOW_MS;
    }

    fn request_cancel(&mut self) -> bool {
        if self.status.is_terminal() {
            return false;
        }
        self.cancel_requested = true;
        true
    }

    fn delta(&mut self, text: String) -> Result<StreamEvent, CommandError> {
        self.ensure_data_allowed()?;
        if text.is_empty() {
            return Err(CommandError::invalid_state("delta text must not be empty"));
        }
        if !self.has_data_capacity() {
            return Err(CommandError::backpressure());
        }
        let seq = self.advance_sequence()?;
        self.text.push_str(&text);
        Ok(StreamEvent::Delta {
            request_id: self.request_id.clone(),
            seq,
            text,
        })
    }

    fn complete(&mut self) -> Result<StreamEvent, CommandError> {
        self.ensure_terminal_allowed()?;
        if self.cancel_requested {
            return Err(CommandError::invalid_state(
                "completion is not allowed after cancellation is accepted",
            ));
        }
        let seq = self.advance_sequence()?;
        self.status = StreamStatus::Completed;
        Ok(StreamEvent::Completed {
            request_id: self.request_id.clone(),
            seq,
        })
    }

    fn cancel(&mut self) -> Result<StreamEvent, CommandError> {
        self.ensure_terminal_allowed()?;
        let seq = self.advance_sequence()?;
        self.status = StreamStatus::Cancelled;
        Ok(StreamEvent::Cancelled {
            request_id: self.request_id.clone(),
            seq,
        })
    }

    fn fail(&mut self, failure: StreamFailure) -> Result<StreamEvent, CommandError> {
        self.ensure_terminal_allowed()?;
        if self.cancel_requested {
            return Err(CommandError::invalid_state(
                "failure is not allowed after cancellation is accepted",
            ));
        }
        let seq = self.advance_sequence()?;
        self.status = StreamStatus::Failed;
        self.error = Some(failure.clone());
        Ok(StreamEvent::Failed {
            request_id: self.request_id.clone(),
            seq,
            error: failure,
        })
    }

    fn ensure_data_allowed(&self) -> Result<(), CommandError> {
        if self.status != StreamStatus::Streaming || self.cancel_requested {
            return Err(CommandError::invalid_state(
                "delta is not allowed after cancellation or termination",
            ));
        }
        Ok(())
    }

    fn ensure_terminal_allowed(&self) -> Result<(), CommandError> {
        if self.status != StreamStatus::Streaming {
            return Err(CommandError::invalid_state(
                "exactly one terminal event is allowed",
            ));
        }
        if !self.has_terminal_capacity() {
            return Err(CommandError::backpressure());
        }
        Ok(())
    }

    fn snapshot(&self) -> StreamSnapshot {
        StreamSnapshot {
            request_id: self.request_id.clone(),
            status: self.status,
            last_seq: self.last_seq,
            last_acked_seq: self.last_acked_seq,
            in_flight: self.in_flight(),
            text_bytes: self.text.len(),
            text_sha256: format!("{:x}", Sha256::digest(self.text.as_bytes())),
            error: self.error.clone(),
            batch_window_ms: self.batch_window_ms,
            effective_batch_window_ms: self.effective_batch_window_ms,
            max_in_flight: self.max_in_flight,
        }
    }
}

struct StreamRequest {
    machine: AsyncMutex<StreamMachine>,
    notify: Notify,
    terminal_signal: watch::Sender<bool>,
    terminal_waiter_active: AtomicBool,
    terminal_seq: AtomicU64,
    terminal_acked: AtomicBool,
    terminal_snapshot_returned: AtomicBool,
    control_plane_only_terminal: AtomicBool,
    terminal_reached_at: Mutex<Option<Instant>>,
    evictable: AtomicBool,
}

impl StreamRequest {
    fn new(machine: StreamMachine) -> Self {
        let (terminal_signal, _terminal_receiver) = watch::channel(false);
        Self {
            machine: AsyncMutex::new(machine),
            notify: Notify::new(),
            terminal_signal,
            terminal_waiter_active: AtomicBool::new(false),
            terminal_seq: AtomicU64::new(NO_TERMINAL_SEQ),
            terminal_acked: AtomicBool::new(false),
            terminal_snapshot_returned: AtomicBool::new(false),
            control_plane_only_terminal: AtomicBool::new(false),
            terminal_reached_at: Mutex::new(None),
            evictable: AtomicBool::new(false),
        }
    }

    fn record_terminal_delivery(&self, seq: u64) {
        self.terminal_seq.store(seq, Ordering::Release);
        self.signal_terminal();
    }

    fn record_acknowledged_through(&self, acknowledged_through: u64) {
        let terminal_seq = self.terminal_seq.load(Ordering::Acquire);
        if terminal_seq != NO_TERMINAL_SEQ && acknowledged_through >= terminal_seq {
            self.terminal_acked.store(true, Ordering::Release);
            self.refresh_evictable();
        }
    }

    fn record_control_plane_only_terminal(&self) {
        // No terminal Channel event exists when handoff or an internal stream
        // transition fails. This explicit state selects the control-plane-only
        // cleanup policy; delivered terminals still require ACK plus snapshot.
        self.control_plane_only_terminal
            .store(true, Ordering::Release);
        self.refresh_evictable();
        self.signal_terminal();
    }

    fn signal_terminal(&self) {
        if let Ok(mut terminal_reached_at) = self.terminal_reached_at.lock() {
            terminal_reached_at.get_or_insert_with(Instant::now);
        }
        let _previous = self.terminal_signal.send_replace(true);
        self.notify.notify_waiters();
    }

    fn terminal_retention_expired(&self, now: Instant) -> bool {
        self.terminal_reached_at
            .lock()
            .ok()
            .and_then(|reached_at| *reached_at)
            .is_some_and(|reached_at| {
                now.saturating_duration_since(reached_at) >= TERMINAL_RETENTION_TTL
            })
    }

    async fn wait_for_terminal(&self) -> Result<(), CommandError> {
        let mut receiver = self.terminal_signal.subscribe();
        loop {
            if *receiver.borrow_and_update() {
                return Ok(());
            }
            receiver
                .changed()
                .await
                .map_err(|_| CommandError::internal("terminal signal closed unexpectedly"))?;
        }
    }

    fn acquire_terminal_waiter(&self) -> Result<TerminalWaitLease<'_>, CommandError> {
        self.terminal_waiter_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| CommandError::terminal_waiter_busy())?;
        Ok(TerminalWaitLease { request: self })
    }

    fn record_terminal_snapshot_returned(&self, snapshot: &StreamSnapshot) {
        let terminal_seq = self.terminal_seq.load(Ordering::Acquire);
        let delivered_terminal = terminal_seq != NO_TERMINAL_SEQ
            && snapshot.status.is_terminal()
            && snapshot.last_seq == Some(terminal_seq);
        let undeliverable_failure = terminal_seq == NO_TERMINAL_SEQ
            && self.control_plane_only_terminal.load(Ordering::Acquire)
            && snapshot.status == StreamStatus::Failed;

        if delivered_terminal || undeliverable_failure {
            self.terminal_snapshot_returned
                .store(true, Ordering::Release);
            self.refresh_evictable();
        }
    }

    fn refresh_evictable(&self) {
        let terminal_seq = self.terminal_seq.load(Ordering::Acquire);
        let delivered_terminal_is_releasable =
            terminal_seq != NO_TERMINAL_SEQ && self.terminal_acked.load(Ordering::Acquire);
        let undeliverable_failure_is_releasable = terminal_seq == NO_TERMINAL_SEQ
            && self.control_plane_only_terminal.load(Ordering::Acquire);

        if self.terminal_snapshot_returned.load(Ordering::Acquire)
            && (delivered_terminal_is_releasable || undeliverable_failure_is_releasable)
        {
            self.evictable.store(true, Ordering::Release);
        }
    }
}

struct TerminalWaitLease<'a> {
    request: &'a StreamRequest,
}

impl Drop for TerminalWaitLease<'_> {
    fn drop(&mut self) {
        self.request
            .terminal_waiter_active
            .store(false, Ordering::Release);
    }
}

#[derive(Default)]
struct RegistryInner {
    requests: HashMap<String, Arc<StreamRequest>>,
    order: VecDeque<String>,
}

#[derive(Default)]
struct StreamRegistry {
    next_id: AtomicU64,
    inner: Mutex<RegistryInner>,
}

impl StreamRegistry {
    fn next_request_id(&self) -> Result<String, CommandError> {
        let previous = self
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| CommandError::internal("request ID space exhausted"))?;
        Ok(format!("m1-channel-{:016x}", previous + 1))
    }

    fn insert(&self, request_id: String, request: Arc<StreamRequest>) -> Result<(), CommandError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| CommandError::internal("stream registry lock poisoned"))?;

        if inner.requests.contains_key(&request_id) {
            return Err(CommandError::internal(format!(
                "duplicate stream requestId {request_id}"
            )));
        }

        while inner.requests.len() >= MAX_REQUESTS {
            let now = Instant::now();
            let expired_terminal = inner.order.iter().position(|id| {
                inner
                    .requests
                    .get(id)
                    .is_some_and(|entry| entry.terminal_retention_expired(now))
            });
            let Some(index) = expired_terminal else {
                return Err(CommandError::capacity());
            };
            if let Some(id) = inner.order.remove(index) {
                inner.requests.remove(&id);
            }
        }

        inner.order.push_back(request_id.clone());
        inner.requests.insert(request_id, request);
        Ok(())
    }

    fn get(&self, request_id: &str) -> Result<Arc<StreamRequest>, CommandError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| CommandError::internal("stream registry lock poisoned"))?;
        inner
            .requests
            .get(request_id)
            .cloned()
            .ok_or_else(CommandError::not_found)
    }

    fn remove_exact(
        &self,
        request_id: &str,
        expected: &Arc<StreamRequest>,
    ) -> Result<bool, CommandError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| CommandError::internal("stream registry lock poisoned"))?;
        let matches = inner
            .requests
            .get(request_id)
            .is_some_and(|registered| Arc::ptr_eq(registered, expected));
        if matches {
            inner.requests.remove(request_id);
            inner.order.retain(|id| id != request_id);
        }
        Ok(matches)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartStreamResponse {
    request_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AckStreamResponse {
    request_id: String,
    acknowledged_through: u64,
    in_flight: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelStreamResponse {
    request_id: String,
    accepted: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ReleaseStreamResponse {
    request_id: String,
    released: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StreamSnapshot {
    request_id: String,
    status: StreamStatus,
    last_seq: Option<u64>,
    last_acked_seq: Option<u64>,
    in_flight: usize,
    text_bytes: usize,
    text_sha256: String,
    error: Option<StreamFailure>,
    batch_window_ms: u64,
    effective_batch_window_ms: u64,
    max_in_flight: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SanitizedHtmlResponse {
    html: String,
    input_bytes: usize,
    output_bytes: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RegisterHostBrokerSessionResponse {
    outcome: RegistrationOutcome,
    generation: u64,
    module_id: String,
    network_policy: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RotateHostBrokerSessionResponse {
    outcome: RotationOutcome,
    generation: u64,
    module_id: String,
    network_policy: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct HostBrokerRequestResponse {
    request_id: String,
    module_id: String,
    result: HostBrokerResult,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum HostBrokerResult {
    StateRead {
        state: &'static str,
    },
    RenderSanitize {
        html: String,
        input_bytes: usize,
        output_bytes: usize,
    },
    ProbeIncrement {
        sentinel: &'static str,
        call_count: u64,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct HostBrokerProbeCountResponse {
    probe_call_count: u64,
    sanitize_call_count: u64,
    command_surface_version: u32,
    command_names: &'static [&'static str],
    command_sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CommandError {
    code: String,
    message: String,
}

impl CommandError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }

    fn invalid_config(message: impl Into<String>) -> Self {
        Self::new("INVALID_CONFIG", message)
    }

    fn invalid_state(message: impl Into<String>) -> Self {
        Self::new("INVALID_STATE", message)
    }

    fn invalid_ack(message: impl Into<String>) -> Self {
        Self::new("INVALID_ACK", message)
    }

    fn invalid_release(message: impl Into<String>) -> Self {
        Self::new("INVALID_RELEASE", message)
    }

    fn backpressure() -> Self {
        Self::new("BACKPRESSURE", "maxInFlight limit reached")
    }

    fn sequence_exhausted() -> Self {
        Self::new(
            "SEQUENCE_EXHAUSTED",
            "stream sequence exceeded JavaScript's safe integer range",
        )
    }

    fn invalid_request_id() -> Self {
        Self::new(
            "INVALID_REQUEST_ID",
            "requestId must use the server-issued stream identifier format",
        )
    }

    fn terminal_waiter_busy() -> Self {
        Self::new(
            "TERMINAL_WAITER_BUSY",
            "one terminal waiter is already active for this stream",
        )
    }

    fn not_found() -> Self {
        Self::new("STREAM_NOT_FOUND", "stream request was not found")
    }

    fn capacity() -> Self {
        Self::new(
            "REGISTRY_CAPACITY",
            "too many retained streams; finish the terminal snapshot/release handshake or wait for terminal TTL cleanup before retrying",
        )
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new("INTERNAL", message)
    }

    fn html_too_large() -> Self {
        Self::new(
            "HTML_TOO_LARGE",
            format!("plugin HTML must be at most {MAX_PLUGIN_HTML_BYTES} bytes"),
        )
    }
}

fn validate_stream_request_id(request_id: &str) -> Result<(), CommandError> {
    let Some(suffix) = request_id.strip_prefix(STREAM_REQUEST_ID_PREFIX) else {
        return Err(CommandError::invalid_request_id());
    };
    if suffix.len() != STREAM_REQUEST_ID_HEX_BYTES
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CommandError::invalid_request_id());
    }
    Ok(())
}

fn sanitize_plugin_html_value(input: &str) -> Result<SanitizedHtmlResponse, CommandError> {
    if input.len() > MAX_PLUGIN_HTML_BYTES {
        return Err(CommandError::html_too_large());
    }

    // This is an intentionally small presentation-only vocabulary. In
    // particular there are no links, media, style-bearing attributes, form
    // controls, SVG/MathML, or URL-bearing elements.
    let allowed_tags = HashSet::from([
        "p",
        "br",
        "strong",
        "em",
        "code",
        "pre",
        "ul",
        "ol",
        "li",
        "blockquote",
        "span",
    ]);
    let clean_content_tags = HashSet::from([
        "script", "style", "iframe", "object", "embed", "svg", "math", "template", "form",
        "noscript",
    ]);

    let mut builder = ammonia::Builder::default();
    builder
        .tags(allowed_tags)
        .tag_attributes(HashMap::new())
        .generic_attributes(HashSet::new())
        .url_schemes(HashSet::new())
        .url_relative(ammonia::UrlRelative::Deny)
        .clean_content_tags(clean_content_tags);

    let html = builder.clean(input).to_string();
    Ok(SanitizedHtmlResponse {
        input_bytes: input.len(),
        output_bytes: html.len(),
        html,
    })
}

#[tauri::command]
fn register_host_broker_session(
    host_token: Option<String>,
    policy: RegistrationPolicy,
    broker: State<'_, HostBroker<SystemMonotonicClock>>,
) -> Result<RegisterHostBrokerSessionResponse, BrokerError> {
    let receipt = broker.register(host_token.as_deref(), policy)?;
    Ok(RegisterHostBrokerSessionResponse {
        outcome: receipt.outcome,
        generation: receipt.generation,
        module_id: receipt.module_id,
        network_policy: "deny",
    })
}

#[tauri::command]
fn rotate_host_broker_session(
    current_host_token: Option<String>,
    next_host_token: Option<String>,
    expected_generation: u64,
    broker: State<'_, HostBroker<SystemMonotonicClock>>,
) -> Result<RotateHostBrokerSessionResponse, BrokerError> {
    let receipt = broker.rotate(
        current_host_token.as_deref(),
        next_host_token.as_deref(),
        expected_generation,
    )?;
    Ok(RotateHostBrokerSessionResponse {
        outcome: receipt.outcome,
        generation: receipt.generation,
        module_id: receipt.module_id,
        network_policy: "deny",
    })
}

#[tauri::command]
fn host_broker_request(
    host_token: Option<String>,
    request_json: String,
    broker: State<'_, HostBroker<SystemMonotonicClock>>,
) -> Result<HostBrokerRequestResponse, BrokerError> {
    broker.execute_json(
        host_token.as_deref(),
        &request_json,
        execute_host_broker_action,
    )
}

fn execute_host_broker_action(
    authorized: AuthorizedAction<'_>,
) -> Result<HostBrokerRequestResponse, BrokerError> {
    let AuthorizedAction {
        request_id,
        module_id,
        action,
    } = authorized;
    let result = match action {
        BrokerAction::StateRead => HostBrokerResult::StateRead { state: "ready" },
        BrokerAction::RenderSanitize { html } => {
            let sanitized = sanitize_plugin_html_value(&html)
                .map_err(|_| BrokerError::action_failed(&request_id))?;
            HOST_BROKER_SANITIZE_CALLS.fetch_add(1, Ordering::SeqCst);
            HostBrokerResult::RenderSanitize {
                html: sanitized.html,
                input_bytes: sanitized.input_bytes,
                output_bytes: sanitized.output_bytes,
            }
        }
        BrokerAction::ProbeIncrement => {
            let call_count = HOST_BROKER_PROBE_CALLS.fetch_add(1, Ordering::SeqCst) + 1;
            HostBrokerResult::ProbeIncrement {
                sentinel: "LOREPIA_HOST_BROKER_PROBE_REACHED",
                call_count,
            }
        }
    };

    let response = HostBrokerRequestResponse {
        request_id,
        module_id: module_id.to_owned(),
        result,
    };
    ensure_direct_broker_response(response)
}

fn ensure_direct_broker_response(
    response: HostBrokerRequestResponse,
) -> Result<HostBrokerRequestResponse, BrokerError> {
    if json_value_uses_direct_path(&response) {
        Ok(response)
    } else {
        Err(BrokerError::action_failed(&response.request_id))
    }
}

#[tauri::command]
fn host_broker_probe_count() -> HostBrokerProbeCountResponse {
    let command_manifest = format!("{}\n", APP_COMMAND_NAMES.join("\n"));
    HostBrokerProbeCountResponse {
        probe_call_count: HOST_BROKER_PROBE_CALLS.load(Ordering::SeqCst),
        sanitize_call_count: HOST_BROKER_SANITIZE_CALLS.load(Ordering::SeqCst),
        command_surface_version: COMMAND_SURFACE_VERSION,
        command_names: APP_COMMAND_NAMES,
        command_sha256: format!("{:x}", Sha256::digest(command_manifest.as_bytes())),
    }
}

#[tauri::command]
async fn start_mock_stream(
    on_event: Channel<StreamEvent>,
    config: Option<StreamConfig>,
    registry: State<'_, StreamRegistry>,
) -> Result<StartStreamResponse, CommandError> {
    let config = ValidatedConfig::try_from(config.unwrap_or_default())?;
    let request_id = registry.next_request_id()?;
    let request = Arc::new(StreamRequest::new(StreamMachine::new(
        request_id.clone(),
        &config,
    )));
    registry.insert(request_id.clone(), Arc::clone(&request))?;

    let started = {
        let mut machine = request.machine.lock().await;
        machine.start()
    };
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            if !registry.remove_exact(&request_id, &request)? {
                return Err(CommandError::internal(
                    "failed to remove a stream after start transition failure",
                ));
            }
            return Err(error);
        }
    };
    if let Err(error) = send_direct_channel_event(&on_event, started) {
        if !registry.remove_exact(&request_id, &request)? {
            return Err(CommandError::internal(
                "failed to remove a stream after started-event delivery failure",
            ));
        }
        return Err(CommandError::internal(format!(
            "failed to deliver started event: {error}"
        )));
    }

    tauri::async_runtime::spawn(run_stream(Arc::clone(&request), config, on_event));
    Ok(StartStreamResponse { request_id })
}

#[tauri::command]
async fn ack_stream(
    request_id: String,
    seq: u64,
    registry: State<'_, StreamRegistry>,
) -> Result<AckStreamResponse, CommandError> {
    validate_stream_request_id(&request_id)?;
    let request = registry.get(&request_id)?;
    let response = {
        let mut machine = request.machine.lock().await;
        let acknowledged_through = machine.acknowledge(seq)?;
        request.record_acknowledged_through(acknowledged_through);
        AckStreamResponse {
            request_id,
            acknowledged_through,
            in_flight: machine.in_flight(),
        }
    };
    request.notify.notify_one();
    Ok(response)
}

#[tauri::command]
async fn cancel_stream(
    request_id: String,
    registry: State<'_, StreamRegistry>,
) -> Result<CancelStreamResponse, CommandError> {
    validate_stream_request_id(&request_id)?;
    let request = registry.get(&request_id)?;
    let accepted = {
        let mut machine = request.machine.lock().await;
        machine.request_cancel()
    };
    if accepted {
        request.notify.notify_one();
    }
    Ok(CancelStreamResponse {
        request_id,
        accepted,
    })
}

#[tauri::command]
async fn get_stream_snapshot(
    request_id: String,
    registry: State<'_, StreamRegistry>,
) -> Result<StreamSnapshot, CommandError> {
    validate_stream_request_id(&request_id)?;
    let request = registry.get(&request_id)?;
    let snapshot = clone_snapshot_for_return(&request).await;
    if !json_value_uses_direct_path(&snapshot) {
        return Err(CommandError::internal(
            "stream snapshot receipt exceeded the direct IPC response budget",
        ));
    }
    Ok(snapshot)
}

#[tauri::command]
async fn wait_stream_terminal(
    request_id: String,
    registry: State<'_, StreamRegistry>,
) -> Result<StreamSnapshot, CommandError> {
    validate_stream_request_id(&request_id)?;
    let request = registry.get(&request_id)?;
    wait_for_terminal_snapshot(&request).await
}

#[tauri::command]
async fn release_stream(
    request_id: String,
    snapshot_seq: u64,
    registry: State<'_, StreamRegistry>,
) -> Result<ReleaseStreamResponse, CommandError> {
    validate_stream_request_id(&request_id)?;
    release_request(&registry, request_id, snapshot_seq).await
}

async fn clone_snapshot_for_return(request: &StreamRequest) -> StreamSnapshot {
    let machine = request.machine.lock().await;
    let snapshot = machine.snapshot();
    // This marker is deliberately set only after the terminal snapshot has been
    // fully cloned for the command return value. Registry eviction therefore
    // cannot invalidate the request while this snapshot is being assembled.
    request.record_terminal_snapshot_returned(&snapshot);
    snapshot
}

async fn wait_for_terminal_snapshot(
    request: &StreamRequest,
) -> Result<StreamSnapshot, CommandError> {
    let _waiter_lease = request.acquire_terminal_waiter()?;
    request.wait_for_terminal().await?;
    let snapshot = request.machine.lock().await.snapshot();
    if !snapshot.status.is_terminal() {
        return Err(CommandError::internal(
            "terminal signal was set before the stream reached a terminal state",
        ));
    }
    if !json_value_uses_direct_path(&snapshot) {
        return Err(CommandError::internal(
            "terminal snapshot receipt exceeded the direct IPC response budget",
        ));
    }
    Ok(snapshot)
}

async fn release_request(
    registry: &StreamRegistry,
    request_id: String,
    snapshot_seq: u64,
) -> Result<ReleaseStreamResponse, CommandError> {
    let request = registry.get(&request_id)?;
    {
        let machine = request.machine.lock().await;
        if !machine.status.is_terminal() {
            return Err(CommandError::invalid_release(
                "stream must be terminal before it can be released",
            ));
        }
        if machine.last_seq != Some(snapshot_seq) {
            return Err(CommandError::invalid_release(format!(
                "snapshotSeq {snapshot_seq} does not match current lastSeq {:?}",
                machine.last_seq
            )));
        }
        if !request.evictable.load(Ordering::Acquire) {
            return Err(CommandError::invalid_release(
                "terminal snapshot must be returned and any delivered terminal must be acknowledged before release",
            ));
        }
    }

    if !registry.remove_exact(&request_id, &request)? {
        return Err(CommandError::not_found());
    }
    Ok(ReleaseStreamResponse {
        request_id,
        released: true,
    })
}

fn split_direct_delta_text(text: &str) -> Option<Vec<String>> {
    if direct_delta_fits(text) {
        return Some(vec![text.to_owned()]);
    }

    let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
    boundaries.push(0);
    boundaries.extend(text.char_indices().skip(1).map(|(index, _)| index));
    boundaries.push(text.len());

    let mut parts = Vec::new();
    let mut start_index = 0usize;
    while start_index + 1 < boundaries.len() {
        let mut low = start_index + 1;
        let mut high = boundaries.len() - 1;
        let mut best = start_index;

        while low <= high {
            let middle = low + (high - low) / 2;
            if direct_delta_fits(&text[boundaries[start_index]..boundaries[middle]]) {
                best = middle;
                low = middle + 1;
            } else {
                high = middle - 1;
            }
        }

        if best == start_index {
            return None;
        }
        parts.push(text[boundaries[start_index]..boundaries[best]].to_owned());
        start_index = best;
    }
    Some(parts)
}

async fn run_stream(
    request: Arc<StreamRequest>,
    config: ValidatedConfig,
    channel: Channel<StreamEvent>,
) {
    let mut source_index = 0usize;

    while source_index < config.chunks.len() {
        if is_cancel_requested(&request).await {
            emit_cancelled(&request, &channel).await;
            return;
        }

        if config.fail_after_chunks == Some(source_index) {
            emit_failed(
                &request,
                &channel,
                StreamFailure::new(
                    "MOCK_FAILURE",
                    format!("deterministic failure after {source_index} chunks"),
                ),
            )
            .await;
            return;
        }

        let effective_window_ms = {
            let machine = request.machine.lock().await;
            machine.effective_batch_window_ms
        };
        let mut desired_batch_size =
            (effective_window_ms / config.chunk_interval_ms).max(1) as usize;
        desired_batch_size = desired_batch_size.min(config.chunks.len() - source_index);
        if let Some(fail_after) = config.fail_after_chunks {
            desired_batch_size =
                desired_batch_size.min(fail_after.saturating_sub(source_index).max(1));
        }

        let mut delta = String::new();
        for chunk in &config.chunks[source_index..source_index + desired_batch_size] {
            if !sleep_unless_cancelled(&request, config.chunk_interval_ms).await {
                emit_cancelled(&request, &channel).await;
                return;
            }
            delta.push_str(chunk);
        }

        let Some(fragments) = split_direct_delta_text(&delta) else {
            emit_failed(
                &request,
                &channel,
                StreamFailure::new(
                    "CHANNEL_EVENT_TOO_LARGE",
                    "a Unicode scalar could not fit the direct Channel transport budget",
                ),
            )
            .await;
            return;
        };

        for fragment in fragments {
            match wait_for_data_capacity(&request, config.ack_timeout_ms).await {
                CapacityWait::Ready => {}
                CapacityWait::Cancelled => {
                    emit_cancelled(&request, &channel).await;
                    return;
                }
                CapacityWait::TimedOut => {
                    emit_failed(
                        &request,
                        &channel,
                        StreamFailure::new(
                            "ACK_TIMEOUT",
                            format!(
                                "frontend did not free stream capacity within {} ms",
                                config.ack_timeout_ms
                            ),
                        ),
                    )
                    .await;
                    return;
                }
            }

            match emit_delta(&request, &channel, fragment).await {
                DeltaDelivery::Sent => {}
                DeltaDelivery::Cancelled => {
                    emit_cancelled(&request, &channel).await;
                    return;
                }
                DeltaDelivery::Failed => return,
            }
        }
        source_index += desired_batch_size;
    }

    if config.fail_after_chunks == Some(source_index) {
        emit_failed(
            &request,
            &channel,
            StreamFailure::new(
                "MOCK_FAILURE",
                format!("deterministic failure after {source_index} chunks"),
            ),
        )
        .await;
        return;
    }

    if matches!(
        deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
        TerminalDelivery::CancellationRequested
    ) {
        emit_cancelled(&request, &channel).await;
    }
}

async fn is_cancel_requested(request: &StreamRequest) -> bool {
    request.machine.lock().await.cancel_requested
}

async fn sleep_unless_cancelled(request: &StreamRequest, duration_ms: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(duration_ms);
    loop {
        if is_cancel_requested(request).await {
            return false;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return true;
        }
        if tokio::time::timeout(remaining, request.notify.notified())
            .await
            .is_err()
        {
            return true;
        }
    }
}

enum CapacityWait {
    Ready,
    Cancelled,
    TimedOut,
}

async fn wait_for_data_capacity(request: &StreamRequest, timeout_ms: u64) -> CapacityWait {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        {
            let mut machine = request.machine.lock().await;
            if machine.status.is_terminal() || machine.cancel_requested {
                return CapacityWait::Cancelled;
            }
            if machine.has_data_capacity() {
                return CapacityWait::Ready;
            }
            machine.apply_pressure();
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return CapacityWait::TimedOut;
        }
        if tokio::time::timeout(remaining, request.notify.notified())
            .await
            .is_err()
        {
            return CapacityWait::TimedOut;
        }
    }
}

async fn emit_cancelled(request: &Arc<StreamRequest>, channel: &Channel<StreamEvent>) {
    let _ = deliver_terminal(request, channel, TerminalTransition::Cancel).await;
}

async fn emit_failed(
    request: &Arc<StreamRequest>,
    channel: &Channel<StreamEvent>,
    failure: StreamFailure,
) {
    if matches!(
        deliver_terminal(request, channel, TerminalTransition::Fail(failure)).await,
        TerminalDelivery::CancellationRequested
    ) {
        emit_cancelled(request, channel).await;
    }
}

enum DeltaDelivery {
    Sent,
    Cancelled,
    Failed,
}

async fn emit_delta(
    request: &Arc<StreamRequest>,
    channel: &Channel<StreamEvent>,
    text: String,
) -> DeltaDelivery {
    // Keep the state lock through the synchronous Channel handoff. A cancel command
    // can therefore only return `accepted: true` before this delta is prepared or
    // after it has been delivered, never in the gap between those operations.
    let mut machine = request.machine.lock().await;
    let previous_len = machine.text.len();
    let previous_seq = machine.last_seq;
    let event = match machine.delta(text) {
        Ok(event) => event,
        Err(_) if machine.cancel_requested => return DeltaDelivery::Cancelled,
        Err(error) => {
            if machine.status == StreamStatus::Streaming {
                machine.status = StreamStatus::Failed;
                machine.error = Some(StreamFailure::new(error.code, error.message));
                request.record_control_plane_only_terminal();
            }
            return DeltaDelivery::Failed;
        }
    };

    if let Err(error) = send_direct_channel_event(channel, event) {
        machine.text.truncate(previous_len);
        machine.last_seq = previous_seq;
        machine.status = StreamStatus::Failed;
        machine.error = Some(StreamFailure::new(
            "CHANNEL_DELIVERY_FAILED",
            error.to_string(),
        ));
        request.record_control_plane_only_terminal();
        request.notify.notify_one();
        return DeltaDelivery::Failed;
    }

    DeltaDelivery::Sent
}

enum TerminalTransition {
    Complete,
    Cancel,
    Fail(StreamFailure),
}

enum TerminalDelivery {
    Sent,
    CancellationRequested,
    Failed,
}

async fn deliver_terminal(
    request: &Arc<StreamRequest>,
    channel: &Channel<StreamEvent>,
    transition: TerminalTransition,
) -> TerminalDelivery {
    // The transition and synchronous Channel handoff are one critical section.
    // A snapshot can therefore observe either the pre-terminal stream or the
    // fully delivered terminal state, never a terminal state whose event is
    // still in the process of being handed to Tauri.
    let mut machine = request.machine.lock().await;
    let previous_status = machine.status;
    let previous_seq = machine.last_seq;
    let previous_error = machine.error.clone();

    let is_cancel_transition = matches!(&transition, TerminalTransition::Cancel);
    let event = match transition {
        TerminalTransition::Complete => machine.complete(),
        TerminalTransition::Cancel => machine.cancel(),
        TerminalTransition::Fail(failure) => machine.fail(failure),
    };
    let event = match event {
        Ok(event) => event,
        Err(_)
            if !is_cancel_transition
                && machine.cancel_requested
                && machine.status == StreamStatus::Streaming =>
        {
            return TerminalDelivery::CancellationRequested;
        }
        Err(error) => {
            if !machine.status.is_terminal() {
                machine.status = StreamStatus::Failed;
                machine.error = Some(StreamFailure::new(error.code, error.message));
                request.record_control_plane_only_terminal();
            }
            return TerminalDelivery::Failed;
        }
    };
    let Some(terminal_seq) = machine.last_seq else {
        machine.status = StreamStatus::Failed;
        machine.error = Some(StreamFailure::new(
            "INTERNAL",
            "terminal transition did not publish a sequence",
        ));
        request.record_control_plane_only_terminal();
        return TerminalDelivery::Failed;
    };

    if let Err(error) = send_direct_channel_event(channel, event) {
        // Roll back the undelivered terminal transition before publishing the
        // local delivery failure. Both changes happen under the same lock, so a
        // concurrent snapshot sees only the final failed snapshot.
        machine.status = previous_status;
        machine.last_seq = previous_seq;
        machine.error = previous_error;
        machine.status = StreamStatus::Failed;
        machine.error = Some(StreamFailure::new(
            "CHANNEL_DELIVERY_FAILED",
            error.to_string(),
        ));
        request.record_control_plane_only_terminal();
        request.notify.notify_one();
        return TerminalDelivery::Failed;
    }

    request.record_terminal_delivery(terminal_seq);
    request.notify.notify_one();
    TerminalDelivery::Sent
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(StreamRegistry::default())
        .manage(HostBroker::<SystemMonotonicClock>::production())
        .invoke_handler(with_lorepia_app_commands!(generate_lorepia_handler))
        .run(tauri::generate_context!())
        .expect("error while running LorePia Channel spike");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_snapshot_text_receipt(snapshot: &StreamSnapshot, expected: &str) {
        assert_eq!(snapshot.text_bytes, expected.len());
        assert_eq!(
            snapshot.text_sha256,
            format!("{:x}", Sha256::digest(expected.as_bytes()))
        );
    }

    #[test]
    fn plugin_html_sanitizer_keeps_only_the_presentation_subset() {
        let input = concat!(
            "<!--marker--><p id='x' class='y' style='color:red' onclick='attack()'>safe ",
            "<strong data-x='1'>bold</strong><a href='javascript:attack()'>link</a></p>",
            "<script>attack()</script><style>body{display:none}</style>",
            "<svg><script>svgAttack()</script><text>svg text</text></svg>",
            "<math><mtext>math text</mtext></math>",
            "<iframe srcdoc='<script>frameAttack()</script>'>frame text</iframe>",
            "<form action='https://example.invalid'><input name='secret'>form text</form>",
            "<img src='data:text/html,attack' onerror='attack()'>",
            "<blockquote><em>quoted</em><br><code>code</code></blockquote>"
        );

        let result = sanitize_plugin_html_value(input).unwrap();

        assert_eq!(result.input_bytes, input.len());
        assert_eq!(result.output_bytes, result.html.len());
        assert!(result
            .html
            .contains("<p>safe <strong>bold</strong>link</p>"));
        assert!(result
            .html
            .contains("<blockquote><em>quoted</em><br><code>code</code></blockquote>"));
        for forbidden in [
            "<!--",
            "onclick",
            "style=",
            "class=",
            "href=",
            "javascript:",
            "data:",
            "<script",
            "attack()",
            "<style",
            "<svg",
            "svg text",
            "<math",
            "math text",
            "<iframe",
            "frame text",
            "<form",
            "form text",
            "<input",
            "<img",
        ] {
            assert!(
                !result.html.contains(forbidden),
                "sanitized HTML retained forbidden content {forbidden:?}: {}",
                result.html
            );
        }
    }

    #[test]
    fn plugin_html_sanitizer_enforces_the_utf8_byte_limit() {
        let exact_ascii_limit = "x".repeat(MAX_PLUGIN_HTML_BYTES);
        let accepted = sanitize_plugin_html_value(&exact_ascii_limit).unwrap();
        assert_eq!(accepted.input_bytes, MAX_PLUGIN_HTML_BYTES);

        let oversized_ascii = "x".repeat(MAX_PLUGIN_HTML_BYTES + 1);
        let error = sanitize_plugin_html_value(&oversized_ascii).unwrap_err();
        assert_eq!(error.code, "HTML_TOO_LARGE");

        let exact_multibyte_floor = "한".repeat(MAX_PLUGIN_HTML_BYTES / "한".len());
        assert!(exact_multibyte_floor.len() <= MAX_PLUGIN_HTML_BYTES);
        sanitize_plugin_html_value(&exact_multibyte_floor).unwrap();

        let oversized_multibyte = format!("{exact_multibyte_floor}한");
        assert!(oversized_multibyte.len() > MAX_PLUGIN_HTML_BYTES);
        assert_eq!(
            sanitize_plugin_html_value(&oversized_multibyte)
                .unwrap_err()
                .code,
            "HTML_TOO_LARGE"
        );
    }

    #[test]
    fn host_broker_executor_maps_authorized_actions_and_uses_dedicated_probe_counter() {
        HOST_BROKER_PROBE_CALLS.store(0, Ordering::SeqCst);
        HOST_BROKER_SANITIZE_CALLS.store(0, Ordering::SeqCst);

        let state = execute_host_broker_action(AuthorizedAction {
            request_id: "state-action".into(),
            module_id: "module.alpha",
            action: BrokerAction::StateRead,
        })
        .unwrap();
        assert_eq!(state.request_id, "state-action");
        assert_eq!(state.module_id, "module.alpha");
        assert_eq!(state.result, HostBrokerResult::StateRead { state: "ready" });

        let sanitized = execute_host_broker_action(AuthorizedAction {
            request_id: "sanitize-action".into(),
            module_id: "module.alpha",
            action: BrokerAction::RenderSanitize {
                html: "<p onclick='attack()'>safe<script>attack()</script></p>".into(),
            },
        })
        .unwrap();
        assert!(matches!(
            sanitized.result,
            HostBrokerResult::RenderSanitize { ref html, .. }
                if html == "<p>safe</p>"
        ));

        let probe = execute_host_broker_action(AuthorizedAction {
            request_id: "probe-action".into(),
            module_id: "module.alpha",
            action: BrokerAction::ProbeIncrement,
        })
        .unwrap();
        assert!(matches!(
            &probe.result,
            HostBrokerResult::ProbeIncrement {
                sentinel: "LOREPIA_HOST_BROKER_PROBE_REACHED",
                call_count: 1
            }
        ));
        let audit = host_broker_probe_count();
        assert_eq!(audit.probe_call_count, 1);
        assert_eq!(audit.sanitize_call_count, 1);
        assert_eq!(audit.command_surface_version, 3);
        assert_eq!(
            audit.command_names,
            [
                "ack_stream",
                "cancel_stream",
                "get_stream_snapshot",
                "host_broker_probe_count",
                "host_broker_request",
                "register_host_broker_session",
                "release_stream",
                "rotate_host_broker_session",
                "start_mock_stream",
                "wait_stream_terminal",
            ]
        );
        assert_eq!(
            audit.command_sha256,
            "679411179e22a191fe48f8fdc503c62d6d302a888aba93fe1606c9a553bc57ce"
        );

        let serialized = serde_json::to_value(probe).unwrap();
        assert_eq!(serialized["requestId"], "probe-action");
        assert_eq!(serialized["moduleId"], "module.alpha");
        assert_eq!(serialized["result"]["type"], "probe_increment");
        assert_eq!(serialized["result"]["callCount"], 1);
    }

    #[test]
    fn oversized_broker_results_fail_before_entering_tauri_response_queue() {
        let error = ensure_direct_broker_response(HostBrokerRequestResponse {
            request_id: "oversized-render".to_owned(),
            module_id: "module.alpha".to_owned(),
            result: HostBrokerResult::RenderSanitize {
                html: "x".repeat(LOREPIA_DIRECT_JSON_BUDGET_BYTES),
                input_bytes: LOREPIA_DIRECT_JSON_BUDGET_BYTES,
                output_bytes: LOREPIA_DIRECT_JSON_BUDGET_BYTES,
            },
        })
        .unwrap_err();

        assert_eq!(error.code, host_broker::BrokerErrorCode::ActionFailed);
        assert_eq!(error.request_id.as_deref(), Some("oversized-render"));
    }

    #[test]
    fn bounded_app_command_response_shapes_stay_on_the_direct_path() {
        fn assert_direct(value: &impl Serialize) {
            let len = serialized_json_len(value).expect("response must serialize");
            assert!(
                len <= LOREPIA_DIRECT_JSON_BUDGET_BYTES,
                "response was {len} bytes"
            );
        }

        let request_id = MAX_CHANNEL_REQUEST_ID.to_owned();
        let maximum_failure = StreamFailure::new("c".repeat(65), "m".repeat(513));
        assert_eq!(maximum_failure.code.len(), 64);
        assert_eq!(maximum_failure.message.len(), 512);

        assert_direct(&StartStreamResponse {
            request_id: request_id.clone(),
        });
        assert_direct(&AckStreamResponse {
            request_id: request_id.clone(),
            acknowledged_through: MAX_SEQUENCE,
            in_flight: 64,
        });
        assert_direct(&CancelStreamResponse {
            request_id: request_id.clone(),
            accepted: true,
        });
        assert_direct(&ReleaseStreamResponse {
            request_id: request_id.clone(),
            released: true,
        });
        assert_direct(&StreamSnapshot {
            request_id: request_id.clone(),
            status: StreamStatus::Failed,
            last_seq: Some(MAX_SEQUENCE),
            last_acked_seq: Some(MAX_SEQUENCE),
            in_flight: 64,
            text_bytes: 1_048_576,
            text_sha256: "f".repeat(64),
            error: Some(maximum_failure),
            batch_window_ms: MAX_BATCH_WINDOW_MS,
            effective_batch_window_ms: MAX_BATCH_WINDOW_MS,
            max_in_flight: 64,
        });
        assert_direct(&RegisterHostBrokerSessionResponse {
            outcome: RegistrationOutcome::Registered,
            generation: u64::MAX,
            module_id: "m".repeat(128),
            network_policy: "deny",
        });
        assert_direct(&RotateHostBrokerSessionResponse {
            outcome: RotationOutcome::Rotated,
            generation: u64::MAX,
            module_id: "m".repeat(128),
            network_policy: "deny",
        });
        assert_direct(&HostBrokerRequestResponse {
            request_id,
            module_id: "m".repeat(128),
            result: HostBrokerResult::StateRead { state: "ready" },
        });
        assert_direct(&host_broker_probe_count());
    }

    fn config(batch_window_ms: u64, max_in_flight: usize) -> ValidatedConfig {
        ValidatedConfig {
            batch_window_ms,
            max_in_flight,
            chunk_interval_ms: 1,
            chunks: vec!["A".into(), "B".into(), "C".into()],
            fail_after_chunks: None,
            ack_timeout_ms: 10,
        }
    }

    fn started_machine(max_in_flight: usize) -> StreamMachine {
        let mut machine = StreamMachine::new("request-1".into(), &config(24, max_in_flight));
        assert!(matches!(
            machine.start().unwrap(),
            StreamEvent::Started { seq: 0, .. }
        ));
        machine
    }

    #[test]
    fn normal_stream_is_ordered_and_has_exactly_one_terminal() {
        let mut machine = started_machine(4);
        machine.acknowledge(0).unwrap();
        let first = machine.delta("AB".into()).unwrap();
        let second = machine.delta("C".into()).unwrap();
        assert!(matches!(first, StreamEvent::Delta { seq: 1, .. }));
        assert!(matches!(second, StreamEvent::Delta { seq: 2, .. }));
        machine.acknowledge(2).unwrap();
        let terminal = machine.complete().unwrap();
        assert!(matches!(terminal, StreamEvent::Completed { seq: 3, .. }));
        assert_snapshot_text_receipt(&machine.snapshot(), "ABC");
        assert_eq!(machine.status, StreamStatus::Completed);
        assert!(machine.complete().is_err());
        assert!(machine.cancel().is_err());
        assert!(machine.fail(StreamFailure::new("X", "x")).is_err());
    }

    #[test]
    fn slow_ack_applies_pressure_without_loss() {
        let mut machine = started_machine(3);
        machine.acknowledge(0).unwrap();
        let mut received = String::new();
        if let StreamEvent::Delta { text, .. } = machine.delta("A".into()).unwrap() {
            received.push_str(&text);
        }
        if let StreamEvent::Delta { text, .. } = machine.delta("B".into()).unwrap() {
            received.push_str(&text);
        }
        assert!(!machine.has_data_capacity());
        machine.apply_pressure();
        assert_eq!(machine.effective_batch_window_ms, 50);
        assert!(machine.delta("lost".into()).is_err());
        machine.acknowledge(1).unwrap();
        if let StreamEvent::Delta { text, .. } = machine.delta("C".into()).unwrap() {
            received.push_str(&text);
        }
        assert_eq!(received, "ABC");
        assert_eq!(machine.text, "ABC");
        assert!(machine.in_flight() <= machine.max_in_flight);
    }

    #[test]
    fn cancellation_preserves_exact_snapshot_and_rejects_late_data() {
        let mut machine = started_machine(3);
        machine.acknowledge(0).unwrap();
        machine.delta("partial".into()).unwrap();
        machine.acknowledge(1).unwrap();
        assert!(machine.request_cancel());
        assert!(machine.delta(" late".into()).is_err());
        assert!(machine.complete().is_err());
        let terminal = machine.cancel().unwrap();
        assert!(matches!(terminal, StreamEvent::Cancelled { seq: 2, .. }));
        assert_snapshot_text_receipt(&machine.snapshot(), "partial");
        assert!(machine.delta(" later".into()).is_err());
        assert!(!machine.request_cancel());
    }

    #[test]
    fn deterministic_failure_is_structured_and_terminal() {
        let mut machine = started_machine(3);
        machine.acknowledge(0).unwrap();
        machine.delta("AB".into()).unwrap();
        machine.acknowledge(1).unwrap();
        let failure = StreamFailure::new("MOCK_FAILURE", "after 2 chunks");
        let terminal = machine.fail(failure.clone()).unwrap();
        assert!(matches!(terminal, StreamEvent::Failed { seq: 2, error, .. } if error == failure));
        let snapshot = machine.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_snapshot_text_receipt(&snapshot, "AB");
        assert_eq!(snapshot.error, Some(failure));
        assert!(machine.complete().is_err());
    }

    #[test]
    fn invalid_config_is_rejected_at_the_boundary() {
        let cases = [
            StreamConfig {
                batch_window_ms: Some(15),
                ..Default::default()
            },
            StreamConfig {
                batch_window_ms: Some(51),
                ..Default::default()
            },
            StreamConfig {
                max_in_flight: Some(1),
                ..Default::default()
            },
            StreamConfig {
                chunks: Some(vec![]),
                ..Default::default()
            },
            StreamConfig {
                chunks: Some(vec![String::new()]),
                ..Default::default()
            },
            StreamConfig {
                chunks: Some(vec!["one".into()]),
                fail_after_chunks: Some(2),
                ..Default::default()
            },
            StreamConfig {
                ack_timeout_ms: Some(9),
                ..Default::default()
            },
        ];

        for invalid in cases {
            let error = ValidatedConfig::try_from(invalid).unwrap_err();
            assert_eq!(error.code, "INVALID_CONFIG");
        }

        for boundary in [16, 50] {
            assert!(ValidatedConfig::try_from(StreamConfig {
                batch_window_ms: Some(boundary),
                ..Default::default()
            })
            .is_ok());
        }

        let defaults = ValidatedConfig::try_from(StreamConfig::default()).unwrap();
        assert!(defaults.chunk_interval_ms < defaults.batch_window_ms);
    }

    #[test]
    fn future_ack_is_rejected_without_mutating_state() {
        let mut machine = started_machine(2);
        let error = machine.acknowledge(1).unwrap_err();
        assert_eq!(error.code, "INVALID_ACK");
        assert_eq!(machine.last_acked_seq, None);
        machine.acknowledge(0).unwrap();
        machine.acknowledge(0).unwrap();
        assert_eq!(machine.last_acked_seq, Some(0));
    }

    #[test]
    fn queued_snapshot_has_no_phantom_sequence_and_rejects_early_ack() {
        let config = config(24, 2);
        let mut machine = StreamMachine::new("request-queued".into(), &config);

        let snapshot = machine.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Queued);
        assert_eq!(snapshot.last_seq, None);
        assert_eq!(snapshot.last_acked_seq, None);
        assert_eq!(snapshot.in_flight, 0);
        assert_snapshot_text_receipt(&snapshot, "");

        let error = machine.acknowledge(0).unwrap_err();
        assert_eq!(error.code, "INVALID_ACK");
        assert_eq!(machine.snapshot(), snapshot);
    }

    #[test]
    fn sequence_limit_allows_the_last_safe_value_then_fails_without_mutation() {
        let mut machine = started_machine(2);
        machine.last_seq = Some(MAX_SEQUENCE - 1);
        machine.last_acked_seq = Some(MAX_SEQUENCE - 1);

        let final_safe_event = machine.delta("A".into()).unwrap();
        assert!(matches!(
            final_safe_event,
            StreamEvent::Delta {
                seq: MAX_SEQUENCE,
                ref text,
                ..
            } if text == "A"
        ));
        machine.last_acked_seq = Some(MAX_SEQUENCE);
        let before = machine.snapshot();

        let error = machine.delta("B".into()).unwrap_err();
        assert_eq!(error.code, "SEQUENCE_EXHAUSTED");
        assert_eq!(machine.snapshot(), before);
    }

    #[test]
    fn request_id_counter_never_wraps_or_reuses_an_id() {
        let registry = StreamRegistry::default();
        registry.next_id.store(u64::MAX - 1, Ordering::Relaxed);

        assert_eq!(
            registry.next_request_id().unwrap(),
            "m1-channel-ffffffffffffffff"
        );
        let error = registry.next_request_id().unwrap_err();
        assert_eq!(error.code, "INTERNAL");
        assert!(error.message.contains("request ID space exhausted"));
        assert_eq!(registry.next_id.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn stream_command_request_ids_are_exact_and_errors_never_reflect_input() {
        for valid in [
            "m1-channel-0000000000000001",
            "m1-channel-0123456789abcdef",
            MAX_CHANNEL_REQUEST_ID,
        ] {
            validate_stream_request_id(valid).unwrap();
        }

        let oversized = format!("m1-channel-{}", "a".repeat(1_048_576));
        for invalid in [
            "",
            "m1-channel-",
            "m1-channel-000000000000001",
            "m1-channel-00000000000000000",
            "m1-channel-0123456789abcdeF",
            "m1-channel-0123456789abcdeg",
            "m1-channel-0123456789abcde한",
            oversized.as_str(),
        ] {
            let error = validate_stream_request_id(invalid).unwrap_err();
            assert_eq!(error.code, "INVALID_REQUEST_ID");
            assert_eq!(
                error.message,
                "requestId must use the server-issued stream identifier format"
            );
            if !invalid.is_empty() {
                assert!(!error.message.contains(invalid));
            }
            assert!(json_value_uses_direct_path(&error));
        }

        let missing = CommandError::not_found();
        assert_eq!(missing.message, "stream request was not found");
        assert!(json_value_uses_direct_path(&missing));
    }

    #[test]
    fn serialized_events_match_the_typescript_camel_case_contract() {
        let values = [
            serde_json::to_value(StreamEvent::Started {
                request_id: "r1".into(),
                seq: 0,
                batch_window_ms: 24,
                max_in_flight: 3,
            })
            .unwrap(),
            serde_json::to_value(StreamEvent::Delta {
                request_id: "r1".into(),
                seq: 1,
                text: "A".into(),
            })
            .unwrap(),
            serde_json::to_value(StreamEvent::Cancelled {
                request_id: "r1".into(),
                seq: 2,
            })
            .unwrap(),
            serde_json::to_value(StreamEvent::Failed {
                request_id: "r1".into(),
                seq: 2,
                error: StreamFailure::new("MOCK_FAILURE", "expected"),
            })
            .unwrap(),
        ];

        assert_eq!(values[0]["type"], "started");
        assert_eq!(values[0]["requestId"], "r1");
        assert_eq!(values[0]["batchWindowMs"], 24);
        assert_eq!(values[0]["maxInFlight"], 3);
        assert_eq!(values[1]["type"], "delta");
        assert_eq!(values[2]["type"], "cancelled");
        assert!(values[2].get("partialText").is_none());
        assert!(values[2].get("text").is_none());
        assert_eq!(values[3]["type"], "failed");
        assert!(values[3].get("partialText").is_none());
        assert!(values[3].get("text").is_none());
        assert_eq!(values[3]["error"]["code"], "MOCK_FAILURE");

        for value in values {
            let object = value.as_object().unwrap();
            assert!(!object.contains_key("request_id"));
            assert!(!object.contains_key("batch_window_ms"));
            assert!(!object.contains_key("partial_text"));
        }

        let snapshot = serde_json::to_value(started_machine(2).snapshot()).unwrap();
        assert_eq!(snapshot["status"], "streaming");
        assert_eq!(snapshot["lastAckedSeq"], serde_json::Value::Null);
        assert_eq!(snapshot["effectiveBatchWindowMs"], 24);
        assert_eq!(snapshot["textBytes"], 0);
        assert_eq!(
            snapshot["textSha256"],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(snapshot.get("text").is_none());
        assert!(snapshot.get("last_acked_seq").is_none());
    }

    #[test]
    fn total_chunk_byte_limit_is_enforced() {
        let invalid = StreamConfig {
            chunks: Some(vec!["x".repeat(2_048); 513]),
            ..Default::default()
        };
        let error = ValidatedConfig::try_from(invalid).unwrap_err();
        assert_eq!(error.code, "INVALID_CONFIG");
        assert!(error.message.contains("total"));
    }

    #[test]
    fn every_channel_event_is_forced_below_tauri_fetch_queue_threshold() {
        let empty_delta = StreamEvent::Delta {
            request_id: MAX_CHANNEL_REQUEST_ID.to_owned(),
            seq: MAX_SEQUENCE,
            text: String::new(),
        };
        let overhead = serialized_channel_event_len(&empty_delta).unwrap();
        let largest_ascii_text = "x".repeat(
            LOREPIA_DIRECT_JSON_BUDGET_BYTES
                .checked_sub(overhead)
                .unwrap(),
        );

        assert!(direct_delta_fits(&largest_ascii_text));
        assert!(!direct_delta_fits(&format!("{largest_ascii_text}x")));

        let config = ValidatedConfig::try_from(StreamConfig {
            chunks: Some(vec![format!("{largest_ascii_text}x")]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(config.chunks.len(), 1);

        let terminal_events = [
            StreamEvent::Completed {
                request_id: MAX_CHANNEL_REQUEST_ID.to_owned(),
                seq: MAX_SEQUENCE,
            },
            StreamEvent::Cancelled {
                request_id: MAX_CHANNEL_REQUEST_ID.to_owned(),
                seq: MAX_SEQUENCE,
            },
            StreamEvent::Failed {
                request_id: MAX_CHANNEL_REQUEST_ID.to_owned(),
                seq: MAX_SEQUENCE,
                error: StreamFailure::new("MOCK_FAILURE", "bounded fixture failure"),
            },
        ];
        assert!(terminal_events.iter().all(channel_event_uses_direct_path));
    }

    #[test]
    fn delta_fragmentation_preserves_ascii_unicode_and_json_escaping_exactly() {
        let samples = [
            "x".repeat(16_384),
            "한글🙂".repeat(1_500),
            "\0\n\r\t\u{0008}\u{000c}\"\\".repeat(1_000),
        ];

        for original in samples {
            let parts = split_direct_delta_text(&original).unwrap();
            assert!(parts.len() > 1);
            assert!(parts.iter().all(|part| direct_delta_fits(part)));
            assert_eq!(parts.concat().as_bytes(), original.as_bytes());
            assert!(parts.iter().all(|part| part.is_char_boundary(part.len())));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn real_channel_runner_preserves_order_and_no_loss() {
        use tauri::ipc::InvokeResponseBody;

        let config = config(24, 2);
        let mut machine = StreamMachine::new("request-channel".into(), &config);
        let started = machine.start().unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let captured = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let callback_request = Arc::clone(&request);
        let callback_captured = Arc::clone(&captured);

        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            let value: serde_json::Value = serde_json::from_str(&json)?;
            let seq = value["seq"]
                .as_u64()
                .ok_or_else(|| std::io::Error::other("missing seq"))?;
            callback_captured.lock().unwrap().push(value);

            let ack_request = Arc::clone(&callback_request);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                ack_request.machine.lock().await.acknowledge(seq).unwrap();
                ack_request.notify.notify_one();
            });
            Ok(())
        });

        channel.send(started).unwrap();
        run_stream(Arc::clone(&request), config, channel).await;
        tokio::time::sleep(Duration::from_millis(10)).await;

        let events = captured.lock().unwrap().clone();
        assert_eq!(events.first().unwrap()["type"], "started");
        assert_eq!(events.last().unwrap()["type"], "completed");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event["type"].as_str(),
                    Some("completed" | "cancelled" | "failed")
                ))
                .count(),
            1
        );
        let received: String = events
            .iter()
            .filter(|event| event["type"] == "delta")
            .map(|event| event["text"].as_str().unwrap())
            .collect();
        assert_eq!(received, "ABC");

        let snapshot = request.machine.lock().await.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Completed);
        assert_snapshot_text_receipt(&snapshot, "ABC");
        assert!((snapshot.batch_window_ms..=MAX_BATCH_WINDOW_MS)
            .contains(&snapshot.effective_batch_window_ms));
        assert_ne!(
            request.terminal_seq.load(Ordering::Acquire),
            NO_TERMINAL_SEQ
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn one_mib_stream_never_uses_the_tauri_channel_fetch_queue() {
        use tauri::ipc::InvokeResponseBody;

        let chunks = vec!["x".repeat(16_384); 64];
        let expected = "x".repeat(1_048_576);
        let config = ValidatedConfig::try_from(StreamConfig {
            batch_window_ms: Some(16),
            max_in_flight: Some(64),
            chunk_interval_ms: Some(1),
            chunks: Some(chunks),
            ack_timeout_ms: Some(100),
            ..Default::default()
        })
        .unwrap();
        let mut machine = StreamMachine::new("request-one-mib".into(), &config);
        let started = machine.start().unwrap();
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let received = Arc::new(Mutex::new(String::with_capacity(expected.len())));
        let sequences = Arc::new(Mutex::new(Vec::<u64>::new()));
        let callback_request = Arc::clone(&request);
        let callback_received = Arc::clone(&received);
        let callback_sequences = Arc::clone(&sequences);

        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            if json.len() > LOREPIA_DIRECT_JSON_BUDGET_BYTES {
                return Err(std::io::Error::other("event exceeded direct budget").into());
            }
            let value: serde_json::Value = serde_json::from_str(&json)?;
            let seq = value["seq"]
                .as_u64()
                .ok_or_else(|| std::io::Error::other("missing seq"))?;
            callback_sequences.lock().unwrap().push(seq);
            if value["type"] == "delta" {
                callback_received
                    .lock()
                    .unwrap()
                    .push_str(value["text"].as_str().unwrap());
            }

            let ack_request = Arc::clone(&callback_request);
            tokio::spawn(async move {
                let acknowledged_through = {
                    let mut machine = ack_request.machine.lock().await;
                    machine.acknowledge(seq).unwrap()
                };
                ack_request.record_acknowledged_through(acknowledged_through);
                ack_request.notify.notify_one();
            });
            Ok(())
        });

        send_direct_channel_event(&channel, started).unwrap();
        // This is a transport-integrity regression, not a performance gate.
        // Hosted Windows debug runners compile and execute several spike jobs
        // concurrently, so keep a bounded but scheduling-tolerant deadline.
        tokio::time::timeout(
            Duration::from_secs(30),
            run_stream(Arc::clone(&request), config, channel),
        )
        .await
        .expect("one MiB stream should complete without queue fallback");
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert_eq!(received.lock().unwrap().as_bytes(), expected.as_bytes());
        {
            let sequences = sequences.lock().unwrap();
            assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
            assert!(sequences.len() > 2);
        }
        let snapshot = request.machine.lock().await.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Completed);
        assert_snapshot_text_receipt(&snapshot, &expected);
        assert!(json_value_uses_direct_path(&snapshot));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_delivery_failure_is_visible_in_snapshot() {
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let channel = Channel::new(|_| Err(std::io::Error::other("closed channel").into()));

        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Failed
        ));
        let snapshot = request.machine.lock().await.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.last_seq, Some(0));
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code.as_str()),
            Some("CHANNEL_DELIVERY_FAILED")
        );
        assert!(snapshot.error.unwrap().message.contains("closed channel"));
        assert_eq!(
            request.terminal_seq.load(Ordering::Acquire),
            NO_TERMINAL_SEQ
        );
        assert!(request.control_plane_only_terminal.load(Ordering::Acquire));
        assert!(!request.evictable.load(Ordering::Acquire));

        let returned_snapshot = clone_snapshot_for_return(&request).await;
        assert_eq!(returned_snapshot.status, StreamStatus::Failed);
        assert!(request.evictable.load(Ordering::Acquire));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delta_delivery_failure_is_releasable_only_after_snapshot_return() {
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let channel = Channel::new(|_| Err(std::io::Error::other("closed channel").into()));

        assert!(matches!(
            emit_delta(&request, &channel, "undelivered".into()).await,
            DeltaDelivery::Failed
        ));
        let snapshot = request.machine.lock().await.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.last_seq, Some(0));
        assert_snapshot_text_receipt(&snapshot, "");
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code.as_str()),
            Some("CHANNEL_DELIVERY_FAILED")
        );
        assert_eq!(
            request.terminal_seq.load(Ordering::Acquire),
            NO_TERMINAL_SEQ
        );
        assert!(request.control_plane_only_terminal.load(Ordering::Acquire));
        assert!(!request.evictable.load(Ordering::Acquire));

        let returned_snapshot = clone_snapshot_for_return(&request).await;
        assert_eq!(returned_snapshot.status, StreamStatus::Failed);
        assert!(request.evictable.load(Ordering::Acquire));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unacked_started_event_still_allows_one_cancelled_terminal() {
        use tauri::ipc::InvokeResponseBody;

        let config = config(24, 2);
        let mut machine = StreamMachine::new("request-cancel-reserve".into(), &config);
        let started = machine.start().unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let captured = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let callback_captured = Arc::clone(&captured);
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            callback_captured
                .lock()
                .unwrap()
                .push(serde_json::from_str(&json)?);
            Ok(())
        });

        channel.send(started).unwrap();
        assert!(request.machine.lock().await.request_cancel());
        run_stream(Arc::clone(&request), config, channel).await;

        let events = captured.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "started");
        assert_eq!(events[1]["type"], "cancelled");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event["type"].as_str(),
                    Some("completed" | "cancelled" | "failed")
                ))
                .count(),
            1
        );

        let snapshot = request.machine.lock().await.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Cancelled);
        assert_eq!(snapshot.last_seq, Some(1));
        assert_eq!(snapshot.last_acked_seq, None);
        assert_eq!(snapshot.in_flight, 2);
        assert!(snapshot.in_flight <= snapshot.max_in_flight);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_and_failed_terminals_both_use_the_reserved_slot() {
        use tauri::ipc::InvokeResponseBody;

        let completion_config = config(24, 2);
        let mut completion_machine =
            StreamMachine::new("request-complete-reserve".into(), &completion_config);
        completion_machine.start().unwrap();
        completion_machine.acknowledge(0).unwrap();
        completion_machine.delta("A".into()).unwrap();
        let completion_request = Arc::new(StreamRequest::new(completion_machine));
        let completed_events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let completed_capture = Arc::clone(&completed_events);
        let completion_channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            completed_capture
                .lock()
                .unwrap()
                .push(serde_json::from_str(&json)?);
            Ok(())
        });

        assert!(matches!(
            deliver_terminal(
                &completion_request,
                &completion_channel,
                TerminalTransition::Complete
            )
            .await,
            TerminalDelivery::Sent
        ));
        let completion_snapshot = completion_request.machine.lock().await.snapshot();
        assert_eq!(completion_snapshot.status, StreamStatus::Completed);
        assert_eq!(completion_snapshot.in_flight, 2);
        assert_eq!(completed_events.lock().unwrap()[0]["type"], "completed");

        let mut failure_config = config(24, 2);
        failure_config.fail_after_chunks = Some(0);
        let mut failure_machine =
            StreamMachine::new("request-fail-reserve".into(), &failure_config);
        let started = failure_machine.start().unwrap();
        let failure_request = Arc::new(StreamRequest::new(failure_machine));
        let failed_events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let failed_capture = Arc::clone(&failed_events);
        let failure_channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            failed_capture
                .lock()
                .unwrap()
                .push(serde_json::from_str(&json)?);
            Ok(())
        });

        failure_channel.send(started).unwrap();
        run_stream(
            Arc::clone(&failure_request),
            failure_config,
            failure_channel,
        )
        .await;
        let failure_snapshot = failure_request.machine.lock().await.snapshot();
        assert_eq!(failure_snapshot.status, StreamStatus::Failed);
        assert_eq!(failure_snapshot.in_flight, 2);
        let failed_events = failed_events.lock().unwrap();
        assert_eq!(failed_events[0]["type"], "started");
        assert_eq!(failed_events[1]["type"], "failed");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_wait_wakes_early_and_late_subscribers_without_marking_cleanup() {
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let waiting_request = Arc::clone(&request);
        let waiter =
            tokio::spawn(
                async move { wait_for_terminal_snapshot(&waiting_request).await.unwrap() },
            );

        while !request.terminal_waiter_active.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        let duplicate = wait_for_terminal_snapshot(&request).await.unwrap_err();
        assert_eq!(duplicate.code, "TERMINAL_WAITER_BUSY");
        assert!(json_value_uses_direct_path(&duplicate));

        let channel = Channel::new(|_| Ok(()));
        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));

        let early = waiter.await.unwrap();
        assert!(!request.terminal_waiter_active.load(Ordering::Acquire));
        let late = wait_for_terminal_snapshot(&request).await.unwrap();
        assert_eq!(early, late);
        assert_eq!(early.status, StreamStatus::Completed);
        assert_eq!(early.last_seq, Some(1));
        assert!(json_value_uses_direct_path(&early));
        assert!(!request.terminal_snapshot_returned.load(Ordering::Acquire));
        assert!(!request.evictable.load(Ordering::Acquire));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sequence_exhaustion_publishes_a_control_plane_terminal_receipt() {
        let mut machine = started_machine(2);
        machine.last_seq = Some(MAX_SEQUENCE);
        machine.last_acked_seq = Some(MAX_SEQUENCE);
        let request = Arc::new(StreamRequest::new(machine));
        let channel = Channel::new(|_| Ok(()));

        assert!(matches!(
            emit_delta(&request, &channel, "must-not-append".into()).await,
            DeltaDelivery::Failed
        ));
        let snapshot = wait_for_terminal_snapshot(&request).await.unwrap();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.last_seq, Some(MAX_SEQUENCE));
        assert_eq!(snapshot.last_acked_seq, Some(MAX_SEQUENCE));
        assert_snapshot_text_receipt(&snapshot, "");
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code.as_str()),
            Some("SEQUENCE_EXHAUSTED")
        );
        assert!(request.control_plane_only_terminal.load(Ordering::Acquire));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_at_sequence_limit_becomes_releasable_control_plane_failure() {
        let request_id = "m1-channel-0000000000000001";
        let registry = StreamRegistry::default();
        let mut machine = started_machine(2);
        machine.request_id = request_id.into();
        machine.last_seq = Some(MAX_SEQUENCE);
        machine.last_acked_seq = Some(MAX_SEQUENCE);
        assert!(machine.request_cancel());
        let request = Arc::new(StreamRequest::new(machine));
        registry
            .insert(request_id.into(), Arc::clone(&request))
            .unwrap();
        let channel = Channel::new(|_| Ok(()));

        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Cancel).await,
            TerminalDelivery::Failed
        ));
        let waited = wait_for_terminal_snapshot(&request).await.unwrap();
        assert_eq!(waited.status, StreamStatus::Failed);
        assert_eq!(waited.last_seq, Some(MAX_SEQUENCE));
        assert_eq!(waited.last_acked_seq, Some(MAX_SEQUENCE));
        assert_eq!(
            waited.error.as_ref().map(|error| error.code.as_str()),
            Some("SEQUENCE_EXHAUSTED")
        );
        assert!(request.control_plane_only_terminal.load(Ordering::Acquire));
        assert_eq!(
            request.terminal_seq.load(Ordering::Acquire),
            NO_TERMINAL_SEQ
        );

        let final_snapshot = clone_snapshot_for_return(&request).await;
        assert_eq!(final_snapshot, waited);
        let released = release_request(&registry, request_id.into(), MAX_SEQUENCE)
            .await
            .unwrap();
        assert!(released.released);
        assert!(matches!(
            registry.get(request_id),
            Err(CommandError { ref code, .. }) if code == "STREAM_NOT_FOUND"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn release_requires_terminal_ack_final_snapshot_and_exact_sequence() {
        let registry = StreamRegistry::default();
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        registry
            .insert("request-release".into(), Arc::clone(&request))
            .unwrap();

        let nonterminal = release_request(&registry, "request-release".into(), 0)
            .await
            .unwrap_err();
        assert_eq!(nonterminal.code, "INVALID_RELEASE");

        let channel = Channel::new(|_| Ok(()));
        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));
        let waited = wait_for_terminal_snapshot(&request).await.unwrap();
        assert_eq!(waited.last_seq, Some(1));

        let before_ack = release_request(&registry, "request-release".into(), 1)
            .await
            .unwrap_err();
        assert_eq!(before_ack.code, "INVALID_RELEASE");

        let acknowledged_through = request.machine.lock().await.acknowledge(1).unwrap();
        request.record_acknowledged_through(acknowledged_through);
        let before_snapshot = release_request(&registry, "request-release".into(), 1)
            .await
            .unwrap_err();
        assert_eq!(before_snapshot.code, "INVALID_RELEASE");

        let final_snapshot = clone_snapshot_for_return(&request).await;
        assert_eq!(final_snapshot.last_seq, Some(1));
        let wrong_sequence = release_request(&registry, "request-release".into(), 0)
            .await
            .unwrap_err();
        assert_eq!(wrong_sequence.code, "INVALID_RELEASE");
        assert!(registry.get("request-release").is_ok());

        let released = release_request(&registry, "request-release".into(), 1)
            .await
            .unwrap();
        assert_eq!(
            released,
            ReleaseStreamResponse {
                request_id: "request-release".into(),
                released: true,
            }
        );
        assert!(matches!(
            registry.get("request-release"),
            Err(CommandError { ref code, .. }) if code == "STREAM_NOT_FOUND"
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_release_has_exactly_one_winner() {
        let registry = Arc::new(StreamRegistry::default());
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        registry
            .insert("request-release-race".into(), Arc::clone(&request))
            .unwrap();
        let channel = Channel::new(|_| Ok(()));
        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));
        let acknowledged_through = request.machine.lock().await.acknowledge(1).unwrap();
        request.record_acknowledged_through(acknowledged_through);
        clone_snapshot_for_return(&request).await;

        let first_registry = Arc::clone(&registry);
        let first = tokio::spawn(async move {
            release_request(&first_registry, "request-release-race".into(), 1).await
        });
        let second_registry = Arc::clone(&registry);
        let second = tokio::spawn(async move {
            release_request(&second_registry, "request-release-race".into(), 1).await
        });
        let results = [first.await.unwrap(), second.await.unwrap()];

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(error) if error.code == "STREAM_NOT_FOUND"))
                .count(),
            1
        );
    }

    #[derive(Clone, Copy)]
    enum DeliveryFailurePoint {
        Delta,
        Terminal,
    }

    async fn assert_registry_recovers_from_delivery_failure(point: DeliveryFailurePoint) {
        let registry = StreamRegistry::default();
        let config = config(24, 2);
        let mut first_machine = StreamMachine::new("request-000".into(), &config);
        first_machine.start().unwrap();
        first_machine.acknowledge(0).unwrap();
        let first = Arc::new(StreamRequest::new(first_machine));
        registry
            .insert("request-000".into(), Arc::clone(&first))
            .unwrap();

        for index in 1..MAX_REQUESTS {
            let request_id = format!("request-{index:03}");
            let request = Arc::new(StreamRequest::new(StreamMachine::new(
                request_id.clone(),
                &config,
            )));
            registry.insert(request_id, request).unwrap();
        }

        let closed_channel = Channel::new(|_| Err(std::io::Error::other("closed channel").into()));
        match point {
            DeliveryFailurePoint::Delta => assert!(matches!(
                emit_delta(&first, &closed_channel, "undelivered".into()).await,
                DeltaDelivery::Failed
            )),
            DeliveryFailurePoint::Terminal => assert!(matches!(
                deliver_terminal(&first, &closed_channel, TerminalTransition::Complete).await,
                TerminalDelivery::Failed
            )),
        }

        assert!(first.control_plane_only_terminal.load(Ordering::Acquire));
        assert!(!first.evictable.load(Ordering::Acquire));
        let candidate = Arc::new(StreamRequest::new(StreamMachine::new(
            "request-new".into(),
            &config,
        )));
        let before_snapshot = registry
            .insert("request-new".into(), Arc::clone(&candidate))
            .unwrap_err();
        assert_eq!(before_snapshot.code, "REGISTRY_CAPACITY");

        let failure_snapshot = clone_snapshot_for_return(&first).await;
        assert_eq!(failure_snapshot.status, StreamStatus::Failed);
        assert_eq!(
            failure_snapshot
                .error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("CHANNEL_DELIVERY_FAILED")
        );
        assert!(first.evictable.load(Ordering::Acquire));

        let after_snapshot = registry
            .insert("request-new".into(), Arc::clone(&candidate))
            .unwrap_err();
        assert_eq!(after_snapshot.code, "REGISTRY_CAPACITY");
        assert!(registry.get("request-000").is_ok());

        let released = release_request(&registry, "request-000".into(), 0)
            .await
            .unwrap();
        assert!(released.released);
        registry.insert("request-new".into(), candidate).unwrap();
        assert!(matches!(
            registry.get("request-000"),
            Err(CommandError { ref code, .. }) if code == "STREAM_NOT_FOUND"
        ));
        assert!(registry.get("request-new").is_ok());
        assert_eq!(registry.inner.lock().unwrap().requests.len(), MAX_REQUESTS);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registry_recovers_at_128_after_delta_delivery_failure_release() {
        assert_registry_recovers_from_delivery_failure(DeliveryFailurePoint::Delta).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registry_recovers_at_128_after_terminal_delivery_failure_release() {
        assert_registry_recovers_from_delivery_failure(DeliveryFailurePoint::Terminal).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registry_keeps_releasable_terminal_until_explicit_release() {
        let registry = StreamRegistry::default();
        let config = config(24, 2);

        let mut first_machine = StreamMachine::new("request-000".into(), &config);
        first_machine.start().unwrap();
        first_machine.acknowledge(0).unwrap();
        let first = Arc::new(StreamRequest::new(first_machine));
        registry
            .insert("request-000".into(), Arc::clone(&first))
            .unwrap();

        for index in 1..MAX_REQUESTS {
            let request_id = format!("request-{index:03}");
            let request = Arc::new(StreamRequest::new(StreamMachine::new(
                request_id.clone(),
                &config,
            )));
            registry.insert(request_id, request).unwrap();
        }

        let channel = Channel::new(|_| Ok(()));
        assert!(matches!(
            deliver_terminal(&first, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));

        let candidate = Arc::new(StreamRequest::new(StreamMachine::new(
            "request-new".into(),
            &config,
        )));
        let before_ack = registry
            .insert("request-new".into(), Arc::clone(&candidate))
            .unwrap_err();
        assert_eq!(before_ack.code, "REGISTRY_CAPACITY");

        let acknowledged_through = {
            let mut machine = first.machine.lock().await;
            machine.acknowledge(1).unwrap()
        };
        first.record_acknowledged_through(acknowledged_through);
        assert!(first.terminal_acked.load(Ordering::Acquire));
        assert!(!first.evictable.load(Ordering::Acquire));

        let before_snapshot = registry
            .insert("request-new".into(), Arc::clone(&candidate))
            .unwrap_err();
        assert_eq!(before_snapshot.code, "REGISTRY_CAPACITY");

        let terminal_snapshot = clone_snapshot_for_return(&first).await;
        assert_eq!(terminal_snapshot.status, StreamStatus::Completed);
        assert!(first.terminal_snapshot_returned.load(Ordering::Acquire));
        assert!(first.evictable.load(Ordering::Acquire));

        let before_release = registry
            .insert("request-new".into(), Arc::clone(&candidate))
            .unwrap_err();
        assert_eq!(before_release.code, "REGISTRY_CAPACITY");
        assert!(registry.get("request-000").is_ok());

        let released = release_request(&registry, "request-000".into(), 1)
            .await
            .unwrap();
        assert!(released.released);
        registry.insert("request-new".into(), candidate).unwrap();
        assert!(matches!(
            registry.get("request-000"),
            Err(CommandError { ref code, .. }) if code == "STREAM_NOT_FOUND"
        ));
        assert!(registry.get("request-new").is_ok());
        assert_eq!(registry.inner.lock().unwrap().requests.len(), MAX_REQUESTS);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn expired_terminal_ttl_recovers_capacity_from_a_dead_webview() {
        let registry = StreamRegistry::default();
        let config = config(24, 2);
        let mut first_machine = StreamMachine::new("request-000".into(), &config);
        first_machine.start().unwrap();
        let first = Arc::new(StreamRequest::new(first_machine));
        registry
            .insert("request-000".into(), Arc::clone(&first))
            .unwrap();
        for index in 1..MAX_REQUESTS {
            let request_id = format!("request-{index:03}");
            registry
                .insert(
                    request_id.clone(),
                    Arc::new(StreamRequest::new(StreamMachine::new(request_id, &config))),
                )
                .unwrap();
        }

        let channel = Channel::new(|_| Ok(()));
        assert!(matches!(
            deliver_terminal(&first, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));
        let waited = wait_for_terminal_snapshot(&first).await.unwrap();
        assert_eq!(waited.status, StreamStatus::Completed);
        assert!(!first.terminal_waiter_active.load(Ordering::Acquire));
        let candidate = Arc::new(StreamRequest::new(StreamMachine::new(
            "request-new".into(),
            &config,
        )));
        assert_eq!(
            registry
                .insert("request-new".into(), Arc::clone(&candidate))
                .unwrap_err()
                .code,
            "REGISTRY_CAPACITY"
        );

        let expired_at = Instant::now()
            .checked_sub(TERMINAL_RETENTION_TTL + Duration::from_millis(1))
            .unwrap();
        *first.terminal_reached_at.lock().unwrap() = Some(expired_at);
        registry.insert("request-new".into(), candidate).unwrap();

        assert!(matches!(
            registry.get("request-000"),
            Err(CommandError { ref code, .. }) if code == "STREAM_NOT_FOUND"
        ));
        assert!(registry.get("request-new").is_ok());
        assert_eq!(registry.inner.lock().unwrap().requests.len(), MAX_REQUESTS);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_cannot_observe_terminal_state_during_channel_handoff() {
        use std::sync::mpsc::sync_channel;

        let config = config(24, 2);
        let mut machine = StreamMachine::new("request-atomic-terminal".into(), &config);
        machine.start().unwrap();
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));

        let (entered_tx, entered_rx) = sync_channel::<()>(1);
        let (release_tx, release_rx) = sync_channel::<()>(1);
        let release_rx = Arc::new(Mutex::new(release_rx));
        let callback_release = Arc::clone(&release_rx);
        let channel = Channel::new(move |_| {
            entered_tx
                .send(())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            callback_release
                .lock()
                .unwrap()
                .recv()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(())
        });

        let delivery_request = Arc::clone(&request);
        let delivery = tokio::spawn(async move {
            deliver_terminal(&delivery_request, &channel, TerminalTransition::Complete).await
        });

        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), request.machine.lock())
                .await
                .is_err()
        );
        release_tx.send(()).unwrap();
        assert!(matches!(delivery.await.unwrap(), TerminalDelivery::Sent));

        let snapshot = clone_snapshot_for_return(&request).await;
        assert_eq!(snapshot.status, StreamStatus::Completed);
        assert_eq!(snapshot.last_seq, Some(1));
        assert_eq!(request.terminal_seq.load(Ordering::Acquire), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pressure_expands_later_delta_batches_from_24_to_50_ms() {
        use tauri::ipc::InvokeResponseBody;

        let chunks = vec!["x".to_owned(); 18];
        let config = ValidatedConfig {
            batch_window_ms: 24,
            max_in_flight: 2,
            chunk_interval_ms: 8,
            chunks: chunks.clone(),
            fail_after_chunks: None,
            ack_timeout_ms: 10,
        };
        let mut machine = StreamMachine::new("request-adaptive-batch".into(), &config);
        let started = machine.start().unwrap();
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let captured = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let callback_request = Arc::clone(&request);
        let callback_captured = Arc::clone(&captured);
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            let value: serde_json::Value = serde_json::from_str(&json)?;
            let seq = value["seq"]
                .as_u64()
                .ok_or_else(|| std::io::Error::other("missing seq"))?;
            let is_delta = value["type"] == "delta";
            callback_captured.lock().unwrap().push(value);

            if is_delta {
                let ack_request = Arc::clone(&callback_request);
                tokio::spawn(async move {
                    if seq == 1 {
                        loop {
                            let pressure_applied = {
                                let machine = ack_request.machine.lock().await;
                                machine.effective_batch_window_ms == MAX_BATCH_WINDOW_MS
                            };
                            if pressure_applied {
                                break;
                            }
                            tokio::task::yield_now().await;
                        }
                    }
                    let acknowledged_through = {
                        let mut machine = ack_request.machine.lock().await;
                        machine.acknowledge(seq).unwrap()
                    };
                    ack_request.record_acknowledged_through(acknowledged_through);
                    ack_request.notify.notify_one();
                });
            }
            Ok(())
        });

        channel.send(started).unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            run_stream(Arc::clone(&request), config, channel),
        )
        .await
        .expect("pressure-driven stream should not stall");

        let events = captured.lock().unwrap().clone();
        let delta_lengths: Vec<usize> = events
            .iter()
            .filter(|event| event["type"] == "delta")
            .map(|event| event["text"].as_str().unwrap().len())
            .collect();
        assert_eq!(delta_lengths.first(), Some(&3));
        assert!(
            delta_lengths.iter().skip(1).any(|length| *length > 3),
            "expected a post-pressure batch larger than the initial 24 ms batch: {delta_lengths:?}"
        );
        assert_eq!(delta_lengths.iter().sum::<usize>(), chunks.len());
        assert_eq!(events.last().unwrap()["type"], "completed");

        let snapshot = request.machine.lock().await.snapshot();
        assert_eq!(snapshot.effective_batch_window_ms, 50);
        assert_snapshot_text_receipt(&snapshot, &"x".repeat(chunks.len()));
        assert!(snapshot.in_flight <= snapshot.max_in_flight);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_ack_reaches_a_bounded_failed_terminal() {
        use tauri::ipc::InvokeResponseBody;

        let mut config = config(24, 2);
        config.ack_timeout_ms = 10;
        let mut machine = StreamMachine::new("request-ack-timeout".into(), &config);
        let started = machine.start().unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let captured = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let callback_captured = Arc::clone(&captured);
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            callback_captured
                .lock()
                .unwrap()
                .push(serde_json::from_str(&json)?);
            Ok(())
        });

        send_direct_channel_event(&channel, started).unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            run_stream(Arc::clone(&request), config, channel),
        )
        .await
        .expect("a missing ACK must not leave the stream runner alive");

        let events = captured.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "started");
        assert_eq!(events[1]["type"], "failed");
        assert_eq!(events[1]["error"]["code"], "ACK_TIMEOUT");

        let snapshot = request.machine.lock().await.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.last_seq, Some(1));
        assert_eq!(snapshot.last_acked_seq, None);
        assert_eq!(snapshot.in_flight, 2);
        assert_snapshot_text_receipt(&snapshot, "");
    }
}
