use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{ipc::Channel, State};
use tokio::sync::{watch, Mutex as AsyncMutex, Notify};

const MIN_BATCH_WINDOW_MS: u64 = 16;
const MAX_BATCH_WINDOW_MS: u64 = 50;
const DEFAULT_BATCH_WINDOW_MS: u64 = 24;
const DEFAULT_MAX_IN_FLIGHT: u64 = 4;
const DEFAULT_CHUNK_INTERVAL_MS: u64 = 8;
const DEFAULT_ACK_TIMEOUT_MS: u64 = 1_000;
const MAX_REQUESTS: usize = 128;
const MAX_SEQUENCE: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StreamFailure {
    code: String,
    message: String,
}

impl StreamFailure {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
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
        max_in_flight: u64,
    },
    Delta {
        request_id: String,
        seq: u64,
        text: String,
    },
    Completed {
        request_id: String,
        seq: u64,
        text: String,
    },
    Cancelled {
        request_id: String,
        seq: u64,
        partial_text: String,
    },
    Failed {
        request_id: String,
        seq: u64,
        partial_text: String,
        error: StreamFailure,
    },
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StreamConfig {
    batch_window_ms: Option<u64>,
    max_in_flight: Option<u64>,
    chunk_interval_ms: Option<u64>,
    chunks: Option<Vec<String>>,
    fail_after_chunks: Option<usize>,
    ack_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct ValidatedConfig {
    batch_window_ms: u64,
    max_in_flight: u64,
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
    max_in_flight: u64,
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

    fn in_flight(&self) -> u64 {
        let emitted = self.last_seq.map_or(0, |seq| seq + 1);
        let acknowledged = self.last_acked_seq.map_or(0, |seq| seq + 1);
        emitted.saturating_sub(acknowledged)
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
            text: self.text.clone(),
        })
    }

    fn cancel(&mut self) -> Result<StreamEvent, CommandError> {
        self.ensure_terminal_allowed()?;
        let seq = self.advance_sequence()?;
        self.status = StreamStatus::Cancelled;
        Ok(StreamEvent::Cancelled {
            request_id: self.request_id.clone(),
            seq,
            partial_text: self.text.clone(),
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
            partial_text: self.text.clone(),
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
            text: self.text.clone(),
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
}

impl StreamRequest {
    fn new(machine: StreamMachine) -> Self {
        let (terminal_signal, _terminal_receiver) = watch::channel(false);
        Self {
            machine: AsyncMutex::new(machine),
            notify: Notify::new(),
            terminal_signal,
        }
    }

    fn signal_terminal(&self) {
        let _previous = self.terminal_signal.send_replace(true);
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
}

#[derive(Default)]
struct RegistryInner {
    requests: HashMap<String, Arc<StreamRequest>>,
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
        let id = previous + 1;
        Ok(format!("m1-channel-{id:016x}"))
    }

    fn insert(&self, request_id: String, request: Arc<StreamRequest>) -> Result<(), CommandError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| CommandError::internal("stream registry lock poisoned"))?;

        if inner.requests.len() >= MAX_REQUESTS {
            return Err(CommandError::capacity());
        }
        if inner.requests.contains_key(&request_id) {
            return Err(CommandError::internal(format!(
                "duplicate requestId {request_id}"
            )));
        }

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
            .ok_or_else(|| CommandError::not_found(request_id))
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
    in_flight: u64,
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
    in_flight: u64,
    text: String,
    error: Option<StreamFailure>,
    batch_window_ms: u64,
    effective_batch_window_ms: u64,
    max_in_flight: u64,
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

    fn not_found(request_id: &str) -> Self {
        Self::new(
            "STREAM_NOT_FOUND",
            format!("unknown requestId {request_id}"),
        )
    }

    fn capacity() -> Self {
        Self::new(
            "REGISTRY_CAPACITY",
            "too many retained streams; finalize and release a terminal stream before retrying",
        )
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new("INTERNAL", message)
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
    if let Err(error) = on_event.send(started) {
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
    let request = registry.get(&request_id)?;
    let response = {
        let mut machine = request.machine.lock().await;
        let acknowledged_through = machine.acknowledge(seq)?;
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
    let request = registry.get(&request_id)?;
    Ok(clone_snapshot(&request).await)
}

#[tauri::command]
async fn wait_stream_terminal(
    request_id: String,
    registry: State<'_, StreamRegistry>,
) -> Result<StreamSnapshot, CommandError> {
    let request = registry.get(&request_id)?;
    wait_for_terminal_snapshot(&request).await
}

#[tauri::command]
async fn release_stream(
    request_id: String,
    snapshot_seq: u64,
    registry: State<'_, StreamRegistry>,
) -> Result<ReleaseStreamResponse, CommandError> {
    release_request(&registry, request_id, snapshot_seq).await
}

async fn clone_snapshot(request: &StreamRequest) -> StreamSnapshot {
    request.machine.lock().await.snapshot()
}

async fn wait_for_terminal_snapshot(
    request: &StreamRequest,
) -> Result<StreamSnapshot, CommandError> {
    request.wait_for_terminal().await?;
    let snapshot = clone_snapshot(request).await;
    if !snapshot.status.is_terminal() {
        return Err(CommandError::internal(
            "terminal signal was set before the stream reached a terminal state",
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
    }

    if !registry.remove_exact(&request_id, &request)? {
        return Err(CommandError::not_found(&request_id));
    }

    Ok(ReleaseStreamResponse {
        request_id,
        released: true,
    })
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
        let mut batch_size = (effective_window_ms / config.chunk_interval_ms).max(1) as usize;
        batch_size = batch_size.min(config.chunks.len() - source_index);
        if let Some(fail_after) = config.fail_after_chunks {
            batch_size = batch_size.min(fail_after.saturating_sub(source_index).max(1));
        }

        let mut delta = String::new();
        for chunk in &config.chunks[source_index..source_index + batch_size] {
            if !sleep_unless_cancelled(&request, config.chunk_interval_ms).await {
                emit_cancelled(&request, &channel).await;
                return;
            }
            delta.push_str(chunk);
        }

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

        match emit_delta(&request, &channel, delta).await {
            DeltaDelivery::Sent => {}
            DeltaDelivery::Cancelled => {
                emit_cancelled(&request, &channel).await;
                return;
            }
            DeltaDelivery::Failed => return,
        }
        source_index += batch_size;
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
        Err(_) => return DeltaDelivery::Failed,
    };

    if let Err(error) = channel.send(event) {
        machine.text.truncate(previous_len);
        machine.last_seq = previous_seq;
        machine.status = StreamStatus::Failed;
        machine.error = Some(StreamFailure::new(
            "CHANNEL_DELIVERY_FAILED",
            error.to_string(),
        ));
        request.signal_terminal();
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

    let event = match transition {
        TerminalTransition::Complete => machine.complete(),
        TerminalTransition::Cancel => machine.cancel(),
        TerminalTransition::Fail(failure) => machine.fail(failure),
    };
    let event = match event {
        Ok(event) => event,
        Err(_) if machine.cancel_requested && machine.status == StreamStatus::Streaming => {
            return TerminalDelivery::CancellationRequested;
        }
        Err(_) => return TerminalDelivery::Failed,
    };
    if let Err(error) = channel.send(event) {
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
        request.signal_terminal();
        request.notify.notify_one();
        return TerminalDelivery::Failed;
    }

    request.signal_terminal();
    request.notify.notify_one();
    TerminalDelivery::Sent
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(StreamRegistry::default())
        .invoke_handler(tauri::generate_handler![
            start_mock_stream,
            ack_stream,
            cancel_stream,
            get_stream_snapshot,
            wait_stream_terminal,
            release_stream
        ])
        .run(tauri::generate_context!())
        .expect("error while running LorePia Channel spike");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(batch_window_ms: u64, max_in_flight: u64) -> ValidatedConfig {
        ValidatedConfig {
            batch_window_ms,
            max_in_flight,
            chunk_interval_ms: 1,
            chunks: vec!["A".into(), "B".into(), "C".into()],
            fail_after_chunks: None,
            ack_timeout_ms: 10,
        }
    }

    fn started_machine(max_in_flight: u64) -> StreamMachine {
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
        assert!(
            matches!(terminal, StreamEvent::Completed { seq: 3, ref text, .. } if text == "ABC")
        );
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
        assert!(
            matches!(terminal, StreamEvent::Cancelled { seq: 2, ref partial_text, .. } if partial_text == "partial")
        );
        assert_eq!(machine.snapshot().text, "partial");
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
        assert!(
            matches!(terminal, StreamEvent::Failed { seq: 2, ref partial_text, error, .. } if partial_text == "AB" && error == failure)
        );
        let snapshot = machine.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.text, "AB");
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
    fn queued_stream_has_no_phantom_event_and_rejects_early_ack() {
        let mut machine = StreamMachine::new("request-queued".into(), &config(24, 3));
        let snapshot = machine.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Queued);
        assert_eq!(snapshot.last_seq, None);
        assert_eq!(snapshot.last_acked_seq, None);
        assert_eq!(snapshot.in_flight, 0);

        let error = machine.acknowledge(0).unwrap_err();
        assert_eq!(error.code, "INVALID_ACK");
        assert_eq!(machine.last_seq, None);
        assert_eq!(machine.last_acked_seq, None);
    }

    #[test]
    fn sequence_exhaustion_is_reported_without_mutating_the_machine() {
        let mut machine = started_machine(3);
        machine.last_seq = Some(MAX_SEQUENCE);
        machine.last_acked_seq = Some(MAX_SEQUENCE);
        let before = machine.snapshot();

        let error = machine.delta("never emitted".into()).unwrap_err();
        assert_eq!(error.code, "SEQUENCE_EXHAUSTED");
        assert_eq!(machine.snapshot(), before);
    }

    #[test]
    fn request_id_exhaustion_is_reported_without_wrapping() {
        let registry = StreamRegistry {
            next_id: AtomicU64::new(u64::MAX),
            inner: Mutex::new(RegistryInner::default()),
        };
        let error = registry.next_request_id().unwrap_err();
        assert_eq!(error.code, "INTERNAL");
        assert!(error.message.contains("exhausted"));
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
                partial_text: "A".into(),
            })
            .unwrap(),
            serde_json::to_value(StreamEvent::Failed {
                request_id: "r1".into(),
                seq: 2,
                partial_text: "A".into(),
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
        assert_eq!(values[2]["partialText"], "A");
        assert_eq!(values[3]["type"], "failed");
        assert_eq!(values[3]["error"]["code"], "MOCK_FAILURE");

        for value in values {
            let object = value.as_object().unwrap();
            assert!(!object.contains_key("request_id"));
            assert!(!object.contains_key("batch_window_ms"));
            assert!(!object.contains_key("partial_text"));
        }

        let queued_snapshot =
            serde_json::to_value(StreamMachine::new("queued".into(), &config(24, 2)).snapshot())
                .unwrap();
        assert!(queued_snapshot["lastSeq"].is_null());
        assert!(queued_snapshot["lastAckedSeq"].is_null());
        assert_eq!(queued_snapshot["inFlight"], 0);

        let snapshot = serde_json::to_value(started_machine(2).snapshot()).unwrap();
        assert_eq!(snapshot["status"], "streaming");
        assert_eq!(snapshot["lastSeq"], 0);
        assert!(snapshot["lastAckedSeq"].is_null());
        assert_eq!(snapshot["effectiveBatchWindowMs"], 24);
        assert!(snapshot.get("last_acked_seq").is_none());
    }

    #[test]
    fn total_chunk_byte_limit_is_enforced() {
        let invalid = StreamConfig {
            chunks: Some(vec!["x".repeat(16_384); 65]),
            ..Default::default()
        };
        let error = ValidatedConfig::try_from(invalid).unwrap_err();
        assert_eq!(error.code, "INVALID_CONFIG");
        assert!(error.message.contains("total"));
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

        let snapshot = wait_for_terminal_snapshot(&request).await.unwrap();
        assert_eq!(snapshot.status, StreamStatus::Completed);
        assert_eq!(snapshot.text, "ABC");
        assert!((snapshot.batch_window_ms..=MAX_BATCH_WINDOW_MS)
            .contains(&snapshot.effective_batch_window_ms));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_waiter_wakes_after_successful_terminal_delivery() {
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let waiter_request = Arc::clone(&request);
        let waiter =
            tokio::spawn(async move { wait_for_terminal_snapshot(&waiter_request).await.unwrap() });
        let channel = Channel::new(|_| Ok(()));

        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));
        let snapshot = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("terminal waiter should wake")
            .unwrap();
        assert_eq!(snapshot.status, StreamStatus::Completed);
        assert_eq!(snapshot.last_seq, Some(1));

        let late_snapshot = tokio::time::timeout(
            Duration::from_millis(20),
            wait_for_terminal_snapshot(&request),
        )
        .await
        .expect("late terminal subscriber should observe retained signal")
        .unwrap();
        assert_eq!(late_snapshot, snapshot);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_waiter_recovers_terminal_delivery_failure() {
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let waiter_request = Arc::clone(&request);
        let waiter =
            tokio::spawn(async move { wait_for_terminal_snapshot(&waiter_request).await.unwrap() });
        let channel = Channel::new(|_| Err(std::io::Error::other("closed channel").into()));

        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Failed
        ));
        let snapshot = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("control-plane waiter should recover a failed terminal handoff")
            .unwrap();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.last_seq, Some(0));
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code.as_str()),
            Some("CHANNEL_DELIVERY_FAILED")
        );
        assert!(snapshot.error.unwrap().message.contains("closed channel"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_waiter_recovers_delta_delivery_failure_without_phantom_text() {
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let waiter_request = Arc::clone(&request);
        let waiter =
            tokio::spawn(async move { wait_for_terminal_snapshot(&waiter_request).await.unwrap() });
        let channel = Channel::new(|_| Err(std::io::Error::other("closed channel").into()));

        assert!(matches!(
            emit_delta(&request, &channel, "undelivered".into()).await,
            DeltaDelivery::Failed
        ));
        let snapshot = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("control-plane waiter should recover a failed delta handoff")
            .unwrap();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.last_seq, Some(0));
        assert_eq!(snapshot.text, "");
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code.as_str()),
            Some("CHANNEL_DELIVERY_FAILED")
        );
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
    async fn snapshot_reads_are_side_effect_free_and_release_is_explicit() {
        let registry = StreamRegistry::default();
        let config = config(24, 2);
        let mut machine = StreamMachine::new("request-release".into(), &config);
        machine.start().unwrap();
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        registry
            .insert("request-release".into(), Arc::clone(&request))
            .unwrap();
        let channel = Channel::new(|_| Ok(()));
        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));

        let first = clone_snapshot(&request).await;
        let second = clone_snapshot(&request).await;
        assert_eq!(first, second);
        assert!(registry.get("request-release").is_ok());

        let stale = release_request(&registry, "request-release".into(), 99)
            .await
            .unwrap_err();
        assert_eq!(stale.code, "INVALID_RELEASE");
        assert!(registry.get("request-release").is_ok());

        let response =
            release_request(&registry, "request-release".into(), first.last_seq.unwrap())
                .await
                .unwrap();
        assert!(response.released);
        assert!(matches!(
            registry.get("request-release"),
            Err(CommandError { ref code, .. }) if code == "STREAM_NOT_FOUND"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn release_rejects_a_nonterminal_stream_without_removing_it() {
        let registry = StreamRegistry::default();
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        registry
            .insert("request-active".into(), Arc::clone(&request))
            .unwrap();

        let error = release_request(&registry, "request-active".into(), 0)
            .await
            .unwrap_err();
        assert_eq!(error.code, "INVALID_RELEASE");
        assert!(registry.get("request-active").is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn channel_delivery_failure_can_be_recovered_and_released_immediately() {
        let registry = StreamRegistry::default();
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        registry
            .insert("request-channel-failure".into(), Arc::clone(&request))
            .unwrap();
        let channel = Channel::new(|_| Err(std::io::Error::other("closed channel").into()));

        assert!(matches!(
            emit_delta(&request, &channel, "undelivered".into()).await,
            DeltaDelivery::Failed
        ));
        let snapshot = wait_for_terminal_snapshot(&request).await.unwrap();
        let response = release_request(
            &registry,
            "request-channel-failure".into(),
            snapshot.last_seq.unwrap(),
        )
        .await
        .unwrap();
        assert!(response.released);
        assert!(matches!(
            registry.get("request-channel-failure"),
            Err(CommandError { ref code, .. }) if code == "STREAM_NOT_FOUND"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registry_capacity_recovers_only_after_explicit_release() {
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

        let candidate = Arc::new(StreamRequest::new(StreamMachine::new(
            "request-new".into(),
            &config,
        )));
        let full = registry
            .insert("request-new".into(), Arc::clone(&candidate))
            .unwrap_err();
        assert_eq!(full.code, "REGISTRY_CAPACITY");

        let channel = Channel::new(|_| Ok(()));
        assert!(matches!(
            deliver_terminal(&first, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));
        let snapshot = wait_for_terminal_snapshot(&first).await.unwrap();

        let still_full = registry
            .insert("request-new".into(), Arc::clone(&candidate))
            .unwrap_err();
        assert_eq!(still_full.code, "REGISTRY_CAPACITY");

        release_request(&registry, "request-000".into(), snapshot.last_seq.unwrap())
            .await
            .unwrap();
        registry.insert("request-new".into(), candidate).unwrap();
        assert_eq!(registry.inner.lock().unwrap().requests.len(), MAX_REQUESTS);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_release_succeeds_exactly_once() {
        let registry = Arc::new(StreamRegistry::default());
        let mut machine = started_machine(2);
        machine.acknowledge(0).unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        registry
            .insert("request-race".into(), Arc::clone(&request))
            .unwrap();
        let channel = Channel::new(|_| Ok(()));
        assert!(matches!(
            deliver_terminal(&request, &channel, TerminalTransition::Complete).await,
            TerminalDelivery::Sent
        ));
        let seq = wait_for_terminal_snapshot(&request)
            .await
            .unwrap()
            .last_seq
            .unwrap();

        let left_registry = Arc::clone(&registry);
        let left = tokio::spawn(async move {
            release_request(&left_registry, "request-race".into(), seq).await
        });
        let right_registry = Arc::clone(&registry);
        let right = tokio::spawn(async move {
            release_request(&right_registry, "request-race".into(), seq).await
        });

        let results = [left.await.unwrap(), right.await.unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(
                    result,
                    Err(CommandError { code, .. }) if code == "STREAM_NOT_FOUND"
                ))
                .count(),
            1
        );
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

        let snapshot = wait_for_terminal_snapshot(&request).await.unwrap();
        assert_eq!(snapshot.status, StreamStatus::Completed);
        assert_eq!(snapshot.last_seq, Some(1));
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
                    ack_request.machine.lock().await.acknowledge(seq).unwrap();
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
        assert_eq!(snapshot.text.len(), chunks.len());
        assert!(snapshot.in_flight <= snapshot.max_in_flight);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_ack_times_out_with_failed_terminal() {
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

        channel.send(started).unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            run_stream(Arc::clone(&request), config, channel),
        )
        .await
        .expect("missing ACK should terminate the stream");

        let events = captured.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "started");
        assert_eq!(events[1]["type"], "failed");
        assert_eq!(events[1]["error"]["code"], "ACK_TIMEOUT");
        assert!(events[1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("10 ms"));

        let snapshot = wait_for_terminal_snapshot(&request).await.unwrap();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.last_seq, Some(1));
        assert_eq!(snapshot.last_acked_seq, None);
        assert_eq!(snapshot.in_flight, 2);
        assert_eq!(snapshot.text, "");
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code.as_str()),
            Some("ACK_TIMEOUT")
        );
    }
}
