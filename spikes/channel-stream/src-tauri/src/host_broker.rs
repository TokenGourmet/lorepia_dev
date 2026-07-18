//! Host-authenticated broker state machine for an untrusted plugin frame.
//!
//! This module intentionally has no Tauri dependency or command macros. The
//! WebView-facing integration should pass the host token separately from the
//! plugin request and call [`HostBroker::execute_json`]. The broker holds a
//! session lease until the supplied native executor finishes.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    fmt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, RwLock,
    },
    time::Instant,
};
use subtle::ConstantTimeEq;

pub const HOST_TOKEN_HEX_LEN: usize = 64;
pub const MAX_SANITIZE_HTML_BYTES: usize = 64 * 1024;

const MAX_MODULE_ID_BYTES: usize = 128;
const MAX_PERMISSION_BYTES: usize = 128;
const MAX_PERMISSION_COUNT: usize = 256;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_REQUEST_JSON_BYTES: usize = 512 * 1024;
const MAX_NETWORK_URL_BYTES: usize = 8 * 1024;
const MAX_REPLAY_HISTORY_CAPACITY: usize = 65_536;
const MAX_IN_FLIGHT_CAPACITY: usize = 4_096;
const INITIAL_SESSION_GENERATION: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerErrorCode {
    InvalidConfiguration,
    InvalidRegistration,
    MissingHostToken,
    InvalidHostToken,
    InvalidRotation,
    StaleGeneration,
    RegistrationConflict,
    NotRegistered,
    MalformedRequest,
    UnknownMethod,
    InvalidPayload,
    PermissionDenied,
    NetworkDenied,
    ReplayedRequest,
    RateLimited,
    SessionExhausted,
    ClockRegression,
    StateUnavailable,
    ActionFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerError {
    pub code: BrokerErrorCode,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl BrokerError {
    fn new(code: BrokerErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            request_id: None,
        }
    }

    fn for_request(mut self, request_id: &str) -> Self {
        self.request_id = Some(request_id.to_owned());
        self
    }

    pub fn action_failed(request_id: &str) -> Self {
        Self::new(
            BrokerErrorCode::ActionFailed,
            "The authorized broker action failed.",
        )
        .for_request(request_id)
    }
}

impl fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({:?})", self.message, self.code)
    }
}

impl std::error::Error for BrokerError {}

/// Registration data that is safe to inspect and log. The host token is
/// deliberately accepted as a separate argument to [`HostBroker::register`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrationPolicy {
    pub module_id: String,
    pub manifest_permissions: Vec<String>,
    pub approved_permissions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationOutcome {
    Registered,
    Idempotent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationReceipt {
    pub outcome: RegistrationOutcome,
    pub generation: u64,
    pub module_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationOutcome {
    Rotated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationReceipt {
    pub outcome: RotationOutcome,
    pub generation: u64,
    pub module_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    pub max_requests: u32,
    pub window_ms: u64,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            max_requests: 64,
            window_ms: 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokerLimits {
    pub replay_history_capacity: usize,
    /// Authenticated attempts per session window. Every attempt consumes this
    /// quota before JSON parsing or method authorization.
    pub rate_limit: RateLimit,
    /// All broker entry attempts, including lifecycle calls and missing or
    /// invalid tokens.
    pub global_rate_limit: RateLimit,
    /// Hard process-local cap held through native action execution.
    pub max_in_flight: usize,
}

impl Default for BrokerLimits {
    fn default() -> Self {
        Self {
            replay_history_capacity: 4_096,
            rate_limit: RateLimit::default(),
            global_rate_limit: RateLimit {
                max_requests: 256,
                window_ms: 1_000,
            },
            max_in_flight: 16,
        }
    }
}

pub trait MonotonicClock: Send + Sync + 'static {
    fn now_millis(&self) -> u64;
}

/// Process-local monotonic clock suitable for the production broker state.
pub struct SystemMonotonicClock {
    origin: Instant,
}

impl Default for SystemMonotonicClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl MonotonicClock for SystemMonotonicClock {
    fn now_millis(&self) -> u64 {
        self.origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedAction<'session> {
    pub request_id: String,
    /// Injected from the first successful registration, never from plugin JSON.
    pub module_id: &'session str,
    pub action: BrokerAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrokerAction {
    StateRead,
    RenderSanitize { html: String },
    ProbeIncrement,
}

/// A parsed host token. It stores only the 256-bit value and intentionally does
/// not implement `Debug`, `Display`, or `Serialize`.
struct HostToken {
    bytes: [u8; 32],
}

impl HostToken {
    fn parse(value: Option<&str>) -> Result<Self, BrokerError> {
        let value = value.ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::MissingHostToken,
                "A host token is required.",
            )
        })?;

        let encoded = value.as_bytes();
        if encoded.len() != HOST_TOKEN_HEX_LEN
            || !encoded
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidHostToken,
                "The host token must be 64 lowercase hexadecimal characters.",
            ));
        }

        let mut bytes = [0_u8; 32];
        for (index, pair) in encoded.chunks_exact(2).enumerate() {
            bytes[index] = (decode_hex_nibble(pair[0]) << 4) | decode_hex_nibble(pair[1]);
        }
        Ok(Self { bytes })
    }

    fn matches(&self, candidate: &Self) -> bool {
        bool::from(self.bytes.ct_eq(&candidate.bytes))
    }
}

fn decode_hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

#[derive(Clone, PartialEq, Eq)]
struct CanonicalRegistration {
    module_id: String,
    manifest_permissions: BTreeSet<String>,
    approved_permissions: BTreeSet<String>,
    effective_permissions: BTreeSet<String>,
    // M-1 is deliberately fail-closed. There is no public registration input
    // that can turn network access on.
    network_allowed: bool,
}

impl CanonicalRegistration {
    fn parse(policy: RegistrationPolicy) -> Result<Self, BrokerError> {
        if !is_valid_identifier(&policy.module_id, MAX_MODULE_ID_BYTES) {
            return Err(invalid_registration(
                "The module id is empty, oversized, or contains invalid characters.",
            ));
        }

        let manifest_permissions = canonical_permissions(policy.manifest_permissions)?;
        let approved_permissions = canonical_permissions(policy.approved_permissions)?;
        let effective_permissions = manifest_permissions
            .intersection(&approved_permissions)
            .cloned()
            .collect();

        Ok(Self {
            module_id: policy.module_id,
            manifest_permissions,
            approved_permissions,
            effective_permissions,
            network_allowed: false,
        })
    }
}

fn canonical_permissions(values: Vec<String>) -> Result<BTreeSet<String>, BrokerError> {
    if values.len() > MAX_PERMISSION_COUNT {
        return Err(invalid_registration(
            "The registration contains too many permissions.",
        ));
    }

    let mut result = BTreeSet::new();
    for permission in values {
        if !is_valid_identifier(&permission, MAX_PERMISSION_BYTES) {
            return Err(invalid_registration(
                "A permission is empty, oversized, or contains invalid characters.",
            ));
        }
        result.insert(permission);
    }
    Ok(result)
}

fn invalid_registration(message: &'static str) -> BrokerError {
    BrokerError::new(BrokerErrorCode::InvalidRegistration, message)
}

fn is_valid_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.as_bytes().iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'-' | b'_' | b':')
        })
}

struct RegisteredSession {
    token: HostToken,
    registration: CanonicalRegistration,
    generation: u64,
    runtime: Mutex<RuntimeState>,
}

#[derive(Default)]
struct FixedWindowState {
    window_started_at: Option<u64>,
    attempts_in_window: u32,
}

impl FixedWindowState {
    fn consume(
        &mut self,
        now_ms: u64,
        limit: RateLimit,
        limit_message: &'static str,
    ) -> Result<(), BrokerError> {
        let (window_started_at, attempts_in_window) = match self.window_started_at {
            None => (now_ms, 0),
            Some(started_at) if now_ms < started_at => {
                return Err(BrokerError::new(
                    BrokerErrorCode::ClockRegression,
                    "The monotonic clock moved backwards.",
                ));
            }
            Some(started_at) if now_ms.saturating_sub(started_at) >= limit.window_ms => (now_ms, 0),
            Some(started_at) => (started_at, self.attempts_in_window),
        };

        if attempts_in_window >= limit.max_requests {
            return Err(BrokerError::new(
                BrokerErrorCode::RateLimited,
                limit_message,
            ));
        }

        self.window_started_at = Some(window_started_at);
        self.attempts_in_window = attempts_in_window + 1;
        Ok(())
    }
}

struct RuntimeState {
    request_ids: HashSet<String>,
    authenticated_attempts: FixedWindowState,
}

impl RuntimeState {
    fn new(replay_history_capacity: usize) -> Self {
        Self {
            request_ids: HashSet::with_capacity(replay_history_capacity),
            authenticated_attempts: FixedWindowState::default(),
        }
    }

    fn consume_authenticated_attempt(
        &mut self,
        now_ms: u64,
        rate_limit: RateLimit,
    ) -> Result<(), BrokerError> {
        self.authenticated_attempts.consume(
            now_ms,
            rate_limit,
            "The authenticated session attempt limit was exceeded.",
        )
    }

    fn consume_request_id(
        &mut self,
        request_id: &str,
        replay_history_capacity: usize,
    ) -> Result<(), BrokerError> {
        if self.request_ids.contains(request_id) {
            return Err(BrokerError::new(
                BrokerErrorCode::ReplayedRequest,
                "The request id was already consumed in this session.",
            )
            .for_request(request_id));
        }
        if self.request_ids.len() >= replay_history_capacity {
            return Err(BrokerError::new(
                BrokerErrorCode::SessionExhausted,
                "The bounded replay history is exhausted; create a new session.",
            )
            .for_request(request_id));
        }
        self.request_ids.insert(request_id.to_owned());
        Ok(())
    }
}

struct InFlightPermit<'broker> {
    counter: &'broker AtomicUsize,
}

impl Drop for InFlightPermit<'_> {
    fn drop(&mut self) {
        let previous = self.counter.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}

pub struct HostBroker<C: MonotonicClock> {
    session: RwLock<Option<RegisteredSession>>,
    global_attempts: Mutex<FixedWindowState>,
    in_flight: AtomicUsize,
    clock: C,
    limits: BrokerLimits,
}

impl<C: MonotonicClock> HostBroker<C> {
    pub fn new(clock: C, limits: BrokerLimits) -> Result<Self, BrokerError> {
        if limits.replay_history_capacity == 0
            || limits.replay_history_capacity > MAX_REPLAY_HISTORY_CAPACITY
            || limits.rate_limit.max_requests == 0
            || limits.rate_limit.window_ms == 0
            || limits.global_rate_limit.max_requests == 0
            || limits.global_rate_limit.window_ms == 0
            || limits.max_in_flight == 0
            || limits.max_in_flight > MAX_IN_FLIGHT_CAPACITY
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidConfiguration,
                "Replay, rate-limit, or in-flight values are outside the supported bounds.",
            ));
        }

        Ok(Self {
            session: RwLock::new(None),
            global_attempts: Mutex::new(FixedWindowState::default()),
            in_flight: AtomicUsize::new(0),
            clock,
            limits,
        })
    }

    /// First-writer registration. A later call is idempotent only when both the
    /// token and the canonicalized policy are identical to the winner.
    pub fn register(
        &self,
        host_token: Option<&str>,
        policy: RegistrationPolicy,
    ) -> Result<RegistrationReceipt, BrokerError> {
        let _global_admission = self.acquire_global_admission()?;
        let token = HostToken::parse(host_token)?;
        let registration = CanonicalRegistration::parse(policy)?;
        let mut slot = self.session.write().map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::StateUnavailable,
                "The registration state is unavailable.",
            )
        })?;

        if let Some(existing) = slot.as_ref() {
            // Compare the token before comparing the non-secret policy so a
            // caller with the wrong token cannot use registration as a
            // configuration oracle.
            if !existing.token.matches(&token) {
                return Err(BrokerError::new(
                    BrokerErrorCode::InvalidHostToken,
                    "The host token is invalid.",
                ));
            }
            if existing.registration != registration {
                return Err(BrokerError::new(
                    BrokerErrorCode::RegistrationConflict,
                    "A different broker policy is already registered.",
                ));
            }
            return Ok(RegistrationReceipt {
                outcome: RegistrationOutcome::Idempotent,
                generation: existing.generation,
                module_id: existing.registration.module_id.clone(),
            });
        }

        let module_id = registration.module_id.clone();
        *slot = Some(RegisteredSession {
            token,
            registration,
            generation: INITIAL_SESSION_GENERATION,
            runtime: Mutex::new(RuntimeState::new(self.limits.replay_history_capacity)),
        });
        Ok(RegistrationReceipt {
            outcome: RegistrationOutcome::Registered,
            generation: INITIAL_SESSION_GENERATION,
            module_id,
        })
    }

    /// Rotates the credential and clears all session-scoped admission and replay
    /// state while preserving the canonical registration policy. The write lock
    /// waits for every old-generation executor lease to finish before returning.
    pub fn rotate(
        &self,
        current_host_token: Option<&str>,
        next_host_token: Option<&str>,
        expected_generation: u64,
    ) -> Result<RotationReceipt, BrokerError> {
        let _global_admission = self.acquire_global_admission()?;
        let current = HostToken::parse(current_host_token)?;
        let next = HostToken::parse(next_host_token)?;
        let mut slot = self.session.write().map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::StateUnavailable,
                "The registration state is unavailable.",
            )
        })?;
        let session = slot.as_mut().ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::NotRegistered,
                "The host broker is not registered.",
            )
        })?;

        if !session.token.matches(&current) {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidHostToken,
                "The current host token is invalid.",
            ));
        }
        if session.generation != expected_generation {
            return Err(BrokerError::new(
                BrokerErrorCode::StaleGeneration,
                "The expected session generation is stale.",
            ));
        }
        if session.token.matches(&next) {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidRotation,
                "The next host token must differ from the current token.",
            ));
        }

        let generation = session.generation.checked_add(1).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidRotation,
                "The session generation cannot be incremented.",
            )
        })?;
        session.token = next;
        session.generation = generation;
        session.runtime = Mutex::new(RuntimeState::new(self.limits.replay_history_capacity));

        Ok(RotationReceipt {
            outcome: RotationOutcome::Rotated,
            generation,
            module_id: session.registration.module_id.clone(),
        })
    }

    /// Authenticates, admits, authorizes, and executes one exact JSON request.
    /// The session read lease remains held until `execute` returns, so a
    /// successful rotation cannot overlap an old-generation native action.
    pub fn execute_json<T, F>(
        &self,
        host_token: Option<&str>,
        request_json: &str,
        execute: F,
    ) -> Result<T, BrokerError>
    where
        F: for<'session> FnOnce(AuthorizedAction<'session>) -> Result<T, BrokerError>,
    {
        let _global_admission = self.acquire_global_admission()?;
        let session_guard = self.session.read().map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::StateUnavailable,
                "The registration state is unavailable.",
            )
        })?;
        let session = session_guard.as_ref().ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::NotRegistered,
                "The host broker is not registered.",
            )
        })?;
        let candidate = HostToken::parse(host_token)?;
        if !session.token.matches(&candidate) {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidHostToken,
                "The host token is invalid.",
            ));
        }

        let now_ms = self.clock.now_millis();
        session
            .runtime
            .lock()
            .map_err(|_| {
                BrokerError::new(
                    BrokerErrorCode::StateUnavailable,
                    "The broker runtime state is unavailable.",
                )
            })?
            .consume_authenticated_attempt(now_ms, self.limits.rate_limit)?;

        let request = parse_request(request_json)?;
        session
            .runtime
            .lock()
            .map_err(|_| {
                BrokerError::new(
                    BrokerErrorCode::StateUnavailable,
                    "The broker runtime state is unavailable.",
                )
                .for_request(&request.request_id)
            })?
            .consume_request_id(&request.request_id, self.limits.replay_history_capacity)?;
        let pending = authorize_method(&session.registration, &request)?;

        let result = execute(AuthorizedAction {
            request_id: request.request_id,
            module_id: &session.registration.module_id,
            action: pending,
        });
        drop(session_guard);
        result
    }

    fn acquire_global_admission(&self) -> Result<InFlightPermit<'_>, BrokerError> {
        let acquired = self
            .in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < self.limits.max_in_flight).then_some(current + 1)
            })
            .is_ok();
        if !acquired {
            return Err(BrokerError::new(
                BrokerErrorCode::RateLimited,
                "The global in-flight broker request limit was exceeded.",
            ));
        }

        let permit = InFlightPermit {
            counter: &self.in_flight,
        };
        let now_ms = self.clock.now_millis();
        self.global_attempts
            .lock()
            .map_err(|_| {
                BrokerError::new(
                    BrokerErrorCode::StateUnavailable,
                    "The global broker admission state is unavailable.",
                )
            })?
            .consume(
                now_ms,
                self.limits.global_rate_limit,
                "The global broker attempt limit was exceeded.",
            )?;
        Ok(permit)
    }
}

impl HostBroker<SystemMonotonicClock> {
    pub fn production() -> Self {
        Self::new(SystemMonotonicClock::default(), BrokerLimits::default())
            .expect("default host broker limits are valid")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerRequest {
    request_id: String,
    method: String,
    payload: serde_json::Value,
}

fn parse_request(request_json: &str) -> Result<BrokerRequest, BrokerError> {
    if request_json.len() > MAX_REQUEST_JSON_BYTES {
        return Err(BrokerError::new(
            BrokerErrorCode::MalformedRequest,
            "The request JSON is oversized.",
        ));
    }

    let request: BrokerRequest = serde_json::from_str(request_json).map_err(|_| {
        BrokerError::new(
            BrokerErrorCode::MalformedRequest,
            "The request must match the exact broker envelope schema.",
        )
    })?;
    if !is_valid_identifier(&request.request_id, MAX_REQUEST_ID_BYTES) {
        return Err(BrokerError::new(
            BrokerErrorCode::MalformedRequest,
            "The request id is empty, oversized, or contains invalid characters.",
        ));
    }
    Ok(request)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyPayload {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SanitizePayload {
    html: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkFetchPayload {
    url: String,
}

fn authorize_method(
    registration: &CanonicalRegistration,
    request: &BrokerRequest,
) -> Result<BrokerAction, BrokerError> {
    let invalid_payload = || {
        BrokerError::new(
            BrokerErrorCode::InvalidPayload,
            "The payload must match the exact method schema.",
        )
        .for_request(&request.request_id)
    };

    match request.method.as_str() {
        "state.read" => {
            require_permission(registration, "state.read", &request.request_id)?;
            serde_json::from_value::<EmptyPayload>(request.payload.clone())
                .map_err(|_| invalid_payload())?;
            Ok(BrokerAction::StateRead)
        }
        "render.sanitize" => {
            require_permission(registration, "render.sanitize", &request.request_id)?;
            let payload = serde_json::from_value::<SanitizePayload>(request.payload.clone())
                .map_err(|_| invalid_payload())?;
            if payload.html.len() > MAX_SANITIZE_HTML_BYTES {
                return Err(BrokerError::new(
                    BrokerErrorCode::InvalidPayload,
                    "The HTML payload exceeds 64 KiB.",
                )
                .for_request(&request.request_id));
            }
            Ok(BrokerAction::RenderSanitize { html: payload.html })
        }
        "probe.increment" => {
            require_permission(registration, "probe.increment", &request.request_id)?;
            serde_json::from_value::<EmptyPayload>(request.payload.clone())
                .map_err(|_| invalid_payload())?;
            Ok(BrokerAction::ProbeIncrement)
        }
        "network.fetch" => {
            let payload = serde_json::from_value::<NetworkFetchPayload>(request.payload.clone())
                .map_err(|_| invalid_payload())?;
            if payload.url.is_empty() || payload.url.len() > MAX_NETWORK_URL_BYTES {
                return Err(invalid_payload());
            }

            debug_assert!(!registration.network_allowed);
            Err(BrokerError::new(
                BrokerErrorCode::NetworkDenied,
                "Network access is denied by the broker policy.",
            )
            .for_request(&request.request_id))
        }
        "secret.read" => {
            serde_json::from_value::<EmptyPayload>(request.payload.clone())
                .map_err(|_| invalid_payload())?;
            Err(BrokerError::new(
                BrokerErrorCode::PermissionDenied,
                "The requested method is not grantable to plugins.",
            )
            .for_request(&request.request_id))
        }
        _ => Err(BrokerError::new(
            BrokerErrorCode::UnknownMethod,
            "The requested broker method is not registered.",
        )
        .for_request(&request.request_id)),
    }
}

fn require_permission(
    registration: &CanonicalRegistration,
    permission: &str,
    request_id: &str,
) -> Result<(), BrokerError> {
    if registration.effective_permissions.contains(permission) {
        Ok(())
    } else {
        Err(BrokerError::new(
            BrokerErrorCode::PermissionDenied,
            "The permission is not both declared and approved.",
        )
        .for_request(request_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Barrier,
    };
    use std::thread;
    use std::time::{Duration, Instant};

    const TOKEN_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const TOKEN_B: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    const TOKEN_C: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[derive(Clone)]
    struct ManualClock(Arc<AtomicU64>);

    impl ManualClock {
        fn new(now_ms: u64) -> Self {
            Self(Arc::new(AtomicU64::new(now_ms)))
        }

        fn set(&self, now_ms: u64) {
            self.0.store(now_ms, Ordering::SeqCst);
        }
    }

    impl MonotonicClock for ManualClock {
        fn now_millis(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn policy(
        module_id: &str,
        manifest_permissions: &[&str],
        approved_permissions: &[&str],
    ) -> RegistrationPolicy {
        RegistrationPolicy {
            module_id: module_id.to_owned(),
            manifest_permissions: manifest_permissions
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            approved_permissions: approved_permissions
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn broker_with(
        clock: ManualClock,
        permissions: &[&str],
        limits: BrokerLimits,
    ) -> HostBroker<ManualClock> {
        let broker = HostBroker::new(clock, limits).unwrap();
        broker
            .register(
                Some(TOKEN_A),
                policy("module.alpha", permissions, permissions),
            )
            .unwrap();
        broker
    }

    fn default_broker(permissions: &[&str]) -> HostBroker<ManualClock> {
        broker_with(ManualClock::new(0), permissions, BrokerLimits::default())
    }

    fn request(request_id: &str, method: &str, payload: serde_json::Value) -> String {
        serde_json::json!({
            "request_id": request_id,
            "method": method,
            "payload": payload,
        })
        .to_string()
    }

    fn assert_code<T>(result: Result<T, BrokerError>, expected: BrokerErrorCode) -> BrokerError {
        let error = result.err().expect("expected broker error");
        assert_eq!(error.code, expected);
        error
    }

    fn wait_for_in_flight<C: MonotonicClock>(broker: &HostBroker<C>, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while broker.in_flight.load(Ordering::Acquire) != expected {
            assert!(
                Instant::now() < deadline,
                "broker did not reach {expected} in-flight operations"
            );
            thread::yield_now();
        }
    }

    struct OwnedAuthorizedAction {
        module_id: String,
        action: BrokerAction,
    }

    impl HostBroker<ManualClock> {
        fn authorize_json(
            &self,
            host_token: Option<&str>,
            request_json: &str,
        ) -> Result<OwnedAuthorizedAction, BrokerError> {
            self.execute_json(host_token, request_json, |authorized| {
                Ok(OwnedAuthorizedAction {
                    module_id: authorized.module_id.to_owned(),
                    action: authorized.action,
                })
            })
        }
    }

    #[test]
    fn registration_is_first_writer_cas_under_a_race() {
        let broker =
            Arc::new(HostBroker::new(ManualClock::new(0), BrokerLimits::default()).unwrap());
        let barrier = Arc::new(Barrier::new(8));
        let mut threads = Vec::new();

        for index in 0..8 {
            let broker = Arc::clone(&broker);
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                let module_id = format!("module.{index}");
                let candidate = policy(&module_id, &["state.read"], &["state.read"]);
                barrier.wait();
                (candidate.clone(), broker.register(Some(TOKEN_A), candidate))
            }));
        }

        let outcomes: Vec<_> = threads
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|(_, outcome)| {
                    matches!(outcome, Ok(receipt) if receipt.outcome == RegistrationOutcome::Registered)
                })
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|(_, outcome)| {
                    matches!(
                        outcome,
                        Err(BrokerError {
                            code: BrokerErrorCode::RegistrationConflict,
                            ..
                        })
                    )
                })
                .count(),
            7
        );

        let winner = outcomes
            .iter()
            .find(|(_, outcome)| {
                matches!(outcome, Ok(receipt) if receipt.outcome == RegistrationOutcome::Registered)
            })
            .unwrap()
            .0
            .clone();
        assert_eq!(
            broker
                .register(Some(TOKEN_A), winner)
                .map(|receipt| receipt.outcome),
            Ok(RegistrationOutcome::Idempotent)
        );
    }

    #[test]
    fn idempotence_requires_the_same_token_and_semantic_policy() {
        let broker = HostBroker::new(ManualClock::new(0), BrokerLimits::default()).unwrap();
        let first = policy(
            "module.alpha",
            &["state.read", "probe.increment"],
            &["probe.increment", "state.read"],
        );
        assert_eq!(
            broker
                .register(Some(TOKEN_A), first)
                .map(|receipt| receipt.outcome),
            Ok(RegistrationOutcome::Registered)
        );

        let reordered_with_duplicate = policy(
            "module.alpha",
            &["probe.increment", "state.read", "state.read"],
            &["state.read", "probe.increment"],
        );
        assert_eq!(
            broker
                .register(Some(TOKEN_A), reordered_with_duplicate)
                .map(|receipt| receipt.outcome),
            Ok(RegistrationOutcome::Idempotent)
        );
        assert_code(
            broker.register(
                Some(TOKEN_B),
                policy(
                    "module.alpha",
                    &["state.read", "probe.increment"],
                    &["probe.increment", "state.read"],
                ),
            ),
            BrokerErrorCode::InvalidHostToken,
        );
        assert_code(
            broker.register(
                Some(TOKEN_A),
                policy("module.other", &["state.read"], &["state.read"]),
            ),
            BrokerErrorCode::RegistrationConflict,
        );
    }

    #[test]
    fn token_format_is_strict_and_token_is_required_for_authorization() {
        let unregistered = HostBroker::new(ManualClock::new(0), BrokerLimits::default()).unwrap();
        assert_code(
            unregistered.register(
                None,
                policy("module.alpha", &["state.read"], &["state.read"]),
            ),
            BrokerErrorCode::MissingHostToken,
        );

        let broker = default_broker(&["state.read"]);
        let state_read = request("token-test", "state.read", serde_json::json!({}));

        assert_code(
            broker.authorize_json(None, &state_read),
            BrokerErrorCode::MissingHostToken,
        );
        assert_code(
            broker.authorize_json(Some(TOKEN_B), &state_read),
            BrokerErrorCode::InvalidHostToken,
        );
        assert_code(
            broker.authorize_json(Some(&TOKEN_A.to_uppercase()), &state_read),
            BrokerErrorCode::InvalidHostToken,
        );
    }

    #[test]
    fn registration_policy_deserialization_is_camel_case_and_strict() {
        let parsed: RegistrationPolicy = serde_json::from_value(serde_json::json!({
            "moduleId": "module.alpha",
            "manifestPermissions": ["state.read"],
            "approvedPermissions": ["state.read"]
        }))
        .unwrap();
        assert_eq!(parsed.module_id, "module.alpha");

        assert!(
            serde_json::from_value::<RegistrationPolicy>(serde_json::json!({
                "moduleId": "module.alpha",
                "manifestPermissions": ["state.read"],
                "approvedPermissions": ["state.read"],
                "networkAllowed": true
            }))
            .is_err()
        );
    }

    #[test]
    fn request_and_payload_schemas_deny_unknown_fields_and_module_id() {
        let broker = default_broker(&["state.read", "render.sanitize"]);
        let unknown_envelope = serde_json::json!({
            "request_id": "unknown-envelope",
            "method": "state.read",
            "payload": {},
            "surprise": true,
        })
        .to_string();
        let injected_module = serde_json::json!({
            "request_id": "module-injection",
            "module_id": "module.attacker",
            "method": "state.read",
            "payload": {},
        })
        .to_string();
        let unknown_payload = request(
            "unknown-payload",
            "render.sanitize",
            serde_json::json!({"html": "safe", "surprise": true}),
        );

        assert_code(
            broker.authorize_json(Some(TOKEN_A), &unknown_envelope),
            BrokerErrorCode::MalformedRequest,
        );
        assert_code(
            broker.authorize_json(Some(TOKEN_A), &injected_module),
            BrokerErrorCode::MalformedRequest,
        );
        assert_code(
            broker.authorize_json(Some(TOKEN_A), &unknown_payload),
            BrokerErrorCode::InvalidPayload,
        );
    }

    #[test]
    fn effective_permission_is_manifest_intersection_approved() {
        for (manifest, approved) in [
            (vec!["state.read"], Vec::<&str>::new()),
            (Vec::<&str>::new(), vec!["state.read"]),
        ] {
            let broker = HostBroker::new(ManualClock::new(0), BrokerLimits::default()).unwrap();
            broker
                .register(Some(TOKEN_A), policy("module.alpha", &manifest, &approved))
                .unwrap();
            assert_code(
                broker.authorize_json(
                    Some(TOKEN_A),
                    &request("intersection-deny", "state.read", serde_json::json!({})),
                ),
                BrokerErrorCode::PermissionDenied,
            );
        }

        let broker = default_broker(&["state.read"]);
        let action = broker
            .authorize_json(
                Some(TOKEN_A),
                &request("intersection-allow", "state.read", serde_json::json!({})),
            )
            .unwrap();
        assert_eq!(action.module_id, "module.alpha");
        assert_eq!(action.action, BrokerAction::StateRead);
    }

    #[test]
    fn network_and_secret_methods_are_fail_closed() {
        let broker = default_broker(&["network.fetch", "secret.read"]);
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request(
                    "network-denied",
                    "network.fetch",
                    serde_json::json!({"url": "https://example.invalid"}),
                ),
            ),
            BrokerErrorCode::NetworkDenied,
        );
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("secret-denied", "secret.read", serde_json::json!({})),
            ),
            BrokerErrorCode::PermissionDenied,
        );
    }

    #[test]
    fn sanitize_html_is_bounded_by_decoded_utf8_bytes() {
        let broker = default_broker(&["render.sanitize"]);
        let at_limit = "x".repeat(MAX_SANITIZE_HTML_BYTES);
        let action = broker
            .authorize_json(
                Some(TOKEN_A),
                &request(
                    "html-at-limit",
                    "render.sanitize",
                    serde_json::json!({"html": at_limit}),
                ),
            )
            .unwrap();
        assert!(matches!(
            action.action,
            BrokerAction::RenderSanitize { ref html }
                if html.len() == MAX_SANITIZE_HTML_BYTES
        ));

        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request(
                    "html-too-large",
                    "render.sanitize",
                    serde_json::json!({"html": "x".repeat(MAX_SANITIZE_HTML_BYTES + 1)}),
                ),
            ),
            BrokerErrorCode::InvalidPayload,
        );
    }

    #[test]
    fn authorized_methods_map_to_actions_without_executing_them() {
        let broker = default_broker(&["state.read", "render.sanitize", "probe.increment"]);

        assert_eq!(
            broker
                .authorize_json(
                    Some(TOKEN_A),
                    &request("map-state", "state.read", serde_json::json!({})),
                )
                .unwrap()
                .action,
            BrokerAction::StateRead
        );
        assert_eq!(
            broker
                .authorize_json(
                    Some(TOKEN_A),
                    &request(
                        "map-sanitize",
                        "render.sanitize",
                        serde_json::json!({"html": "<b>safe</b>"}),
                    ),
                )
                .unwrap()
                .action,
            BrokerAction::RenderSanitize {
                html: "<b>safe</b>".to_owned()
            }
        );
        assert_eq!(
            broker
                .authorize_json(
                    Some(TOKEN_A),
                    &request("map-probe", "probe.increment", serde_json::json!({})),
                )
                .unwrap()
                .action,
            BrokerAction::ProbeIncrement
        );
    }

    #[test]
    fn global_attempt_quota_counts_registration_and_missing_invalid_request_tokens() {
        let clock = ManualClock::new(0);
        let limits = BrokerLimits {
            global_rate_limit: RateLimit {
                max_requests: 3,
                window_ms: 100,
            },
            rate_limit: RateLimit {
                max_requests: 10,
                window_ms: 100,
            },
            ..BrokerLimits::default()
        };
        let broker = broker_with(clock.clone(), &["state.read"], limits);
        let state = request("global-valid", "state.read", serde_json::json!({}));

        assert_code(
            broker.authorize_json(None, &state),
            BrokerErrorCode::MissingHostToken,
        );
        assert_code(
            broker.authorize_json(Some(TOKEN_B), &state),
            BrokerErrorCode::InvalidHostToken,
        );
        assert_code(
            broker.authorize_json(Some(TOKEN_A), &state),
            BrokerErrorCode::RateLimited,
        );

        clock.set(100);
        assert_eq!(
            broker.authorize_json(Some(TOKEN_A), &state).unwrap().action,
            BrokerAction::StateRead
        );
    }

    #[test]
    fn invalid_lifecycle_attempts_consume_the_shared_global_quota() {
        let clock = ManualClock::new(0);
        let limits = BrokerLimits {
            global_rate_limit: RateLimit {
                max_requests: 2,
                window_ms: 100,
            },
            ..BrokerLimits::default()
        };
        let broker = HostBroker::new(clock.clone(), limits).unwrap();
        let registration = policy("module.alpha", &["state.read"], &["state.read"]);

        assert_code(
            broker.register(None, registration.clone()),
            BrokerErrorCode::MissingHostToken,
        );
        assert_code(
            broker.rotate(None, Some(TOKEN_B), 1),
            BrokerErrorCode::MissingHostToken,
        );
        assert_code(
            broker.register(Some(TOKEN_A), registration.clone()),
            BrokerErrorCode::RateLimited,
        );

        clock.set(100);
        assert_eq!(
            broker
                .register(Some(TOKEN_A), registration)
                .unwrap()
                .outcome,
            RegistrationOutcome::Registered
        );
    }

    #[test]
    fn lifecycle_calls_do_not_consume_the_authenticated_request_quota() {
        let limits = BrokerLimits {
            rate_limit: RateLimit {
                max_requests: 1,
                window_ms: 1_000,
            },
            global_rate_limit: RateLimit {
                max_requests: 20,
                window_ms: 1_000,
            },
            ..BrokerLimits::default()
        };
        let broker = broker_with(ManualClock::new(0), &["state.read"], limits);
        let registration = policy("module.alpha", &["state.read"], &["state.read"]);

        assert_eq!(
            broker
                .register(Some(TOKEN_A), registration)
                .unwrap()
                .outcome,
            RegistrationOutcome::Idempotent
        );
        assert_code(
            broker.rotate(Some(TOKEN_B), Some(TOKEN_C), 1),
            BrokerErrorCode::InvalidHostToken,
        );

        assert_eq!(
            broker
                .authorize_json(
                    Some(TOKEN_A),
                    &request("request-only-quota", "state.read", serde_json::json!({})),
                )
                .unwrap()
                .action,
            BrokerAction::StateRead
        );
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("request-over-quota", "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::RateLimited,
        );
    }

    #[test]
    fn authenticated_quota_counts_parse_method_and_replay_denials() {
        let limits = BrokerLimits {
            rate_limit: RateLimit {
                max_requests: 4,
                window_ms: 1_000,
            },
            global_rate_limit: RateLimit {
                max_requests: 20,
                window_ms: 1_000,
            },
            ..BrokerLimits::default()
        };
        let broker = broker_with(ManualClock::new(0), &["state.read"], limits);
        let accepted = request("accepted", "state.read", serde_json::json!({}));

        assert_code(
            broker.authorize_json(Some(TOKEN_A), "not-json"),
            BrokerErrorCode::MalformedRequest,
        );
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("unknown", "unknown.method", serde_json::json!({})),
            ),
            BrokerErrorCode::UnknownMethod,
        );
        broker.authorize_json(Some(TOKEN_A), &accepted).unwrap();
        assert_code(
            broker.authorize_json(Some(TOKEN_A), &accepted),
            BrokerErrorCode::ReplayedRequest,
        );
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("over-quota", "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::RateLimited,
        );
    }

    #[test]
    fn exact_request_id_is_consumed_before_method_authorization() {
        let broker = default_broker(&["state.read", "network.fetch"]);
        let request_id = "denied-then-reused";
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request(request_id, "unknown.method", serde_json::json!({})),
            ),
            BrokerErrorCode::UnknownMethod,
        );
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request(request_id, "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::ReplayedRequest,
        );

        let network_id = "network-denied-then-reused";
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request(
                    network_id,
                    "network.fetch",
                    serde_json::json!({"url": "https://example.invalid"}),
                ),
            ),
            BrokerErrorCode::NetworkDenied,
        );
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request(network_id, "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::ReplayedRequest,
        );
    }

    #[test]
    fn global_in_flight_cap_is_held_through_executor_completion() {
        let limits = BrokerLimits {
            max_in_flight: 1,
            rate_limit: RateLimit {
                max_requests: 10,
                window_ms: 1_000,
            },
            global_rate_limit: RateLimit {
                max_requests: 10,
                window_ms: 1_000,
            },
            ..BrokerLimits::default()
        };
        let broker = Arc::new(broker_with(ManualClock::new(0), &["state.read"], limits));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let worker = {
            let broker = Arc::clone(&broker);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            thread::spawn(move || {
                broker.execute_json(
                    Some(TOKEN_A),
                    &request("held", "state.read", serde_json::json!({})),
                    |authorized| {
                        entered.wait();
                        release.wait();
                        Ok(authorized.action)
                    },
                )
            })
        };

        entered.wait();
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("blocked", "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::RateLimited,
        );
        release.wait();
        assert_eq!(worker.join().unwrap().unwrap(), BrokerAction::StateRead);

        assert_eq!(
            broker
                .authorize_json(
                    Some(TOKEN_A),
                    &request("after-release", "state.read", serde_json::json!({})),
                )
                .unwrap()
                .action,
            BrokerAction::StateRead
        );
    }

    #[test]
    fn register_holds_global_admission_while_waiting_for_the_session_lock() {
        let limits = BrokerLimits {
            max_in_flight: 2,
            rate_limit: RateLimit {
                max_requests: 20,
                window_ms: 1_000,
            },
            global_rate_limit: RateLimit {
                max_requests: 20,
                window_ms: 1_000,
            },
            ..BrokerLimits::default()
        };
        let broker = Arc::new(broker_with(ManualClock::new(0), &["state.read"], limits));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let request_worker = {
            let broker = Arc::clone(&broker);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            thread::spawn(move || {
                broker.execute_json(
                    Some(TOKEN_A),
                    &request("register-lock-holder", "state.read", serde_json::json!({})),
                    |authorized| {
                        entered.wait();
                        release.wait();
                        Ok(authorized.action)
                    },
                )
            })
        };
        entered.wait();

        let registration_worker = {
            let broker = Arc::clone(&broker);
            thread::spawn(move || {
                broker.register(
                    Some(TOKEN_A),
                    policy("module.alpha", &["state.read"], &["state.read"]),
                )
            })
        };
        wait_for_in_flight(&broker, 2);

        let (overflow_tx, overflow_rx) = mpsc::channel();
        let overflow_worker = {
            let broker = Arc::clone(&broker);
            thread::spawn(move || {
                overflow_tx
                    .send(broker.rotate(None, Some(TOKEN_B), 1))
                    .unwrap();
            })
        };
        let overflow_result = overflow_rx.recv_timeout(Duration::from_secs(1));

        release.wait();
        assert_eq!(
            request_worker.join().unwrap().unwrap(),
            BrokerAction::StateRead
        );
        assert_eq!(
            registration_worker.join().unwrap().unwrap().outcome,
            RegistrationOutcome::Idempotent
        );
        overflow_worker.join().unwrap();
        assert_code(
            overflow_result.expect("overflow rotation must fail before the session lock"),
            BrokerErrorCode::RateLimited,
        );
    }

    #[test]
    fn rotate_holds_global_admission_while_waiting_for_the_session_lock() {
        let limits = BrokerLimits {
            max_in_flight: 2,
            rate_limit: RateLimit {
                max_requests: 20,
                window_ms: 1_000,
            },
            global_rate_limit: RateLimit {
                max_requests: 20,
                window_ms: 1_000,
            },
            ..BrokerLimits::default()
        };
        let broker = Arc::new(broker_with(ManualClock::new(0), &["state.read"], limits));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let request_worker = {
            let broker = Arc::clone(&broker);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            thread::spawn(move || {
                broker.execute_json(
                    Some(TOKEN_A),
                    &request("rotate-lock-holder", "state.read", serde_json::json!({})),
                    |authorized| {
                        entered.wait();
                        release.wait();
                        Ok(authorized.action)
                    },
                )
            })
        };
        entered.wait();

        let rotation_worker = {
            let broker = Arc::clone(&broker);
            thread::spawn(move || broker.rotate(Some(TOKEN_A), Some(TOKEN_B), 1))
        };
        wait_for_in_flight(&broker, 2);

        let (overflow_tx, overflow_rx) = mpsc::channel();
        let overflow_worker = {
            let broker = Arc::clone(&broker);
            thread::spawn(move || {
                overflow_tx
                    .send(broker.register(
                        Some(TOKEN_A),
                        policy("INVALID-MODULE", &["state.read"], &["state.read"]),
                    ))
                    .unwrap();
            })
        };
        let overflow_result = overflow_rx.recv_timeout(Duration::from_secs(1));

        release.wait();
        assert_eq!(
            request_worker.join().unwrap().unwrap(),
            BrokerAction::StateRead
        );
        assert_eq!(rotation_worker.join().unwrap().unwrap().generation, 2);
        overflow_worker.join().unwrap();
        assert_code(
            overflow_result.expect("overflow registration must fail before the session lock"),
            BrokerErrorCode::RateLimited,
        );
    }

    #[test]
    fn rotation_requires_current_token_and_expected_generation() {
        let broker = default_broker(&["state.read"]);

        assert_code(
            broker.rotate(Some(TOKEN_B), Some(TOKEN_C), 1),
            BrokerErrorCode::InvalidHostToken,
        );
        assert_code(
            broker.rotate(Some(TOKEN_A), Some(TOKEN_B), 2),
            BrokerErrorCode::StaleGeneration,
        );
        assert_code(
            broker.rotate(Some(TOKEN_A), Some(TOKEN_A), 1),
            BrokerErrorCode::InvalidRotation,
        );

        let receipt = broker.rotate(Some(TOKEN_A), Some(TOKEN_B), 1).unwrap();
        assert_eq!(receipt.outcome, RotationOutcome::Rotated);
        assert_eq!(receipt.generation, 2);
        assert_eq!(receipt.module_id, "module.alpha");
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("stale-token", "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::InvalidHostToken,
        );
        assert_eq!(
            broker
                .authorize_json(
                    Some(TOKEN_B),
                    &request("fresh-token", "state.read", serde_json::json!({})),
                )
                .unwrap()
                .action,
            BrokerAction::StateRead
        );
    }

    #[test]
    fn rotation_resets_history_and_authenticated_rate_state() {
        let limits = BrokerLimits {
            replay_history_capacity: 1,
            rate_limit: RateLimit {
                max_requests: 2,
                window_ms: 1_000,
            },
            global_rate_limit: RateLimit {
                max_requests: 20,
                window_ms: 1_000,
            },
            ..BrokerLimits::default()
        };
        let broker = broker_with(ManualClock::new(0), &["state.read"], limits);
        let reused = request("generation-scoped", "state.read", serde_json::json!({}));

        broker.authorize_json(Some(TOKEN_A), &reused).unwrap();
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("history-full", "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::SessionExhausted,
        );

        broker.rotate(Some(TOKEN_A), Some(TOKEN_B), 1).unwrap();
        assert_eq!(
            broker
                .authorize_json(Some(TOKEN_B), &reused)
                .unwrap()
                .action,
            BrokerAction::StateRead
        );
    }

    #[test]
    fn concurrent_rotation_is_single_winner() {
        let broker = Arc::new(default_broker(&["state.read"]));
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for next in [TOKEN_B, TOKEN_C] {
            let broker = Arc::clone(&broker);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                broker.rotate(Some(TOKEN_A), Some(next), 1)
            }));
        }
        barrier.wait();

        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(BrokerError {
                            code: BrokerErrorCode::InvalidHostToken,
                            ..
                        })
                    )
                })
                .count(),
            1
        );
    }

    #[test]
    fn rotation_waits_for_old_generation_executor_lease() {
        let broker = Arc::new(default_broker(&["probe.increment"]));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let side_effects = Arc::new(AtomicU64::new(0));

        let request_worker = {
            let broker = Arc::clone(&broker);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            let side_effects = Arc::clone(&side_effects);
            thread::spawn(move || {
                broker.execute_json(
                    Some(TOKEN_A),
                    &request("old-generation", "probe.increment", serde_json::json!({})),
                    |_authorized| {
                        entered.wait();
                        release.wait();
                        side_effects.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )
            })
        };
        entered.wait();

        let (rotation_tx, rotation_rx) = mpsc::channel();
        let rotation_started = Arc::new(Barrier::new(2));
        let rotation_worker = {
            let broker = Arc::clone(&broker);
            let rotation_started = Arc::clone(&rotation_started);
            thread::spawn(move || {
                rotation_started.wait();
                let result = broker.rotate(Some(TOKEN_A), Some(TOKEN_B), 1);
                rotation_tx.send(result).unwrap();
            })
        };
        rotation_started.wait();
        assert!(rotation_rx.recv_timeout(Duration::from_millis(50)).is_err());

        release.wait();
        request_worker.join().unwrap().unwrap();
        let receipt = rotation_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        rotation_worker.join().unwrap();
        assert_eq!(receipt.generation, 2);
        assert_eq!(side_effects.load(Ordering::SeqCst), 1);

        assert_code(
            broker.execute_json(
                Some(TOKEN_A),
                &request(
                    "late-old-generation",
                    "probe.increment",
                    serde_json::json!({}),
                ),
                |_authorized| {
                    side_effects.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            ),
            BrokerErrorCode::InvalidHostToken,
        );
        assert_eq!(side_effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn replay_history_is_bounded_and_fails_closed() {
        let limits = BrokerLimits {
            replay_history_capacity: 1,
            rate_limit: RateLimit {
                max_requests: 10,
                window_ms: 1_000,
            },
            ..BrokerLimits::default()
        };
        let broker = broker_with(ManualClock::new(0), &["state.read"], limits);
        let first = request("one-shot", "state.read", serde_json::json!({}));
        broker.authorize_json(Some(TOKEN_A), &first).unwrap();

        assert_code(
            broker.authorize_json(Some(TOKEN_A), &first),
            BrokerErrorCode::ReplayedRequest,
        );
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("history-full", "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::SessionExhausted,
        );
    }

    #[test]
    fn fixed_window_rate_limit_uses_the_injected_monotonic_clock() {
        let clock = ManualClock::new(0);
        let limits = BrokerLimits {
            replay_history_capacity: 16,
            rate_limit: RateLimit {
                max_requests: 2,
                window_ms: 100,
            },
            ..BrokerLimits::default()
        };
        let broker = broker_with(clock.clone(), &["state.read"], limits);

        for request_id in ["window-one", "window-two"] {
            broker
                .authorize_json(
                    Some(TOKEN_A),
                    &request(request_id, "state.read", serde_json::json!({})),
                )
                .unwrap();
        }
        assert_code(
            broker.authorize_json(
                Some(TOKEN_A),
                &request("window-three", "state.read", serde_json::json!({})),
            ),
            BrokerErrorCode::RateLimited,
        );

        clock.set(100);
        broker
            .authorize_json(
                Some(TOKEN_A),
                &request("next-window", "state.read", serde_json::json!({})),
            )
            .unwrap();
    }
}
