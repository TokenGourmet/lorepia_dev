use crate::SecretBytes;
use keyring_core::{CredentialStore, Entry, Error as KeyringError};
#[cfg(any(target_os = "android", target_os = "ios", target_os = "windows"))]
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zeroize::Zeroize;

pub(crate) const SERVICE_NAME: &str = "dev.lorepia.client.provider-credentials.v1";
#[cfg(any(target_os = "android", test))]
pub(crate) const ANDROID_STORE_NAME: &str = "lorepia-provider-credentials-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreFailure {
    Unavailable,
    Locked,
    Failure,
}

pub(crate) trait CredentialStoreBackend: Send + Sync {
    fn get(&self, account: &str) -> std::result::Result<Option<SecretBytes>, StoreFailure>;
    fn set(&self, account: &str, secret: &[u8]) -> std::result::Result<(), StoreFailure>;
    fn delete(&self, account: &str) -> std::result::Result<bool, StoreFailure>;
}

struct KeyringBackend {
    store: Arc<CredentialStore>,
}

impl KeyringBackend {
    fn entry(&self, account: &str) -> std::result::Result<Entry, StoreFailure> {
        #[cfg(target_os = "windows")]
        {
            let modifiers = HashMap::from([("persistence", "Local")]);
            return self
                .store
                .build(SERVICE_NAME, account, Some(&modifiers))
                .map_err(classify_keyring_error);
        }
        #[cfg(not(target_os = "windows"))]
        self.store
            .build(SERVICE_NAME, account, None)
            .map_err(classify_keyring_error)
    }
}

impl CredentialStoreBackend for KeyringBackend {
    fn get(&self, account: &str) -> std::result::Result<Option<SecretBytes>, StoreFailure> {
        match self.entry(account)?.get_secret() {
            Ok(secret) => SecretBytes::try_from_store(secret)
                .map(Some)
                .map_err(|_| StoreFailure::Failure),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(classify_keyring_error(error)),
        }
    }

    fn set(&self, account: &str, secret: &[u8]) -> std::result::Result<(), StoreFailure> {
        self.entry(account)?
            .set_secret(secret)
            .map_err(classify_keyring_error)
    }

    fn delete(&self, account: &str) -> std::result::Result<bool, StoreFailure> {
        match self.entry(account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(KeyringError::NoEntry) => Ok(false),
            Err(error) => Err(classify_keyring_error(error)),
        }
    }
}

struct RetryCache<T> {
    value: Mutex<Option<Arc<T>>>,
}

impl<T> RetryCache<T> {
    const fn new() -> Self {
        Self {
            value: Mutex::new(None),
        }
    }

    fn get_or_try_init(
        &self,
        initialize: impl FnOnce() -> std::result::Result<T, StoreFailure>,
    ) -> std::result::Result<Arc<T>, StoreFailure> {
        let mut value = self.value.lock().map_err(|_| StoreFailure::Failure)?;
        if let Some(existing) = value.as_ref() {
            return Ok(Arc::clone(existing));
        }
        let initialized = Arc::new(initialize()?);
        *value = Some(Arc::clone(&initialized));
        Ok(initialized)
    }
}

static PLATFORM_BACKEND: RetryCache<KeyringBackend> = RetryCache::new();

pub(crate) fn platform_backend()
-> std::result::Result<Arc<dyn CredentialStoreBackend>, StoreFailure> {
    PLATFORM_BACKEND
        .get_or_try_init(|| create_platform_store().map(|store| KeyringBackend { store }))
        .map(|backend| backend as Arc<dyn CredentialStoreBackend>)
}

fn classify_keyring_error(mut error: KeyringError) -> StoreFailure {
    let failure = match &error {
        KeyringError::NoStorageAccess(_) => StoreFailure::Locked,
        KeyringError::NoDefaultStore | KeyringError::NotSupportedByStore(_) => {
            StoreFailure::Unavailable
        }
        _ => StoreFailure::Failure,
    };
    redact_keyring_error(&mut error);
    failure
}

fn redact_keyring_error(error: &mut KeyringError) {
    match error {
        KeyringError::BadEncoding(bytes) | KeyringError::BadDataFormat(bytes, _) => {
            bytes.zeroize();
        }
        _ => {}
    }
}

#[cfg(target_os = "macos")]
fn create_platform_store() -> std::result::Result<Arc<CredentialStore>, StoreFailure> {
    let store: Arc<CredentialStore> =
        apple_native_keyring_store::keychain::Store::new().map_err(classify_keyring_error)?;
    Ok(store)
}

#[cfg(target_os = "ios")]
fn create_platform_store() -> std::result::Result<Arc<CredentialStore>, StoreFailure> {
    let configuration = HashMap::from([("cloud-sync", "false")]);
    let store: Arc<CredentialStore> =
        apple_native_keyring_store::protected::Store::new_with_configuration(&configuration)
            .map_err(classify_keyring_error)?;
    Ok(store)
}

#[cfg(target_os = "windows")]
fn create_platform_store() -> std::result::Result<Arc<CredentialStore>, StoreFailure> {
    let store: Arc<CredentialStore> =
        windows_native_keyring_store::Store::new().map_err(classify_keyring_error)?;
    Ok(store)
}

#[cfg(target_os = "linux")]
fn create_platform_store() -> std::result::Result<Arc<CredentialStore>, StoreFailure> {
    let store: Arc<CredentialStore> =
        zbus_secret_service_keyring_store::Store::new().map_err(|mut error| {
            redact_keyring_error(&mut error);
            StoreFailure::Unavailable
        })?;
    Ok(store)
}

#[cfg(target_os = "android")]
fn create_platform_store() -> std::result::Result<Arc<CredentialStore>, StoreFailure> {
    let configuration = HashMap::from([
        ("name", ANDROID_STORE_NAME),
        ("filename", ANDROID_STORE_NAME),
    ]);
    let store: Arc<CredentialStore> =
        android_native_keyring_store::Store::new_with_configuration(&configuration)
            .map_err(classify_keyring_error)?;
    Ok(store)
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
compile_error!("LorePia credential vault supports only macOS, iOS, Windows, Linux, and Android");

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn cache_retries_after_initialization_failure() {
        let cache = RetryCache::new();
        let attempts = AtomicUsize::new(0);
        let initialize = || {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(StoreFailure::Unavailable)
            } else {
                Ok(42_u8)
            }
        };
        assert_eq!(
            cache.get_or_try_init(initialize).unwrap_err(),
            StoreFailure::Unavailable
        );
        assert_eq!(*cache.get_or_try_init(initialize).unwrap(), 42);
        assert_eq!(*cache.get_or_try_init(initialize).unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn sensitive_keyring_error_payload_is_zeroized_before_drop() {
        let mut error = KeyringError::BadEncoding(vec![0x41; 32]);
        redact_keyring_error(&mut error);
        let KeyringError::BadEncoding(bytes) = error else {
            panic!("error variant changed unexpectedly");
        };
        assert!(bytes.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn product_store_identifiers_are_fixed() {
        assert_eq!(SERVICE_NAME, "dev.lorepia.client.provider-credentials.v1");
        assert_eq!(ANDROID_STORE_NAME, "lorepia-provider-credentials-v1");
    }
}
