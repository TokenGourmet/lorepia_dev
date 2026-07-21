use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use lorepia_storage::{
    AppPreferences, CharacterId, Chat, ChatCursor, ChatId, CreateChat, DefaultMode,
    MAX_SAFE_INTEGER, Message, MessageOrdinalCursor, MessageRole, MessageStatus, ProviderId,
    ProviderModelIds, StorageError, Store, Theme, TimestampMillis, UpdatePreferences,
    WalCheckpointPolicy, WalMaintenanceReport,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const STORAGE_FILE_NAME: &str = "lorepia.sqlite3";
const MAX_CHAT_PAGE: u16 = 100;
const MAX_MESSAGE_PAGE: u16 = 200;
const WAL_RESTART_THRESHOLD_BYTES: u64 = 64 * 1024 * 1024;
const WAL_EMERGENCY_TRUNCATE_THRESHOLD_BYTES: u64 = 512 * 1024 * 1024;
const WAL_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(60);
const STORAGE_ADMISSION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default)]
struct WalMaintenanceTelemetry {
    successful_runs: u64,
    failed_runs: u64,
    consecutive_starvation_runs: u64,
    last_attempt_started_at_ms: Option<i64>,
    last_attempt_completed_at_ms: Option<i64>,
    last_attempt_duration_ms: Option<u64>,
    last_success_at_ms: Option<i64>,
    last_error_at_ms: Option<i64>,
    last_error_code: Option<&'static str>,
    last_report: Option<WalMaintenanceReport>,
}

#[derive(Debug)]
struct WalMaintenanceRuntime {
    scheduler_started: AtomicBool,
    run_in_progress: AtomicBool,
    shutdown: CancellationToken,
    telemetry: Mutex<WalMaintenanceTelemetry>,
}

#[derive(Debug, Default)]
struct ActiveReaderRuntime {
    next_id: AtomicU64,
    started: Mutex<BTreeMap<u64, Instant>>,
}

impl ActiveReaderRuntime {
    fn register(self: &Arc<Self>) -> ActiveReaderGuard {
        self.register_at(Instant::now())
    }

    fn register_at(self: &Arc<Self>, started_at: Instant) -> ActiveReaderGuard {
        // The storage admission gate currently permits one operation at a
        // time, so wrapping cannot collide with an active identifier. Keeping
        // distinct identifiers makes this tracker safe if bounded read
        // concurrency is introduced later.
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, started_at);
        ActiveReaderGuard {
            id,
            runtime: Arc::clone(self),
        }
    }

    fn snapshot_at(&self, now: Instant) -> (u64, Option<u64>) {
        let started = self
            .started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active_readers = u64::try_from(started.len())
            .unwrap_or(MAX_SAFE_INTEGER)
            .min(MAX_SAFE_INTEGER);
        let oldest_reader_age_ms = started
            .values()
            .min()
            .map(|started_at| bounded_duration_ms(now.saturating_duration_since(*started_at)));
        (active_readers, oldest_reader_age_ms)
    }
}

struct ActiveReaderGuard {
    id: u64,
    runtime: Arc<ActiveReaderRuntime>,
}

impl Drop for ActiveReaderGuard {
    fn drop(&mut self) {
        self.runtime
            .started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.id);
    }
}

impl Default for WalMaintenanceRuntime {
    fn default() -> Self {
        Self {
            scheduler_started: AtomicBool::new(false),
            run_in_progress: AtomicBool::new(false),
            shutdown: CancellationToken::new(),
            telemetry: Mutex::new(WalMaintenanceTelemetry::default()),
        }
    }
}

impl Drop for WalMaintenanceRuntime {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

struct WalMaintenanceRunGuard<'a>(&'a AtomicBool);

impl Drop for WalMaintenanceRunGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

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
            StorageError::SnapshotCancelled => Self {
                code: "STORAGE_OPERATION_CANCELLED",
                message: "storage operation was cancelled",
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
    operation_gate: Arc<Semaphore>,
    wal_maintenance: Arc<WalMaintenanceRuntime>,
    active_readers: Arc<ActiveReaderRuntime>,
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
            operation_gate: Arc::new(Semaphore::new(1)),
            wal_maintenance: Arc::new(WalMaintenanceRuntime::default()),
            active_readers: Arc::new(ActiveReaderRuntime::default()),
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
        self.run_inner(operation, false).await
    }

    pub(crate) async fn run_read<T, F>(&self, operation: F) -> Result<T, StorageCommandError>
    where
        T: Send + 'static,
        F: FnOnce(Store) -> Result<T, StorageError> + Send + 'static,
    {
        self.run_inner(operation, true).await
    }

    async fn run_inner<T, F>(
        &self,
        operation: F,
        track_active_reader: bool,
    ) -> Result<T, StorageCommandError>
    where
        T: Send + 'static,
        F: FnOnce(Store) -> Result<T, StorageError> + Send + 'static,
    {
        let store = self.store()?;
        let operation_gate = Arc::clone(&self.operation_gate);
        let active_readers = Arc::clone(&self.active_readers);
        // Bound queue admission, but never abandon a mutation after it starts:
        // timing out the blocking SQLite closure itself could let a late
        // commit create an orphan turn after the caller has already retried.
        let permit =
            tokio::time::timeout(STORAGE_ADMISSION_TIMEOUT, operation_gate.acquire_owned())
                .await
                .map_err(|_| StorageCommandError {
                    code: "STORAGE_ADMISSION_TIMEOUT",
                    message: "local storage is busy",
                })?
                .map_err(|_| StorageCommandError {
                    code: "STORAGE_TASK_FAILED",
                    message: "local storage operation gate is closed",
                })?;
        tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            // Store read APIs scope their SQLite connection and transaction to
            // this closure. Consequently this guard is a conservative upper
            // bound on the lifetime of the product read transaction, and it
            // cannot be retained by a UI cursor between frames.
            let _active_reader = track_active_reader.then(|| active_readers.register());
            operation(store).map_err(StorageCommandError::from_storage)
        })
        .await
        .map_err(|_| StorageCommandError {
            code: "STORAGE_TASK_FAILED",
            message: "local storage task failed",
        })?
    }

    pub(crate) fn start_wal_maintenance(&self) -> bool {
        self.start_wal_maintenance_with_interval(WAL_MAINTENANCE_INTERVAL)
    }

    fn start_wal_maintenance_with_interval(&self, interval: Duration) -> bool {
        if interval.is_zero()
            || self.store().is_err()
            || self.wal_maintenance.shutdown.is_cancelled()
            || self
                .wal_maintenance
                .scheduler_started
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return false;
        }

        let backend = Arc::downgrade(&self.backend);
        let operation_gate = Arc::downgrade(&self.operation_gate);
        let wal_maintenance = Arc::downgrade(&self.wal_maintenance);
        let active_readers = Arc::downgrade(&self.active_readers);
        let shutdown = self.wal_maintenance.shutdown.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                if shutdown.is_cancelled() {
                    break;
                }
                if !run_scheduled_wal_maintenance(
                    &backend,
                    &operation_gate,
                    &wal_maintenance,
                    &active_readers,
                )
                .await
                {
                    break;
                }
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(interval) => {}
                }
            }
        });
        true
    }

    pub(crate) fn shutdown_wal_maintenance(&self) {
        self.wal_maintenance.shutdown.cancel();
    }

    async fn run_wal_maintenance_once(&self) -> bool {
        if self.wal_maintenance.shutdown.is_cancelled()
            || self
                .wal_maintenance
                .run_in_progress
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return false;
        }
        let _run_guard = WalMaintenanceRunGuard(&self.wal_maintenance.run_in_progress);
        if self.wal_maintenance.shutdown.is_cancelled() {
            return false;
        }

        let started_at_ms = current_timestamp_ms();
        if let Ok(mut telemetry) = self.wal_maintenance.telemetry.lock() {
            telemetry.last_attempt_started_at_ms = started_at_ms;
        }
        let started = Instant::now();
        let result = self
            .run(|store| {
                store.maintain_wal(WalCheckpointPolicy {
                    restart_threshold_bytes: Some(WAL_RESTART_THRESHOLD_BYTES),
                    emergency_truncate_threshold_bytes: Some(
                        WAL_EMERGENCY_TRUNCATE_THRESHOLD_BYTES,
                    ),
                })
            })
            .await;
        let duration_ms = bounded_duration_ms(started.elapsed());
        let completed_at_ms = current_timestamp_ms();

        if let Ok(mut telemetry) = self.wal_maintenance.telemetry.lock() {
            telemetry.last_attempt_completed_at_ms = completed_at_ms;
            telemetry.last_attempt_duration_ms = Some(duration_ms);
            match result {
                Ok(report) => {
                    telemetry.successful_runs = bounded_increment(telemetry.successful_runs);
                    telemetry.consecutive_starvation_runs = if report.starvation_observed {
                        bounded_increment(telemetry.consecutive_starvation_runs)
                    } else {
                        0
                    };
                    telemetry.last_success_at_ms = completed_at_ms;
                    telemetry.last_report = Some(report);
                }
                Err(error) => {
                    telemetry.failed_runs = bounded_increment(telemetry.failed_runs);
                    telemetry.last_error_at_ms = completed_at_ms;
                    telemetry.last_error_code = Some(error.code);
                }
            }
        }
        true
    }
}

async fn run_scheduled_wal_maintenance(
    backend: &Weak<StorageBackend>,
    operation_gate: &Weak<Semaphore>,
    wal_maintenance: &Weak<WalMaintenanceRuntime>,
    active_readers: &Weak<ActiveReaderRuntime>,
) -> bool {
    let (Some(backend), Some(operation_gate), Some(wal_maintenance), Some(active_readers)) = (
        backend.upgrade(),
        operation_gate.upgrade(),
        wal_maintenance.upgrade(),
        active_readers.upgrade(),
    ) else {
        return false;
    };
    let storage = StorageState {
        backend,
        operation_gate,
        wal_maintenance,
        active_readers,
    };
    storage.run_wal_maintenance_once().await;
    true
}

fn bounded_increment(value: u64) -> u64 {
    value.saturating_add(1).min(MAX_SAFE_INTEGER)
}

fn bounded_duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis())
        .unwrap_or(MAX_SAFE_INTEGER)
        .min(MAX_SAFE_INTEGER)
}

fn bounded_telemetry_value(value: u64) -> u64 {
    value.min(MAX_SAFE_INTEGER)
}

fn current_timestamp_ms() -> Option<i64> {
    TimestampMillis::now().ok().map(TimestampMillis::get)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WalMaintenanceStatusResponse {
    scheduler_started: bool,
    running: bool,
    interval_ms: u64,
    restart_threshold_bytes: u64,
    emergency_truncate_threshold_bytes: u64,
    successful_runs: u64,
    failed_runs: u64,
    consecutive_starvation_runs: u64,
    active_readers: u64,
    oldest_reader_age_ms: Option<u64>,
    last_attempt_started_at_ms: Option<i64>,
    last_attempt_completed_at_ms: Option<i64>,
    last_attempt_duration_ms: Option<u64>,
    last_success_at_ms: Option<i64>,
    last_error_at_ms: Option<i64>,
    last_error_code: Option<&'static str>,
    passive_busy: Option<bool>,
    passive_remaining_frames: Option<u64>,
    passive_wal_file_bytes: Option<u64>,
    restart_busy: Option<bool>,
    restart_remaining_frames: Option<u64>,
    restart_wal_file_bytes: Option<u64>,
    truncate_busy: Option<bool>,
    truncate_remaining_frames: Option<u64>,
    truncate_wal_file_bytes: Option<u64>,
    threshold_exceeded: Option<bool>,
    emergency_truncate_threshold_exceeded: Option<bool>,
    starvation_observed: Option<bool>,
}

impl StorageState {
    fn wal_maintenance_status(&self) -> WalMaintenanceStatusResponse {
        let scheduler_started = self
            .wal_maintenance
            .scheduler_started
            .load(Ordering::Acquire);
        let running = self.wal_maintenance.run_in_progress.load(Ordering::Acquire);
        let (active_readers, oldest_reader_age_ms) =
            self.active_readers.snapshot_at(Instant::now());
        let Ok(telemetry) = self.wal_maintenance.telemetry.lock() else {
            return WalMaintenanceStatusResponse {
                scheduler_started,
                running,
                interval_ms: bounded_duration_ms(WAL_MAINTENANCE_INTERVAL),
                restart_threshold_bytes: WAL_RESTART_THRESHOLD_BYTES,
                emergency_truncate_threshold_bytes: WAL_EMERGENCY_TRUNCATE_THRESHOLD_BYTES,
                successful_runs: 0,
                failed_runs: 0,
                consecutive_starvation_runs: 0,
                active_readers,
                oldest_reader_age_ms,
                last_attempt_started_at_ms: None,
                last_attempt_completed_at_ms: None,
                last_attempt_duration_ms: None,
                last_success_at_ms: None,
                last_error_at_ms: None,
                last_error_code: Some("WAL_TELEMETRY_UNAVAILABLE"),
                passive_busy: None,
                passive_remaining_frames: None,
                passive_wal_file_bytes: None,
                restart_busy: None,
                restart_remaining_frames: None,
                restart_wal_file_bytes: None,
                truncate_busy: None,
                truncate_remaining_frames: None,
                truncate_wal_file_bytes: None,
                threshold_exceeded: None,
                emergency_truncate_threshold_exceeded: None,
                starvation_observed: None,
            };
        };
        let passive = telemetry.last_report.map(|report| report.passive);
        let restart = telemetry.last_report.and_then(|report| report.restart);
        let truncate = telemetry.last_report.and_then(|report| report.truncate);
        WalMaintenanceStatusResponse {
            scheduler_started,
            running,
            interval_ms: bounded_duration_ms(WAL_MAINTENANCE_INTERVAL),
            restart_threshold_bytes: WAL_RESTART_THRESHOLD_BYTES,
            emergency_truncate_threshold_bytes: WAL_EMERGENCY_TRUNCATE_THRESHOLD_BYTES,
            successful_runs: bounded_telemetry_value(telemetry.successful_runs),
            failed_runs: bounded_telemetry_value(telemetry.failed_runs),
            consecutive_starvation_runs: bounded_telemetry_value(
                telemetry.consecutive_starvation_runs,
            ),
            active_readers,
            oldest_reader_age_ms,
            last_attempt_started_at_ms: telemetry.last_attempt_started_at_ms,
            last_attempt_completed_at_ms: telemetry.last_attempt_completed_at_ms,
            last_attempt_duration_ms: telemetry
                .last_attempt_duration_ms
                .map(bounded_telemetry_value),
            last_success_at_ms: telemetry.last_success_at_ms,
            last_error_at_ms: telemetry.last_error_at_ms,
            last_error_code: telemetry.last_error_code,
            passive_busy: passive.map(|sample| sample.busy),
            passive_remaining_frames: passive
                .map(|sample| bounded_telemetry_value(sample.remaining_frames)),
            passive_wal_file_bytes: passive
                .map(|sample| bounded_telemetry_value(sample.wal_file_bytes)),
            restart_busy: restart.map(|sample| sample.busy),
            restart_remaining_frames: restart
                .map(|sample| bounded_telemetry_value(sample.remaining_frames)),
            restart_wal_file_bytes: restart
                .map(|sample| bounded_telemetry_value(sample.wal_file_bytes)),
            truncate_busy: truncate.map(|sample| sample.busy),
            truncate_remaining_frames: truncate
                .map(|sample| bounded_telemetry_value(sample.remaining_frames)),
            truncate_wal_file_bytes: truncate
                .map(|sample| bounded_telemetry_value(sample.wal_file_bytes)),
            threshold_exceeded: telemetry
                .last_report
                .map(|report| report.threshold_exceeded),
            emergency_truncate_threshold_exceeded: telemetry
                .last_report
                .map(|report| report.emergency_truncate_threshold_exceeded),
            starvation_observed: telemetry
                .last_report
                .map(|report| report.starvation_observed),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageStatusResponse {
    available: bool,
    schema_version: Option<u64>,
    error_code: Option<&'static str>,
    wal_maintenance: WalMaintenanceStatusResponse,
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
    has_more: bool,
    older_cursor: Option<MessageCursorResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MessageCursorInput {
    chat_id: String,
    ordinal: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MessageCursorResponse {
    chat_id: String,
    ordinal: u64,
}

impl From<MessageOrdinalCursor> for MessageCursorResponse {
    fn from(cursor: MessageOrdinalCursor) -> Self {
        Self {
            chat_id: cursor.chat_id.to_string(),
            ordinal: cursor.ordinal,
        }
    }
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
    let wal_maintenance = storage.wal_maintenance_status();
    match storage.backend.as_ref() {
        StorageBackend::Ready(store) => StorageStatusResponse {
            available: true,
            schema_version: u64::try_from(store.startup_report().schema_version).ok(),
            error_code: None,
            wal_maintenance,
        },
        StorageBackend::Unavailable(error) => StorageStatusResponse {
            available: false,
            schema_version: None,
            error_code: Some(error.code),
            wal_maintenance,
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
        .run_read(move |store| store.list_chats(limit, before.as_ref()))
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
    before: Option<MessageCursorInput>,
    storage: State<'_, StorageState>,
) -> Result<MessagePageResponse, StorageCommandError> {
    if limit == 0 || limit > MAX_MESSAGE_PAGE {
        return Err(StorageCommandError {
            code: "STORAGE_INPUT_INVALID",
            message: "message page size is invalid",
        });
    }
    let chat_id = ChatId::parse(chat_id).map_err(StorageCommandError::from_storage)?;
    let before = before
        .map(|cursor| MessageOrdinalCursor::new(ChatId::parse(cursor.chat_id)?, cursor.ordinal))
        .transpose()
        .map_err(StorageCommandError::from_storage)?;
    let page = storage
        .run_read(move |store| store.load_recent_messages(&chat_id, before.as_ref(), limit))
        .await?;
    let has_more = page.older_cursor.is_some();
    Ok(MessagePageResponse {
        items: page.messages.into_iter().map(Into::into).collect(),
        has_more,
        older_cursor: page.older_cursor.map(Into::into),
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
        .run_read(|store| store.load_preferences())
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
    use lorepia_storage::WalCheckpointTelemetry;

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

    #[test]
    fn message_cursor_input_is_closed_and_range_checked() {
        let valid: MessageCursorInput = serde_json::from_value(serde_json::json!({
            "chatId": "a".repeat(32),
            "ordinal": 42,
        }))
        .unwrap();
        assert_eq!(
            MessageOrdinalCursor::new(ChatId::parse(valid.chat_id).unwrap(), valid.ordinal)
                .unwrap()
                .ordinal,
            42
        );

        assert!(
            serde_json::from_value::<MessageCursorInput>(serde_json::json!({
                "chatId": "a".repeat(32),
                "ordinal": 42,
                "afterOrdinal": 41,
            }))
            .is_err()
        );
        let chat_id = ChatId::parse("a".repeat(32)).unwrap();
        assert!(MessageOrdinalCursor::new(chat_id.clone(), 0).is_err());
        assert!(MessageOrdinalCursor::new(chat_id, u64::MAX).is_err());
    }

    #[test]
    fn wal_status_clamps_counters_and_file_sizes_to_json_safe_integers() {
        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));
        let sample = WalCheckpointTelemetry {
            busy: true,
            log_frames: 4,
            checkpointed_frames: 1,
            remaining_frames: u64::MAX,
            page_size_bytes: 4_096,
            frame_payload_bytes: 16_384,
            wal_file_bytes: u64::MAX,
        };
        {
            let mut telemetry = storage.wal_maintenance.telemetry.lock().unwrap();
            telemetry.successful_runs = u64::MAX;
            telemetry.failed_runs = u64::MAX;
            telemetry.consecutive_starvation_runs = u64::MAX;
            telemetry.last_error_code = Some("STORAGE_WRITE_FAILED");
            telemetry.last_report = Some(WalMaintenanceReport {
                passive: sample,
                restart: Some(sample),
                truncate: Some(sample),
                restart_threshold_bytes: Some(WAL_RESTART_THRESHOLD_BYTES),
                emergency_truncate_threshold_bytes: Some(WAL_EMERGENCY_TRUNCATE_THRESHOLD_BYTES),
                threshold_exceeded: true,
                emergency_truncate_threshold_exceeded: true,
                starvation_observed: true,
            });
        }

        let status = storage.wal_maintenance_status();
        assert_eq!(status.successful_runs, MAX_SAFE_INTEGER);
        assert_eq!(status.failed_runs, MAX_SAFE_INTEGER);
        assert_eq!(status.consecutive_starvation_runs, MAX_SAFE_INTEGER);
        assert_eq!(status.passive_remaining_frames, Some(MAX_SAFE_INTEGER));
        assert_eq!(status.passive_wal_file_bytes, Some(MAX_SAFE_INTEGER));
        assert_eq!(status.restart_remaining_frames, Some(MAX_SAFE_INTEGER));
        assert_eq!(status.restart_wal_file_bytes, Some(MAX_SAFE_INTEGER));
        assert_eq!(status.truncate_remaining_frames, Some(MAX_SAFE_INTEGER));
        assert_eq!(status.truncate_wal_file_bytes, Some(MAX_SAFE_INTEGER));
        assert_eq!(status.last_error_code, Some("STORAGE_WRITE_FAILED"));
        assert_eq!(status.active_readers, 0);
        assert_eq!(status.oldest_reader_age_ms, None);
    }

    #[test]
    fn active_reader_age_uses_monotonic_time_and_clears_with_the_guard() {
        let runtime = Arc::new(ActiveReaderRuntime::default());
        let started_at = Instant::now();
        let first = runtime.register_at(started_at);
        let second = runtime.register_at(started_at + Duration::from_millis(10));

        assert_eq!(
            runtime.snapshot_at(started_at + Duration::from_millis(25)),
            (2, Some(25))
        );
        drop(first);
        assert_eq!(
            runtime.snapshot_at(started_at + Duration::from_millis(25)),
            (1, Some(15))
        );
        drop(second);
        assert_eq!(
            runtime.snapshot_at(started_at + Duration::from_millis(25)),
            (0, None)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn product_read_is_observable_only_while_its_scoped_operation_is_active() {
        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));
        let read_storage = storage.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let read = tokio::spawn(async move {
            read_storage
                .run_read(move |store| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(1)).unwrap();
                    store.load_preferences()
                })
                .await
        });

        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let active = storage.wal_maintenance_status();
        assert_eq!(active.active_readers, 1);
        assert!(active.oldest_reader_age_ms.is_some());

        release_tx.send(()).unwrap();
        read.await.unwrap().unwrap();
        let inactive = storage.wal_maintenance_status();
        assert_eq!(inactive.active_readers, 0);
        assert_eq!(inactive.oldest_reader_age_ms, None);
    }

    #[tokio::test(start_paused = true)]
    async fn queued_storage_work_times_out_before_it_can_commit_late() {
        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));
        let held = Arc::clone(&storage.operation_gate)
            .acquire_owned()
            .await
            .unwrap();
        let queued_storage = storage.clone();
        let queued =
            tokio::spawn(async move { queued_storage.run(|store| store.load_preferences()).await });
        tokio::task::yield_now().await;

        tokio::time::advance(STORAGE_ADMISSION_TIMEOUT).await;
        tokio::task::yield_now().await;
        let error = queued.await.unwrap().unwrap_err();
        assert_eq!(error.code, "STORAGE_ADMISSION_TIMEOUT");

        // The closure never acquired admission, so releasing the permit cannot
        // make the timed-out operation run or commit afterward.
        drop(held);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wal_scheduler_starts_once_and_stops_without_starting_more_runs() {
        let directory = tempfile::tempdir().unwrap();
        let storage = StorageState::open(Ok(directory.path().to_path_buf()));

        assert!(storage.start_wal_maintenance_with_interval(Duration::from_millis(5)));
        assert!(!storage.start_wal_maintenance_with_interval(Duration::from_millis(5)));

        for _ in 0..100 {
            if storage.wal_maintenance_status().successful_runs > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(storage.wal_maintenance_status().successful_runs > 0);

        storage.shutdown_wal_maintenance();
        for _ in 0..100 {
            if !storage.wal_maintenance_status().running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let completed_after_shutdown = storage.wal_maintenance_status().successful_runs;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            storage.wal_maintenance_status().successful_runs,
            completed_after_shutdown
        );
        assert!(!storage.start_wal_maintenance_with_interval(Duration::from_millis(5)));
    }
}
