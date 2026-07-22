use serde::Serialize;
use std::fmt;

pub type Result<T> = std::result::Result<T, CredentialVaultError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialVaultErrorCode {
    UnsupportedProvider,
    InvalidSecret,
    SecretTooLarge,
    NotConfigured,
    StoreUnavailable,
    StoreLocked,
    StoreFailure,
    InternalState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CredentialVaultError {
    pub code: CredentialVaultErrorCode,
}

impl CredentialVaultError {
    #[must_use]
    pub const fn new(code: CredentialVaultErrorCode) -> Self {
        Self { code }
    }
}

impl fmt::Display for CredentialVaultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.code {
            CredentialVaultErrorCode::UnsupportedProvider => "unsupported provider credential",
            CredentialVaultErrorCode::InvalidSecret => "invalid credential secret",
            CredentialVaultErrorCode::SecretTooLarge => "credential secret exceeds size limit",
            CredentialVaultErrorCode::NotConfigured => "provider credential is not configured",
            CredentialVaultErrorCode::StoreUnavailable => "credential store is unavailable",
            CredentialVaultErrorCode::StoreLocked => "credential store is locked",
            CredentialVaultErrorCode::StoreFailure => "credential store operation failed",
            CredentialVaultErrorCode::InternalState => "credential vault internal state failed",
        })
    }
}

impl std::error::Error for CredentialVaultError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_error_shape_is_bounded_and_stable() {
        let error = CredentialVaultError::new(CredentialVaultErrorCode::StoreFailure);
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"code":"STORE_FAILURE"}"#
        );
        assert_eq!(error.to_string(), "credential store operation failed");
    }
}
