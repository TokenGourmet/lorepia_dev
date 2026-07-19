use crate::{CredentialVaultError, CredentialVaultErrorCode, Result};
use serde::de::{self, Deserialize, Deserializer, Visitor};
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

pub const MAX_SECRET_BYTES: usize = 2_048;
const REDACTED_DEBUG: &str = "[REDACTED]";

pub struct SecretInput(Zeroizing<Vec<u8>>);

impl SecretInput {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl TryFrom<String> for SecretInput {
    type Error = CredentialVaultError;

    fn try_from(value: String) -> Result<Self> {
        let mut bytes = value.into_bytes();
        let validation = validate_secret(&bytes);
        if let Err(error) = validation {
            bytes.zeroize();
            return Err(error);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }
}

impl fmt::Debug for SecretInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED_DEBUG)
    }
}

impl<'de> Deserialize<'de> for SecretInput {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SecretVisitor;

        impl Visitor<'_> for SecretVisitor {
            type Value = SecretInput;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded credential secret")
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                SecretInput::try_from(value).map_err(|error| E::custom(error.to_string()))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_string(value.to_owned())
            }
        }

        deserializer.deserialize_string(SecretVisitor)
    }
}

pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    pub(crate) fn try_from_store(mut bytes: Vec<u8>) -> Result<Self> {
        if let Err(error) = validate_secret(&bytes) {
            bytes.zeroize();
            return Err(error);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED_DEBUG)
    }
}

fn validate_secret(secret: &[u8]) -> Result<()> {
    if secret.is_empty() {
        return Err(CredentialVaultError::new(
            CredentialVaultErrorCode::InvalidSecret,
        ));
    }
    if secret.len() > MAX_SECRET_BYTES {
        return Err(CredentialVaultError::new(
            CredentialVaultErrorCode::SecretTooLarge,
        ));
    }
    if secret.iter().any(|byte| matches!(byte, 0 | b'\r' | b'\n')) {
        return Err(CredentialVaultError::new(
            CredentialVaultErrorCode::InvalidSecret,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_portable_limit() {
        let input = SecretInput::try_from("x".repeat(MAX_SECRET_BYTES)).unwrap();
        assert_eq!(input.as_bytes().len(), MAX_SECRET_BYTES);
    }

    #[test]
    fn rejects_empty_oversize_nul_and_newlines() {
        let cases = [
            String::new(),
            "x".repeat(MAX_SECRET_BYTES + 1),
            "api\0key".to_string(),
            "api\nkey".to_string(),
            "api\rkey".to_string(),
        ];
        for value in cases {
            assert!(SecretInput::try_from(value).is_err());
        }
    }

    #[test]
    fn debug_never_discloses_secret() {
        let input = SecretInput::try_from("diagnostic-sentinel".to_string()).unwrap();
        let loaded = SecretBytes::try_from_store(b"diagnostic-sentinel".to_vec()).unwrap();
        assert_eq!(format!("{input:?}"), REDACTED_DEBUG);
        assert_eq!(format!("{loaded:?}"), REDACTED_DEBUG);
    }

    #[test]
    fn serde_deserializes_secret_input_without_echoing_it() {
        let input: SecretInput = serde_json::from_str(r#""provider-secret""#).unwrap();
        assert_eq!(input.as_bytes(), b"provider-secret");
    }
}
