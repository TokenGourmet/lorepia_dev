use std::{error::Error, fmt};

pub type Result<T> = std::result::Result<T, SafetyError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafetyError {
    InvalidField(&'static str),
    FieldTooLarge(&'static str),
    ContentConsentRequired,
    ContentConsentMismatch,
    TooManyDiagnosticCodes,
    DiagnosticBundleTooLarge,
    SerializationFailed,
}

impl SafetyError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidField(_) => "SAFETY_INPUT_INVALID",
            Self::FieldTooLarge(_) => "SAFETY_INPUT_TOO_LARGE",
            Self::ContentConsentRequired => "AI_REPORT_CONTENT_CONSENT_REQUIRED",
            Self::ContentConsentMismatch => "AI_REPORT_CONTENT_CONSENT_MISMATCH",
            Self::TooManyDiagnosticCodes => "DIAGNOSTIC_CODE_LIMIT_EXCEEDED",
            Self::DiagnosticBundleTooLarge => "DIAGNOSTIC_BUNDLE_TOO_LARGE",
            Self::SerializationFailed => "SAFETY_SERIALIZATION_FAILED",
        }
    }

    #[must_use]
    pub const fn public_message(self) -> &'static str {
        match self {
            Self::InvalidField(_) => "safety input is invalid",
            Self::FieldTooLarge(_) => "safety input exceeds the product limit",
            Self::ContentConsentRequired => {
                "selected AI output requires explicit user consent before inclusion"
            }
            Self::ContentConsentMismatch => "AI report content consent is inconsistent",
            Self::TooManyDiagnosticCodes => "too many diagnostic codes were supplied",
            Self::DiagnosticBundleTooLarge => "diagnostic bundle exceeds the product limit",
            Self::SerializationFailed => "safety artifact could not be serialized",
        }
    }
}

impl fmt::Display for SafetyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.public_message())
    }
}

impl Error for SafetyError {}
