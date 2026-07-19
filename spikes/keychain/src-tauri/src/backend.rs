use keyring_core::{CredentialStore, Entry, Error as KeyringError};
#[cfg(any(target_os = "windows", target_os = "android"))]
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zeroize::{Zeroize, Zeroizing};

const PROBE_SERVICE: &str = "dev.lorepia.spike.keychain.m1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreFailure {
    Unavailable,
    Locked,
    Failure,
}

pub(crate) trait ProbeStore: Send + Sync {
    fn backend_label(&self) -> &'static str;
    fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreFailure>;
    fn set(&self, account: &str, secret: &[u8]) -> Result<(), StoreFailure>;
    fn delete(&self, account: &str) -> Result<bool, StoreFailure>;
}

pub(crate) struct KeyringStore {
    store: Arc<CredentialStore>,
}

impl KeyringStore {
    fn entry(&self, account: &str) -> Result<Entry, StoreFailure> {
        #[cfg(target_os = "windows")]
        {
            let modifiers = HashMap::from([("persistence", "Local")]);
            self.store
                .build(PROBE_SERVICE, account, Some(&modifiers))
                .map_err(classify_keyring_error)
        }
        #[cfg(not(target_os = "windows"))]
        self.store
            .build(PROBE_SERVICE, account, None)
            .map_err(classify_keyring_error)
    }
}

impl ProbeStore for KeyringStore {
    fn backend_label(&self) -> &'static str {
        platform_backend_label()
    }

    fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreFailure> {
        match self.entry(account)?.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(classify_keyring_error(error)),
        }
    }

    fn set(&self, account: &str, secret: &[u8]) -> Result<(), StoreFailure> {
        self.entry(account)?
            .set_secret(secret)
            .map_err(classify_keyring_error)
    }

    fn delete(&self, account: &str) -> Result<bool, StoreFailure> {
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
        initialize: impl FnOnce() -> Result<T, StoreFailure>,
    ) -> Result<Arc<T>, StoreFailure> {
        let mut value = self.value.lock().map_err(|_| StoreFailure::Failure)?;
        if let Some(existing) = value.as_ref() {
            return Ok(Arc::clone(existing));
        }
        let initialized = Arc::new(initialize()?);
        *value = Some(Arc::clone(&initialized));
        Ok(initialized)
    }
}

static PLATFORM_STORE: RetryCache<KeyringStore> = RetryCache::new();

pub(crate) fn platform_store() -> Result<Arc<KeyringStore>, StoreFailure> {
    PLATFORM_STORE.get_or_try_init(|| create_platform_store().map(|store| KeyringStore { store }))
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
fn create_platform_store() -> Result<Arc<CredentialStore>, StoreFailure> {
    let store: Arc<CredentialStore> =
        apple_native_keyring_store::keychain::Store::new().map_err(classify_keyring_error)?;
    Ok(store)
}

#[cfg(target_os = "ios")]
fn create_platform_store() -> Result<Arc<CredentialStore>, StoreFailure> {
    let store: Arc<CredentialStore> =
        apple_native_keyring_store::protected::Store::new().map_err(classify_keyring_error)?;
    Ok(store)
}

#[cfg(target_os = "windows")]
fn create_platform_store() -> Result<Arc<CredentialStore>, StoreFailure> {
    let store: Arc<CredentialStore> =
        windows_native_keyring_store::Store::new().map_err(classify_keyring_error)?;
    Ok(store)
}

#[cfg(target_os = "linux")]
fn create_platform_store() -> Result<Arc<CredentialStore>, StoreFailure> {
    let store: Arc<CredentialStore> =
        zbus_secret_service_keyring_store::Store::new().map_err(|mut error| {
            redact_keyring_error(&mut error);
            StoreFailure::Unavailable
        })?;
    Ok(store)
}

#[cfg(target_os = "android")]
fn create_platform_store() -> Result<Arc<CredentialStore>, StoreFailure> {
    let configuration = HashMap::from([
        ("name", "lorepia-keyring-v1"),
        ("filename", "lorepia-keyring-v1"),
    ]);
    let store: Arc<CredentialStore> =
        android_native_keyring_store::Store::new_with_configuration(&configuration)
            .map_err(classify_keyring_error)?;
    Ok(store)
}

#[cfg(target_os = "macos")]
const fn platform_backend_label() -> &'static str {
    "macos-keychain"
}

#[cfg(target_os = "ios")]
const fn platform_backend_label() -> &'static str {
    "ios-protected-data"
}

#[cfg(target_os = "windows")]
const fn platform_backend_label() -> &'static str {
    "windows-credential-manager"
}

#[cfg(target_os = "linux")]
const fn platform_backend_label() -> &'static str {
    "linux-secret-service"
}

#[cfg(target_os = "android")]
const fn platform_backend_label() -> &'static str {
    "android-keystore-encrypted-preferences"
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
compile_error!("the LorePia keychain probe supports only macOS, iOS, Windows, Linux, and Android");

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn cache_retries_after_initialization_failure() {
        let cache = RetryCache::new();
        let attempts = AtomicUsize::new(0);
        let initialize = || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                Err(StoreFailure::Unavailable)
            } else {
                Ok(42_u8)
            }
        };
        assert!(matches!(
            cache.get_or_try_init(initialize),
            Err(StoreFailure::Unavailable)
        ));
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
}
