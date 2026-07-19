use crate::backend::{CredentialStoreBackend, StoreFailure, platform_backend};
use crate::{CredentialVaultError, CredentialVaultErrorCode, Result, SecretBytes, SecretInput};
use lorepia_providers::ProviderId;
use serde::Serialize;
use std::sync::{Arc, Mutex};

static PLATFORM_OPERATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    pub provider: ProviderId,
    pub configured: bool,
}

enum BackendSource {
    Platform,
    #[cfg(test)]
    Fixed(Arc<dyn CredentialStoreBackend>),
}

pub struct CredentialVault {
    backend: BackendSource,
}

impl CredentialVault {
    #[must_use]
    pub const fn open_platform() -> Self {
        Self {
            backend: BackendSource::Platform,
        }
    }

    pub fn status(&self, provider: ProviderId) -> Result<CredentialStatus> {
        let account = api_key_account(provider)?;
        self.with_store(|store| {
            let configured = store.get(account)?.is_some();
            Ok(CredentialStatus {
                provider,
                configured,
            })
        })
    }

    pub fn replace_api_key(
        &self,
        provider: ProviderId,
        secret: SecretInput,
    ) -> Result<CredentialStatus> {
        let account = api_key_account(provider)?;
        self.with_store(|store| {
            store.set(account, secret.as_bytes())?;
            let stored = store.get(account)?.ok_or(StoreFailure::Failure)?;
            if stored.as_bytes() != secret.as_bytes() {
                return Err(StoreFailure::Failure.into());
            }
            Ok(CredentialStatus {
                provider,
                configured: true,
            })
        })
    }

    pub fn load_api_key_for_native_use(&self, provider: ProviderId) -> Result<SecretBytes> {
        let account = api_key_account(provider)?;
        self.with_store(|store| {
            store
                .get(account)?
                .ok_or_else(|| CredentialVaultError::new(CredentialVaultErrorCode::NotConfigured))
        })
    }

    pub fn delete(&self, provider: ProviderId) -> Result<CredentialStatus> {
        let account = api_key_account(provider)?;
        self.with_store(|store| {
            let _was_present = store.delete(account)?;
            if store.get(account)?.is_some() {
                return Err(StoreFailure::Failure.into());
            }
            Ok(CredentialStatus {
                provider,
                configured: false,
            })
        })
    }

    fn with_store<T>(
        &self,
        operation: impl FnOnce(&dyn CredentialStoreBackend) -> Result<T>,
    ) -> Result<T> {
        let _guard = PLATFORM_OPERATION_LOCK
            .lock()
            .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::InternalState))?;
        let store = self.resolve_backend().map_err(CredentialVaultError::from)?;
        operation(store.as_ref())
    }

    fn resolve_backend(
        &self,
    ) -> std::result::Result<Arc<dyn CredentialStoreBackend>, StoreFailure> {
        match &self.backend {
            BackendSource::Platform => platform_backend(),
            #[cfg(test)]
            BackendSource::Fixed(store) => Ok(Arc::clone(store)),
        }
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn CredentialStoreBackend>) -> Self {
        Self {
            backend: BackendSource::Fixed(backend),
        }
    }
}

impl Default for CredentialVault {
    fn default() -> Self {
        Self::open_platform()
    }
}

impl From<StoreFailure> for CredentialVaultError {
    fn from(failure: StoreFailure) -> Self {
        let code = match failure {
            StoreFailure::Unavailable => CredentialVaultErrorCode::StoreUnavailable,
            StoreFailure::Locked => CredentialVaultErrorCode::StoreLocked,
            StoreFailure::Failure => CredentialVaultErrorCode::StoreFailure,
        };
        Self::new(code)
    }
}

fn api_key_account(provider: ProviderId) -> Result<&'static str> {
    match provider {
        ProviderId::OpenAi => Ok("openai-api-key-v1"),
        ProviderId::Anthropic => Ok("anthropic-api-key-v1"),
        ProviderId::DeepSeek => Ok("deepseek-api-key-v1"),
        ProviderId::OllamaCloud => Ok("ollama-cloud-api-key-v1"),
        ProviderId::GoogleGemini => Ok("google-gemini-api-key-v1"),
        ProviderId::GoogleVertexAi => Err(CredentialVaultError::new(
            CredentialVaultErrorCode::UnsupportedProvider,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeBackend {
        entries: Mutex<HashMap<String, Vec<u8>>>,
        calls: Mutex<Vec<String>>,
        failure: Mutex<Option<StoreFailure>>,
        retain_after_delete: bool,
    }

    impl FakeBackend {
        fn accounts(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn fail_with(&self, failure: StoreFailure) {
            *self.failure.lock().unwrap() = Some(failure);
        }

        fn maybe_fail(&self) -> std::result::Result<(), StoreFailure> {
            self.failure.lock().unwrap().take().map_or(Ok(()), Err)
        }
    }

    impl CredentialStoreBackend for FakeBackend {
        fn get(&self, account: &str) -> std::result::Result<Option<SecretBytes>, StoreFailure> {
            self.maybe_fail()?;
            self.calls.lock().unwrap().push(account.to_string());
            self.entries
                .lock()
                .unwrap()
                .get(account)
                .cloned()
                .map(SecretBytes::try_from_store)
                .transpose()
                .map_err(|_| StoreFailure::Failure)
        }

        fn set(&self, account: &str, secret: &[u8]) -> std::result::Result<(), StoreFailure> {
            self.maybe_fail()?;
            self.calls.lock().unwrap().push(account.to_string());
            self.entries
                .lock()
                .unwrap()
                .insert(account.to_string(), secret.to_vec());
            Ok(())
        }

        fn delete(&self, account: &str) -> std::result::Result<bool, StoreFailure> {
            self.maybe_fail()?;
            self.calls.lock().unwrap().push(account.to_string());
            if self.retain_after_delete {
                return Ok(self.entries.lock().unwrap().contains_key(account));
            }
            Ok(self.entries.lock().unwrap().remove(account).is_some())
        }
    }

    fn vault_with_fake() -> (CredentialVault, Arc<FakeBackend>) {
        let backend = Arc::new(FakeBackend::default());
        let vault = CredentialVault::with_backend(backend.clone());
        (vault, backend)
    }

    #[test]
    fn fixed_api_key_accounts_round_trip_for_five_providers() {
        let expected = [
            (ProviderId::OpenAi, "openai-api-key-v1"),
            (ProviderId::Anthropic, "anthropic-api-key-v1"),
            (ProviderId::DeepSeek, "deepseek-api-key-v1"),
            (ProviderId::OllamaCloud, "ollama-cloud-api-key-v1"),
            (ProviderId::GoogleGemini, "google-gemini-api-key-v1"),
        ];
        for (provider, account) in expected {
            let (vault, backend) = vault_with_fake();
            let secret = format!("secret-for-{}", provider.as_str());
            assert_eq!(
                vault
                    .replace_api_key(provider, SecretInput::try_from(secret.clone()).unwrap())
                    .unwrap(),
                CredentialStatus {
                    provider,
                    configured: true,
                }
            );
            assert_eq!(
                vault
                    .load_api_key_for_native_use(provider)
                    .unwrap()
                    .as_bytes(),
                secret.as_bytes()
            );
            assert!(backend.accounts().iter().all(|actual| actual == account));
        }
    }

    #[test]
    fn update_status_and_idempotent_delete_are_exact() {
        let (vault, _) = vault_with_fake();
        assert_eq!(
            vault.status(ProviderId::OpenAi).unwrap(),
            CredentialStatus {
                provider: ProviderId::OpenAi,
                configured: false,
            }
        );
        vault
            .replace_api_key(
                ProviderId::OpenAi,
                SecretInput::try_from("initial".to_string()).unwrap(),
            )
            .unwrap();
        vault
            .replace_api_key(
                ProviderId::OpenAi,
                SecretInput::try_from("updated".to_string()).unwrap(),
            )
            .unwrap();
        assert_eq!(
            vault
                .load_api_key_for_native_use(ProviderId::OpenAi)
                .unwrap()
                .as_bytes(),
            b"updated"
        );
        assert!(!vault.delete(ProviderId::OpenAi).unwrap().configured);
        assert!(!vault.delete(ProviderId::OpenAi).unwrap().configured);
    }

    #[test]
    fn vertex_api_keys_are_rejected_before_store_access() {
        let (vault, backend) = vault_with_fake();
        let error = vault
            .replace_api_key(
                ProviderId::GoogleVertexAi,
                SecretInput::try_from("must-not-store".to_string()).unwrap(),
            )
            .unwrap_err();
        assert_eq!(error.code, CredentialVaultErrorCode::UnsupportedProvider);
        assert!(backend.accounts().is_empty());
    }

    #[test]
    fn missing_credential_has_stable_error() {
        let (vault, _) = vault_with_fake();
        let error = vault
            .load_api_key_for_native_use(ProviderId::Anthropic)
            .unwrap_err();
        assert_eq!(error.code, CredentialVaultErrorCode::NotConfigured);
    }

    #[test]
    fn backend_failures_are_redacted_to_stable_codes() {
        let (vault, backend) = vault_with_fake();
        backend.fail_with(StoreFailure::Locked);
        let error = vault.status(ProviderId::DeepSeek).unwrap_err();
        assert_eq!(error.code, CredentialVaultErrorCode::StoreLocked);
        assert!(!format!("{error:?}").contains("secret"));
    }

    #[test]
    fn corrupt_oversize_stored_value_fails_closed() {
        let (vault, backend) = vault_with_fake();
        backend.entries.lock().unwrap().insert(
            "deepseek-api-key-v1".to_string(),
            vec![b'x'; crate::MAX_SECRET_BYTES + 1],
        );
        let error = vault
            .load_api_key_for_native_use(ProviderId::DeepSeek)
            .unwrap_err();
        assert_eq!(error.code, CredentialVaultErrorCode::StoreFailure);
    }

    #[test]
    fn save_and_delete_verify_backend_effects() {
        let backend = Arc::new(FakeBackend {
            retain_after_delete: true,
            ..FakeBackend::default()
        });
        let vault = CredentialVault::with_backend(backend);
        vault
            .replace_api_key(
                ProviderId::GoogleGemini,
                SecretInput::try_from("gemini-key".to_string()).unwrap(),
            )
            .unwrap();
        let error = vault.delete(ProviderId::GoogleGemini).unwrap_err();
        assert_eq!(error.code, CredentialVaultErrorCode::StoreFailure);
    }
}
