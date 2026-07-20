use std::fmt;

use lorepia_prompt::PromptError;

pub type Result<T> = std::result::Result<T, MemoryError>;

#[derive(Debug)]
pub enum MemoryError {
    InvalidField {
        field: String,
        reason: String,
    },
    TooManyItems {
        field: String,
        max: usize,
    },
    PayloadTooLarge {
        field: String,
        max_bytes: usize,
    },
    RevisionConflict {
        expected: u64,
        actual: u64,
    },
    BindingMismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    DuplicateId {
        field: &'static str,
        id: String,
    },
    RetrievalUnavailable(&'static str),
    RevisionOverflow,
    State(String),
    Regex(&'static str),
    Prompt(PromptError),
    Json(serde_json::Error),
}

impl MemoryError {
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

    pub(crate) fn state(reason: impl Into<String>) -> Self {
        Self::State(reason.into())
    }
}

impl fmt::Display for MemoryError {
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
            Self::RevisionConflict { expected, actual } => write!(
                formatter,
                "memory preset revision conflict: expected {expected}, actual {actual}"
            ),
            Self::BindingMismatch {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "memory binding {field} mismatch: expected {expected}, actual {actual}"
            ),
            Self::DuplicateId { field, id } => {
                write!(formatter, "duplicate {field}: {id}")
            }
            Self::RetrievalUnavailable(reason) => {
                write!(formatter, "memory retrieval unavailable: {reason}")
            }
            Self::RevisionOverflow => formatter.write_str("memory revision overflowed"),
            Self::State(reason) => write!(formatter, "invalid memory state: {reason}"),
            Self::Regex(reason) => write!(formatter, "memory regex rejected: {reason}"),
            Self::Prompt(error) => write!(formatter, "memory prompt processing failed: {error}"),
            Self::Json(error) => write!(formatter, "memory state JSON failed: {error}"),
        }
    }
}

impl std::error::Error for MemoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Prompt(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PromptError> for MemoryError {
    fn from(value: PromptError) -> Self {
        Self::Prompt(value)
    }
}

impl From<serde_json::Error> for MemoryError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
