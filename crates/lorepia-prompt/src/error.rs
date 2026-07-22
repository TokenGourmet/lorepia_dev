use std::fmt;

use lorepia_providers::ProviderConfigError;

pub type Result<T> = std::result::Result<T, PromptError>;
pub type ExactTokenResult<T> = std::result::Result<T, ExactTokenCountError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactTokenCountErrorKind {
    Unavailable,
    InvalidRequest,
    Transport,
    HttpStatus,
    InvalidResponse,
    Timeout,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactTokenCountError {
    kind: ExactTokenCountErrorKind,
    code: String,
    message: String,
    http_status: Option<u16>,
    retriable: bool,
}

impl ExactTokenCountError {
    #[must_use]
    pub fn new(
        kind: ExactTokenCountErrorKind,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
            http_status: None,
            retriable: false,
        }
    }

    #[must_use]
    pub const fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    #[must_use]
    pub const fn retriable(mut self, retriable: bool) -> Self {
        self.retriable = retriable;
        self
    }

    #[must_use]
    pub const fn kind(&self) -> ExactTokenCountErrorKind {
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

impl fmt::Display for ExactTokenCountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ExactTokenCountError {}

#[derive(Debug)]
pub enum PromptError {
    InvalidField { field: String, reason: String },
    TooManyItems { field: String, max: usize },
    PayloadTooLarge { field: String, max_bytes: usize },
    UnknownVariable(String),
    UnsupportedFeature { feature: String, reason: String },
    Regex { rule: String, reason: String },
    Import(String),
    ExactTokenCount(ExactTokenCountError),
    Provider(ProviderConfigError),
    Json(serde_json::Error),
}

impl PromptError {
    pub(crate) fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidField {
            field: field.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn too_many(field: impl Into<String>, max: usize) -> Self {
        Self::TooManyItems {
            field: field.into(),
            max,
        }
    }

    pub(crate) fn too_large(field: impl Into<String>, max_bytes: usize) -> Self {
        Self::PayloadTooLarge {
            field: field.into(),
            max_bytes,
        }
    }
}

impl fmt::Display for PromptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField { field, reason } => {
                write!(formatter, "invalid field {field}: {reason}")
            }
            Self::TooManyItems { field, max } => {
                write!(formatter, "field {field} exceeds {max} items")
            }
            Self::PayloadTooLarge { field, max_bytes } => {
                write!(formatter, "field {field} exceeds {max_bytes} bytes")
            }
            Self::UnknownVariable(name) => write!(formatter, "unknown prompt variable: {name}"),
            Self::UnsupportedFeature { feature, reason } => {
                write!(formatter, "unsupported prompt feature {feature}: {reason}")
            }
            Self::Regex { rule, reason } => {
                write!(formatter, "regex rule {rule} failed: {reason}")
            }
            Self::Import(reason) => write!(formatter, "invalid prompt import: {reason}"),
            Self::ExactTokenCount(error) => write!(formatter, "exact token count failed: {error}"),
            Self::Provider(error) => write!(formatter, "provider request rejected: {error}"),
            Self::Json(error) => write!(formatter, "prompt JSON failed: {error}"),
        }
    }
}

impl std::error::Error for PromptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Provider(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::ExactTokenCount(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProviderConfigError> for PromptError {
    fn from(value: ProviderConfigError) -> Self {
        Self::Provider(value)
    }
}

impl From<ExactTokenCountError> for PromptError {
    fn from(value: ExactTokenCountError) -> Self {
        Self::ExactTokenCount(value)
    }
}

impl From<serde_json::Error> for PromptError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
