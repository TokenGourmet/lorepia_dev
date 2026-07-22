use std::{fmt, time::Duration};

/// A typed hint for a *new*, caller-authorized request.
///
/// The provider runtime never replays a streaming POST automatically. In
/// particular, callers must not interpret this as permission to resume or
/// duplicate a stream which has already emitted events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryDecision {
    Never,
    RetryAfter {
        delay: Duration,
    },
    ExponentialBackoff {
        initial_delay: Duration,
        maximum_delay: Duration,
    },
}

impl RetryDecision {
    pub(crate) const fn exponential() -> Self {
        Self::ExponentialBackoff {
            initial_delay: Duration::from_secs(1),
            maximum_delay: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeErrorKind {
    InvalidRequest,
    InvalidEndpoint,
    UnsafeEndpoint,
    DnsResolution,
    CredentialMismatch,
    InvalidCredential,
    Http,
    HttpStatus,
    UnexpectedContentType,
    StreamTooLarge,
    StreamProtocol,
    Provider,
    Cancelled,
    ConsumerClosed,
    Timeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeError {
    kind: RuntimeErrorKind,
    code: String,
    message: String,
    http_status: Option<u16>,
    retry_decision: RetryDecision,
}

impl RuntimeError {
    pub(crate) fn new(
        kind: RuntimeErrorKind,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            code: truncate_utf8(code.into(), 64),
            message: truncate_utf8(message.into(), 512),
            http_status: None,
            retry_decision: RetryDecision::Never,
        }
    }

    pub(crate) fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    pub(crate) fn retriable(mut self, retriable: bool) -> Self {
        self.retry_decision = if retriable {
            RetryDecision::exponential()
        } else {
            RetryDecision::Never
        };
        self
    }

    pub(crate) fn with_retry_decision(mut self, retry_decision: RetryDecision) -> Self {
        self.retry_decision = retry_decision;
        self
    }

    #[must_use]
    pub const fn kind(&self) -> RuntimeErrorKind {
        self.kind
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    #[must_use]
    pub const fn is_retriable(&self) -> bool {
        !matches!(self.retry_decision, RetryDecision::Never)
    }

    #[must_use]
    pub const fn retry_decision(&self) -> RetryDecision {
        self.retry_decision
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RuntimeError {}

pub type Result<T> = std::result::Result<T, RuntimeError>;

pub(crate) fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
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
