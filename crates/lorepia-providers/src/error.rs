use std::fmt;

use crate::{Capability, ProviderId};

pub type Result<T> = std::result::Result<T, ProviderConfigError>;

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderConfigError {
    ProviderOptionsMismatch {
        request_provider: ProviderId,
        options_provider: ProviderId,
    },
    MissingField(&'static str),
    EmptyField(&'static str),
    FieldTooLong {
        field: &'static str,
        max_bytes: usize,
    },
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    OutOfRange {
        field: &'static str,
        min: f64,
        max: f64,
    },
    UnsupportedOption {
        provider: ProviderId,
        capability: Capability,
    },
    TooManyItems {
        field: &'static str,
        max: usize,
    },
    PayloadTooLarge {
        field: &'static str,
        max_bytes: usize,
    },
    UnsafeExtraParameter(String),
    ExtraParameterCollision(String),
    ExtraParameterTooDeep {
        key: String,
        max_depth: usize,
    },
    MissingConversation,
}

impl fmt::Display for ProviderConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProviderOptionsMismatch {
                request_provider,
                options_provider,
            } => write!(
                formatter,
                "provider options mismatch: request={request_provider:?}, options={options_provider:?}"
            ),
            Self::MissingField(field) => write!(formatter, "missing required field: {field}"),
            Self::EmptyField(field) => write!(formatter, "field must not be empty: {field}"),
            Self::FieldTooLong { field, max_bytes } => {
                write!(formatter, "field exceeds {max_bytes} bytes: {field}")
            }
            Self::InvalidField { field, reason } => {
                write!(formatter, "invalid field {field}: {reason}")
            }
            Self::OutOfRange { field, min, max } => {
                write!(formatter, "field {field} must be between {min} and {max}")
            }
            Self::UnsupportedOption {
                provider,
                capability,
            } => write!(
                formatter,
                "provider {provider:?} does not support option {capability:?}"
            ),
            Self::TooManyItems { field, max } => {
                write!(formatter, "field {field} exceeds {max} items")
            }
            Self::PayloadTooLarge { field, max_bytes } => {
                write!(formatter, "field {field} exceeds {max_bytes} bytes")
            }
            Self::UnsafeExtraParameter(key) => {
                write!(formatter, "unsafe extra parameter rejected: {key}")
            }
            Self::ExtraParameterCollision(key) => {
                write!(
                    formatter,
                    "extra parameter collides with compiled request: {key}"
                )
            }
            Self::ExtraParameterTooDeep { key, max_depth } => {
                write!(formatter, "extra parameter {key} exceeds depth {max_depth}")
            }
            Self::MissingConversation => write!(formatter, "request has no non-system message"),
        }
    }
}

impl std::error::Error for ProviderConfigError {}
