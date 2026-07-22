use std::fmt;

pub type Result<T> = std::result::Result<T, PersonaError>;

#[derive(Debug)]
pub enum PersonaError {
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
    AlreadyExists {
        kind: &'static str,
        id: String,
    },
    NotFound {
        kind: &'static str,
        id: String,
    },
    RevisionConflict {
        persona_id: String,
        expected: u64,
        actual: u64,
    },
    PersonaInUse {
        persona_id: String,
        scope: String,
    },
    BindingMismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    RevisionOverflow,
    State(String),
    Json(serde_json::Error),
}

impl PersonaError {
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

impl fmt::Display for PersonaError {
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
            Self::AlreadyExists { kind, id } => write!(formatter, "{kind} already exists: {id}"),
            Self::NotFound { kind, id } => write!(formatter, "{kind} was not found: {id}"),
            Self::RevisionConflict {
                persona_id,
                expected,
                actual,
            } => write!(
                formatter,
                "persona {persona_id} revision conflict: expected {expected}, actual {actual}"
            ),
            Self::PersonaInUse { persona_id, scope } => write!(
                formatter,
                "persona {persona_id} is still referenced by {scope}"
            ),
            Self::BindingMismatch {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "persona binding {field} mismatch: expected {expected}, actual {actual}"
            ),
            Self::RevisionOverflow => formatter.write_str("persona revision overflowed"),
            Self::State(reason) => write!(formatter, "invalid persona state: {reason}"),
            Self::Json(error) => write!(formatter, "persona state JSON failed: {error}"),
        }
    }
}

impl std::error::Error for PersonaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for PersonaError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
