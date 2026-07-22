use std::sync::Arc;

use lorepia_credential_vault::{
    CredentialStatus, CredentialVault, CredentialVaultError, CredentialVaultErrorCode, SecretInput,
};
use lorepia_providers::ProviderId;
use tauri::State;

pub(crate) struct CredentialVaultState(Arc<CredentialVault>);

impl Default for CredentialVaultState {
    fn default() -> Self {
        Self(Arc::new(CredentialVault::open_platform()))
    }
}

impl CredentialVaultState {
    pub(crate) fn vault(&self) -> Arc<CredentialVault> {
        Arc::clone(&self.0)
    }
}

pub(crate) async fn run_vault_operation<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, CredentialVaultError> + Send + 'static,
) -> Result<T, CredentialVaultError> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::InternalState))?
}

#[tauri::command]
pub(crate) async fn get_provider_credential_status(
    provider: ProviderId,
    vault: State<'_, CredentialVaultState>,
) -> Result<CredentialStatus, CredentialVaultError> {
    let vault = Arc::clone(&vault.0);
    run_vault_operation(move || vault.status(provider)).await
}

#[tauri::command]
pub(crate) async fn save_provider_api_key(
    provider: ProviderId,
    secret: SecretInput,
    vault: State<'_, CredentialVaultState>,
) -> Result<CredentialStatus, CredentialVaultError> {
    let vault = Arc::clone(&vault.0);
    run_vault_operation(move || vault.replace_api_key(provider, secret)).await
}

#[tauri::command]
pub(crate) async fn delete_provider_credential(
    provider: ProviderId,
    vault: State<'_, CredentialVaultState>,
) -> Result<CredentialStatus, CredentialVaultError> {
    let vault = Arc::clone(&vault.0);
    run_vault_operation(move || vault.delete(provider)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_state_is_lazy_and_does_not_open_the_os_store_at_startup() {
        let _state = CredentialVaultState::default();
    }
}
