use std::fmt;

use lorepia_providers::ProviderId;
use zeroize::Zeroizing;

use crate::{Result, RuntimeError, RuntimeErrorKind};

const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CredentialScope {
    OfficialProvider(ProviderId),
    OverrideHost(String),
}

/// Native-only credential material.
///
/// This type intentionally implements neither serde nor `Clone`. Product IPC
/// should pass a credential reference, resolve it in the native vault, and only
/// then construct this value.
pub struct ProviderCredential {
    scope: CredentialScope,
    secret: Zeroizing<String>,
}

impl ProviderCredential {
    pub fn for_official(provider: ProviderId, secret: impl Into<String>) -> Result<Self> {
        Self::new(CredentialScope::OfficialProvider(provider), secret.into())
    }

    pub fn for_override_host(host: impl Into<String>, secret: impl Into<String>) -> Result<Self> {
        let host = host.into().to_ascii_lowercase();
        if host.is_empty()
            || host.len() > 253
            || host.starts_with('.')
            || host.ends_with('.')
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
            })
            || host.chars().any(|character| {
                !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
            })
        {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidCredential,
                "INVALID_CREDENTIAL_SCOPE",
                "override credential scope must be an exact DNS host name",
            ));
        }
        Self::new(CredentialScope::OverrideHost(host), secret.into())
    }

    fn new(scope: CredentialScope, secret: String) -> Result<Self> {
        // Wrap before validation so every early-return path wipes the owned
        // credential buffer instead of dropping a plain `String`.
        let secret = Zeroizing::new(secret);
        if secret.is_empty() || secret.len() > MAX_CREDENTIAL_BYTES {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidCredential,
                "INVALID_CREDENTIAL",
                "credential must contain between 1 and 16384 bytes",
            ));
        }
        if secret.chars().any(|character| character.is_ascii_control()) {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidCredential,
                "INVALID_CREDENTIAL",
                "credential must not contain control characters",
            ));
        }
        Ok(Self { scope, secret })
    }

    pub(crate) fn scope(&self) -> &CredentialScope {
        &self.scope
    }

    pub(crate) fn secret(&self) -> &str {
        self.secret.as_str()
    }
}

impl fmt::Debug for ProviderCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCredential")
            .field("scope", &self.scope)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_exposes_secret() {
        let credential = ProviderCredential::for_official(ProviderId::OpenAi, "sk-secret")
            .expect("valid credential");
        let debug = format!("{credential:?}");
        assert!(!debug.contains("sk-secret"));
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn control_characters_are_rejected() {
        let error = ProviderCredential::for_official(ProviderId::OpenAi, "secret\nheader")
            .expect_err("header injection must fail");
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidCredential);
    }
}
