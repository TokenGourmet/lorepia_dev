use std::fmt;

pub type Result<T> = std::result::Result<T, LorebookError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitKind {
    LiteralMatchEvents,
    RegexEvaluations,
    RegexScanBytes,
    RegexMatches,
}

pub enum LorebookError {
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    TooManyItems {
        field: &'static str,
        max: usize,
    },
    PayloadTooLarge {
        field: &'static str,
        max_bytes: usize,
    },
    DuplicateEntryId,
    InvalidRegex {
        entry_index: usize,
    },
    SearchLimitExceeded {
        limit: LimitKind,
    },
    ImportSyntax,
    ImportSchema,
    UnsupportedImportVersion,
    Serialization,
}

impl LorebookError {
    pub(crate) const fn invalid(field: &'static str, reason: &'static str) -> Self {
        Self::InvalidField { field, reason }
    }

    pub(crate) const fn too_many(field: &'static str, max: usize) -> Self {
        Self::TooManyItems { field, max }
    }

    pub(crate) const fn too_large(field: &'static str, max_bytes: usize) -> Self {
        Self::PayloadTooLarge { field, max_bytes }
    }
}

impl fmt::Debug for LorebookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Keep Debug log-safe: never retain or print imported patterns, lore
        // text, chat text, or JSON snippets.
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for LorebookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField { field, reason } => {
                write!(formatter, "invalid lorebook field {field}: {reason}")
            }
            Self::TooManyItems { field, max } => {
                write!(formatter, "lorebook field {field} exceeds {max} items")
            }
            Self::PayloadTooLarge { field, max_bytes } => {
                write!(
                    formatter,
                    "lorebook field {field} exceeds {max_bytes} bytes"
                )
            }
            Self::DuplicateEntryId => formatter.write_str("duplicate lorebook entry id"),
            Self::InvalidRegex { entry_index } => {
                write!(
                    formatter,
                    "invalid lorebook regex at entry index {entry_index}"
                )
            }
            Self::SearchLimitExceeded { limit } => {
                write!(formatter, "lorebook search limit exceeded: {limit:?}")
            }
            Self::ImportSyntax => formatter.write_str("lorebook import is not valid JSON"),
            Self::ImportSchema => {
                formatter.write_str("lorebook import does not match the closed schema")
            }
            Self::UnsupportedImportVersion => {
                formatter.write_str("unsupported lorebook import version")
            }
            Self::Serialization => formatter.write_str("lorebook serialization failed"),
        }
    }
}

impl std::error::Error for LorebookError {}
