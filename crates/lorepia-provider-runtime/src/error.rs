use std::fmt;

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
    retriable: bool,
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
            retriable: false,
        }
    }

    pub(crate) fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    pub(crate) fn retriable(mut self, retriable: bool) -> Self {
        self.retriable = retriable;
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
        self.retriable
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
