use crate::backend::{ProbeStore, StoreFailure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{Mutex, TryLockError};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

const REGISTRY_ACCOUNT: &str = "m1-cleanup-registry-v1";
const TARGET_ACCOUNT_PREFIX: &str = "m1-probe-";
const REGISTRY_MAGIC: &[u8; 8] = b"LPM1REG1";
const REFERENCE_BYTES: usize = 16;
const SECRET_BYTES: usize = 32;
const HASH_BYTES: usize = 32;
const REGISTRY_BYTES: usize = REGISTRY_MAGIC.len() + REFERENCE_BYTES + (2 * HASH_BYTES);

static PROBE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ProbeErrorCode {
    ProbeBusy,
    StoreUnavailable,
    StoreLocked,
    StoreFailure,
    CleanupFailed,
    Collision,
    RandomFailure,
    InternalState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeError {
    code: ProbeErrorCode,
    cleanup_pending: bool,
}

impl ProbeError {
    fn new(code: ProbeErrorCode, cleanup_pending: bool) -> Self {
        Self {
            code,
            cleanup_pending,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LifecycleEvidence {
    absent_before_create: bool,
    created: bool,
    initial_read_matched: bool,
    updated: bool,
    updated_read_matched: bool,
    deleted: bool,
    absent_after_delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeReceipt {
    run_id: String,
    backend: &'static str,
    reference_fingerprint: String,
    lifecycle: LifecycleEvidence,
    stale_cleanup_recovered: bool,
    cleanup_pending: bool,
}

pub(crate) trait RandomSource: Send + Sync {
    fn fill(&self, destination: &mut [u8]) -> Result<(), ()>;
}

pub(crate) struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&self, destination: &mut [u8]) -> Result<(), ()> {
        getrandom::fill(destination).map_err(|_| ())
    }
}

struct RegistryRecord {
    reference: [u8; REFERENCE_BYTES],
    initial_hash: [u8; HASH_BYTES],
    updated_hash: [u8; HASH_BYTES],
}

impl RegistryRecord {
    fn new(
        reference: [u8; REFERENCE_BYTES],
        initial_hash: [u8; HASH_BYTES],
        updated_hash: [u8; HASH_BYTES],
    ) -> Self {
        Self {
            reference,
            initial_hash,
            updated_hash,
        }
    }

    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut encoded = Zeroizing::new(Vec::with_capacity(REGISTRY_BYTES));
        encoded.extend_from_slice(REGISTRY_MAGIC);
        encoded.extend_from_slice(&self.reference);
        encoded.extend_from_slice(&self.initial_hash);
        encoded.extend_from_slice(&self.updated_hash);
        encoded
    }

    fn parse(encoded: &[u8]) -> Option<Self> {
        if encoded.len() != REGISTRY_BYTES
            || !constant_time_equal(&encoded[..REGISTRY_MAGIC.len()], REGISTRY_MAGIC)
        {
            return None;
        }

        let mut reference = [0_u8; REFERENCE_BYTES];
        let mut initial_hash = [0_u8; HASH_BYTES];
        let mut updated_hash = [0_u8; HASH_BYTES];
        let reference_start = REGISTRY_MAGIC.len();
        let initial_start = reference_start + REFERENCE_BYTES;
        let updated_start = initial_start + HASH_BYTES;
        reference.copy_from_slice(&encoded[reference_start..initial_start]);
        initial_hash.copy_from_slice(&encoded[initial_start..updated_start]);
        updated_hash.copy_from_slice(&encoded[updated_start..]);
        Some(Self::new(reference, initial_hash, updated_hash))
    }

    fn target_account(&self) -> String {
        format!("{TARGET_ACCOUNT_PREFIX}{}", lowercase_hex(&self.reference))
    }

    fn owns_secret(&self, secret: &[u8]) -> bool {
        let actual_hash = sha256(secret);
        constant_time_equal(&actual_hash, &self.initial_hash)
            || constant_time_equal(&actual_hash, &self.updated_hash)
    }
}

impl Drop for RegistryRecord {
    fn drop(&mut self) {
        self.reference.zeroize();
        self.initial_hash.zeroize();
        self.updated_hash.zeroize();
    }
}

struct ActiveRegistry<'a> {
    store: &'a dyn ProbeStore,
    record: RegistryRecord,
    encoded: Zeroizing<Vec<u8>>,
    target_account: String,
}

impl<'a> ActiveRegistry<'a> {
    fn new(store: &'a dyn ProbeStore, record: RegistryRecord) -> Self {
        let target_account = record.target_account();
        let encoded = record.encode();
        Self {
            store,
            record,
            encoded,
            target_account,
        }
    }

    fn delete_registry_if_owned(&self) -> Result<(), ()> {
        let Some(current) = self.store.get(REGISTRY_ACCOUNT).map_err(|_| ())? else {
            return Ok(());
        };
        if !constant_time_equal(current.as_slice(), self.encoded.as_slice()) {
            return Err(());
        }
        if !self.store.delete(REGISTRY_ACCOUNT).map_err(|_| ())? {
            return Err(());
        }
        if self.store.get(REGISTRY_ACCOUNT).map_err(|_| ())?.is_some() {
            return Err(());
        }
        Ok(())
    }

    fn verify_registry_ownership(&self) -> Result<(), ProbeError> {
        match self.store.get(REGISTRY_ACCOUNT) {
            Ok(Some(current))
                if constant_time_equal(current.as_slice(), self.encoded.as_slice()) =>
            {
                Ok(())
            }
            Ok(_) => Err(ProbeError::new(ProbeErrorCode::InternalState, true)),
            Err(failure) => Err(store_error(failure, true)),
        }
    }

    fn cleanup_owned_target_and_registry(&self) -> Result<(), ()> {
        let current_registry = self.store.get(REGISTRY_ACCOUNT).map_err(|_| ())?;
        match current_registry {
            Some(current) if constant_time_equal(current.as_slice(), self.encoded.as_slice()) => {}
            Some(_) => return Err(()),
            None => {
                return if self
                    .store
                    .get(&self.target_account)
                    .map_err(|_| ())?
                    .is_none()
                {
                    Ok(())
                } else {
                    Err(())
                };
            }
        }

        if let Some(secret) = self.store.get(&self.target_account).map_err(|_| ())? {
            if !self.record.owns_secret(secret.as_slice()) {
                return Err(());
            }
            if !self.store.delete(&self.target_account).map_err(|_| ())? {
                return Err(());
            }
            if self
                .store
                .get(&self.target_account)
                .map_err(|_| ())?
                .is_some()
            {
                return Err(());
            }
        }
        self.delete_registry_if_owned()
    }
}

#[cfg(test)]
pub(crate) fn run_with_process_lock(
    store: &dyn ProbeStore,
    random: &dyn RandomSource,
) -> Result<ProbeReceipt, ProbeError> {
    with_process_lock(|| execute_probe(store, random))
}

pub(crate) fn with_process_lock<T>(
    operation: impl FnOnce() -> Result<T, ProbeError>,
) -> Result<T, ProbeError> {
    let _guard = match PROBE_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => {
            return Err(ProbeError::new(ProbeErrorCode::ProbeBusy, false));
        }
        Err(TryLockError::Poisoned(_)) => {
            return Err(ProbeError::new(ProbeErrorCode::InternalState, true));
        }
    };
    operation()
}

pub(crate) fn execute_probe(
    store: &dyn ProbeStore,
    random: &dyn RandomSource,
) -> Result<ProbeReceipt, ProbeError> {
    let stale_cleanup_recovered = recover_stale_registry(store)?;

    let mut run_identifier = [0_u8; REFERENCE_BYTES];
    let mut reference = Zeroizing::new([0_u8; REFERENCE_BYTES]);
    let mut initial_random = Zeroizing::new([0_u8; SECRET_BYTES]);
    let mut updated_random = Zeroizing::new([0_u8; SECRET_BYTES]);
    random
        .fill(&mut run_identifier)
        .map_err(|_| ProbeError::new(ProbeErrorCode::RandomFailure, false))?;
    random
        .fill(reference.as_mut())
        .map_err(|_| ProbeError::new(ProbeErrorCode::RandomFailure, false))?;
    random
        .fill(initial_random.as_mut())
        .map_err(|_| ProbeError::new(ProbeErrorCode::RandomFailure, false))?;
    random
        .fill(updated_random.as_mut())
        .map_err(|_| ProbeError::new(ProbeErrorCode::RandomFailure, false))?;
    if constant_time_equal(&run_identifier, reference.as_ref())
        || constant_time_equal(initial_random.as_ref(), updated_random.as_ref())
    {
        return Err(ProbeError::new(ProbeErrorCode::RandomFailure, false));
    }
    // Keep Secret Service/KWallet interop predictable: stored probe values are
    // non-empty, high-entropy UTF-8 rather than arbitrary binary blobs.
    let initial_secret = lowercase_hex_bytes(initial_random.as_ref());
    let updated_secret = lowercase_hex_bytes(updated_random.as_ref());

    let record = RegistryRecord::new(
        *reference,
        sha256(initial_secret.as_ref()),
        sha256(updated_secret.as_ref()),
    );
    let active = ActiveRegistry::new(store, record);
    let mut lifecycle = LifecycleEvidence::default();

    match store.get(&active.target_account) {
        Ok(None) => lifecycle.absent_before_create = true,
        Ok(Some(_)) => return Err(ProbeError::new(ProbeErrorCode::Collision, false)),
        Err(failure) => return Err(store_error(failure, false)),
    }

    if let Err(failure) = store.set(REGISTRY_ACCOUNT, active.encoded.as_slice()) {
        return finish_after_failure(&active, store_error(failure, true));
    }

    match store.get(&active.target_account) {
        Ok(None) => {}
        Ok(Some(_)) => {
            return match active.delete_registry_if_owned() {
                Ok(()) => Err(ProbeError::new(ProbeErrorCode::Collision, false)),
                Err(()) => Err(ProbeError::new(ProbeErrorCode::CleanupFailed, true)),
            };
        }
        Err(failure) => {
            return finish_after_failure(&active, store_error(failure, true));
        }
    }

    let lifecycle_result = (|| {
        store
            .set(&active.target_account, initial_secret.as_ref())
            .map_err(|failure| store_error(failure, true))?;
        lifecycle.created = true;

        lifecycle.initial_read_matched =
            read_matches(store, &active.target_account, initial_secret.as_ref())?;
        if !lifecycle.initial_read_matched {
            return Err(ProbeError::new(ProbeErrorCode::StoreFailure, true));
        }

        store
            .set(&active.target_account, updated_secret.as_ref())
            .map_err(|failure| store_error(failure, true))?;
        lifecycle.updated = true;

        lifecycle.updated_read_matched =
            read_matches(store, &active.target_account, updated_secret.as_ref())?;
        if !lifecycle.updated_read_matched {
            return Err(ProbeError::new(ProbeErrorCode::StoreFailure, true));
        }

        // Deleting requires both the just-verified target value and the exact
        // cleanup registry record. Recheck the registry immediately before
        // the destructive operation, then cleanup checks it again.
        active.verify_registry_ownership()?;
        lifecycle.deleted = store
            .delete(&active.target_account)
            .map_err(|failure| store_error(failure, true))?;
        if !lifecycle.deleted {
            return Err(ProbeError::new(ProbeErrorCode::StoreFailure, true));
        }

        lifecycle.absent_after_delete = match store.get(&active.target_account) {
            Ok(None) => true,
            Ok(Some(_)) => false,
            Err(failure) => return Err(store_error(failure, true)),
        };
        if !lifecycle.absent_after_delete {
            return Err(ProbeError::new(ProbeErrorCode::StoreFailure, true));
        }
        Ok(())
    })();

    if let Err(error) = lifecycle_result {
        return finish_after_failure(&active, error);
    }
    if active.delete_registry_if_owned().is_err() {
        return Err(ProbeError::new(ProbeErrorCode::CleanupFailed, true));
    }

    let run_id = lowercase_hex(&run_identifier);
    let reference_hash = sha256(reference.as_ref());
    Ok(ProbeReceipt {
        run_id,
        backend: store.backend_label(),
        reference_fingerprint: lowercase_hex(&reference_hash[..8]),
        lifecycle,
        stale_cleanup_recovered,
        cleanup_pending: false,
    })
}

fn read_matches(
    store: &dyn ProbeStore,
    account: &str,
    expected: &[u8],
) -> Result<bool, ProbeError> {
    match store.get(account) {
        Ok(Some(actual)) => Ok(constant_time_equal(actual.as_slice(), expected)),
        Ok(None) => Ok(false),
        Err(failure) => Err(store_error(failure, true)),
    }
}

fn finish_after_failure(
    active: &ActiveRegistry<'_>,
    mut original: ProbeError,
) -> Result<ProbeReceipt, ProbeError> {
    match active.cleanup_owned_target_and_registry() {
        Ok(()) => {
            original.cleanup_pending = false;
            Err(original)
        }
        Err(()) => Err(ProbeError::new(ProbeErrorCode::CleanupFailed, true)),
    }
}

fn recover_stale_registry(store: &dyn ProbeStore) -> Result<bool, ProbeError> {
    let encoded = match store.get(REGISTRY_ACCOUNT) {
        Ok(Some(encoded)) => encoded,
        Ok(None) => return Ok(false),
        Err(failure) => return Err(store_error(failure, true)),
    };
    let record = RegistryRecord::parse(encoded.as_slice())
        .ok_or_else(|| ProbeError::new(ProbeErrorCode::InternalState, true))?;
    let active = ActiveRegistry {
        store,
        target_account: record.target_account(),
        record,
        encoded,
    };

    match store.get(&active.target_account) {
        Ok(Some(secret)) if !active.record.owns_secret(secret.as_slice()) => {
            return Err(ProbeError::new(ProbeErrorCode::Collision, true));
        }
        Ok(_) => {}
        Err(failure) => return Err(store_error(failure, true)),
    }
    active
        .cleanup_owned_target_and_registry()
        .map_err(|_| ProbeError::new(ProbeErrorCode::CleanupFailed, true))?;
    Ok(true)
}

fn store_error(failure: StoreFailure, cleanup_pending: bool) -> ProbeError {
    let code = match failure {
        StoreFailure::Unavailable => ProbeErrorCode::StoreUnavailable,
        StoreFailure::Locked => ProbeErrorCode::StoreLocked,
        StoreFailure::Failure => ProbeErrorCode::StoreFailure,
    };
    ProbeError::new(code, cleanup_pending)
}

pub(crate) fn probe_error_from_store_failure(failure: StoreFailure) -> ProbeError {
    // Backend initialization happens before stale-registry inspection, so a
    // prior interrupted run cannot be ruled out when initialization fails.
    store_error(failure, true)
}

pub(crate) fn internal_state_error() -> ProbeError {
    ProbeError::new(ProbeErrorCode::InternalState, true)
}

fn sha256(value: &[u8]) -> [u8; HASH_BYTES] {
    Sha256::digest(value).into()
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && bool::from(left.ct_eq(right))
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn lowercase_hex_bytes(bytes: &[u8]) -> Zeroizing<Vec<u8>> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = Zeroizing::new(Vec::with_capacity(bytes.len() * 2));
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)]);
        encoded.push(HEX[usize::from(byte & 0x0f)]);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Get(String),
        Set(String),
        Delete(String),
    }

    #[derive(Default)]
    struct FakeState {
        entries: HashMap<String, Zeroizing<Vec<u8>>>,
        calls: Vec<Call>,
        operation: usize,
        fail_at: Option<usize>,
    }

    #[derive(Default)]
    struct FakeStore {
        state: Mutex<FakeState>,
    }

    impl FakeStore {
        fn fail_at(operation: usize) -> Self {
            Self {
                state: Mutex::new(FakeState {
                    fail_at: Some(operation),
                    ..FakeState::default()
                }),
            }
        }

        fn seed(&self, account: &str, secret: &[u8]) {
            self.state
                .lock()
                .unwrap()
                .entries
                .insert(account.to_owned(), Zeroizing::new(secret.to_vec()));
        }

        fn contains(&self, account: &str) -> bool {
            self.state.lock().unwrap().entries.contains_key(account)
        }

        fn calls(&self) -> Vec<Call> {
            self.state.lock().unwrap().calls.clone()
        }

        fn begin(state: &mut FakeState, call: Call) -> Result<(), StoreFailure> {
            state.operation += 1;
            state.calls.push(call);
            if state.fail_at == Some(state.operation) {
                return Err(StoreFailure::Failure);
            }
            Ok(())
        }
    }

    impl ProbeStore for FakeStore {
        fn backend_label(&self) -> &'static str {
            "macos-keychain"
        }

        fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreFailure> {
            let mut state = self.state.lock().unwrap();
            Self::begin(&mut state, Call::Get(account.to_owned()))?;
            Ok(state.entries.get(account).cloned())
        }

        fn set(&self, account: &str, secret: &[u8]) -> Result<(), StoreFailure> {
            let mut state = self.state.lock().unwrap();
            Self::begin(&mut state, Call::Set(account.to_owned()))?;
            state
                .entries
                .insert(account.to_owned(), Zeroizing::new(secret.to_vec()));
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<bool, StoreFailure> {
            let mut state = self.state.lock().unwrap();
            Self::begin(&mut state, Call::Delete(account.to_owned()))?;
            Ok(state.entries.remove(account).is_some())
        }
    }

    struct FixedRandom {
        outputs: Mutex<VecDeque<Vec<u8>>>,
    }

    impl FixedRandom {
        fn standard() -> Self {
            Self {
                outputs: Mutex::new(VecDeque::from([
                    vec![0x10; REFERENCE_BYTES],
                    vec![0x11; REFERENCE_BYTES],
                    vec![0x22; SECRET_BYTES],
                    vec![0x33; SECRET_BYTES],
                ])),
            }
        }
    }

    impl RandomSource for FixedRandom {
        fn fill(&self, destination: &mut [u8]) -> Result<(), ()> {
            let output = self.outputs.lock().unwrap().pop_front().ok_or(())?;
            if output.len() != destination.len() {
                return Err(());
            }
            destination.copy_from_slice(&output);
            Ok(())
        }
    }

    #[test]
    fn lifecycle_call_order_and_receipt_are_exact() {
        let store = FakeStore::default();
        let receipt = execute_probe(&store, &FixedRandom::standard()).unwrap();
        let target = format!("{TARGET_ACCOUNT_PREFIX}{}", "11".repeat(REFERENCE_BYTES));
        assert_eq!(
            store.calls(),
            vec![
                Call::Get(REGISTRY_ACCOUNT.to_owned()),
                Call::Get(target.clone()),
                Call::Set(REGISTRY_ACCOUNT.to_owned()),
                Call::Get(target.clone()),
                Call::Set(target.clone()),
                Call::Get(target.clone()),
                Call::Set(target.clone()),
                Call::Get(target.clone()),
                Call::Get(REGISTRY_ACCOUNT.to_owned()),
                Call::Delete(target.clone()),
                Call::Get(target),
                Call::Get(REGISTRY_ACCOUNT.to_owned()),
                Call::Delete(REGISTRY_ACCOUNT.to_owned()),
                Call::Get(REGISTRY_ACCOUNT.to_owned()),
            ]
        );
        assert_eq!(receipt.run_id, "10".repeat(REFERENCE_BYTES));
        assert_eq!(receipt.reference_fingerprint.len(), 16);
        assert!(receipt.lifecycle.absent_before_create);
        assert!(receipt.lifecycle.created);
        assert!(receipt.lifecycle.initial_read_matched);
        assert!(receipt.lifecycle.updated);
        assert!(receipt.lifecycle.updated_read_matched);
        assert!(receipt.lifecycle.deleted);
        assert!(receipt.lifecycle.absent_after_delete);
        assert!(!receipt.stale_cleanup_recovered);
        assert!(!receipt.cleanup_pending);
        assert!(!store.contains(REGISTRY_ACCOUNT));
    }

    #[test]
    fn failure_runs_finally_cleanup() {
        let store = FakeStore::fail_at(6);
        let error = execute_probe(&store, &FixedRandom::standard()).unwrap_err();
        let target = format!("{TARGET_ACCOUNT_PREFIX}{}", "11".repeat(REFERENCE_BYTES));
        assert_eq!(error.code, ProbeErrorCode::StoreFailure);
        assert!(!error.cleanup_pending);
        assert!(!store.contains(&target));
        assert!(!store.contains(REGISTRY_ACCOUNT));
        assert!(store.calls().contains(&Call::Delete(target)));
        assert!(store
            .calls()
            .contains(&Call::Delete(REGISTRY_ACCOUNT.to_owned())));
    }

    #[test]
    fn malformed_registry_fails_closed() {
        let store = FakeStore::default();
        store.seed(REGISTRY_ACCOUNT, b"not-a-lorepia-registry");
        let error = execute_probe(&store, &FixedRandom::standard()).unwrap_err();
        assert_eq!(error.code, ProbeErrorCode::InternalState);
        assert!(error.cleanup_pending);
        assert!(store.contains(REGISTRY_ACCOUNT));
        assert_eq!(store.calls(), vec![Call::Get(REGISTRY_ACCOUNT.to_owned())]);
    }

    #[test]
    fn stale_owned_target_is_recovered() {
        let store = FakeStore::default();
        let old = RegistryRecord::new(
            [0x44; REFERENCE_BYTES],
            sha256(&[0x55; SECRET_BYTES]),
            sha256(&[0x66; SECRET_BYTES]),
        );
        let old_target = old.target_account();
        store.seed(REGISTRY_ACCOUNT, old.encode().as_slice());
        store.seed(&old_target, &[0x66; SECRET_BYTES]);
        let receipt = execute_probe(&store, &FixedRandom::standard()).unwrap();
        assert!(receipt.stale_cleanup_recovered);
        assert!(!store.contains(&old_target));
        assert!(!store.contains(REGISTRY_ACCOUNT));
    }

    #[test]
    fn stale_unowned_target_is_never_deleted() {
        let store = FakeStore::default();
        let old = RegistryRecord::new(
            [0x44; REFERENCE_BYTES],
            sha256(&[0x55; SECRET_BYTES]),
            sha256(&[0x66; SECRET_BYTES]),
        );
        let old_target = old.target_account();
        store.seed(REGISTRY_ACCOUNT, old.encode().as_slice());
        store.seed(&old_target, b"unrelated credential");
        let error = execute_probe(&store, &FixedRandom::standard()).unwrap_err();
        assert_eq!(error.code, ProbeErrorCode::Collision);
        assert!(error.cleanup_pending);
        assert!(store.contains(&old_target));
        assert!(store.contains(REGISTRY_ACCOUNT));
        assert!(!store.calls().contains(&Call::Delete(old_target)));
    }

    #[test]
    fn process_lock_fails_fast() {
        let _guard = PROBE_LOCK.lock().unwrap();
        let error =
            run_with_process_lock(&FakeStore::default(), &FixedRandom::standard()).unwrap_err();
        assert_eq!(error.code, ProbeErrorCode::ProbeBusy);
        assert!(!error.cleanup_pending);
    }

    #[test]
    fn serialized_success_and_error_do_not_leak_secret_or_account() {
        let store = FakeStore::default();
        let receipt = execute_probe(&store, &FixedRandom::standard()).unwrap();
        let json = serde_json::to_string(&receipt).unwrap();
        assert!(!json.contains(&"22".repeat(SECRET_BYTES)));
        assert!(!json.contains(TARGET_ACCOUNT_PREFIX));
        assert!(!json.contains(&"11".repeat(REFERENCE_BYTES)));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            serde_json::json!({
                "runId": "10".repeat(REFERENCE_BYTES),
                "backend": "macos-keychain",
                "referenceFingerprint": receipt.reference_fingerprint,
                "lifecycle": {
                    "absentBeforeCreate": true,
                    "created": true,
                    "initialReadMatched": true,
                    "updated": true,
                    "updatedReadMatched": true,
                    "deleted": true,
                    "absentAfterDelete": true
                },
                "staleCleanupRecovered": false,
                "cleanupPending": false
            })
        );

        let platform_text = "raw platform password account detail";
        let error = ProbeError::new(ProbeErrorCode::StoreFailure, true);
        let error_json = serde_json::to_string(&error).unwrap();
        assert_eq!(
            error_json,
            r#"{"code":"STORE_FAILURE","cleanupPending":true}"#
        );
        assert!(!error_json.contains(platform_text));
    }

    #[test]
    fn target_collision_never_deletes_preexisting_credential() {
        let store = FakeStore::default();
        let target = format!("{TARGET_ACCOUNT_PREFIX}{}", "11".repeat(REFERENCE_BYTES));
        store.seed(&target, b"preexisting unrelated credential");
        let error = execute_probe(&store, &FixedRandom::standard()).unwrap_err();
        assert_eq!(error.code, ProbeErrorCode::Collision);
        assert!(!error.cleanup_pending);
        assert!(store.contains(&target));
        assert!(!store.calls().contains(&Call::Delete(target)));
    }

    #[test]
    fn every_operation_fault_after_registry_admission_is_fail_safe() {
        for fault_at in 3..=14 {
            let store = FakeStore::fail_at(fault_at);
            let error = execute_probe(&store, &FixedRandom::standard()).unwrap_err();
            let target = format!("{TARGET_ACCOUNT_PREFIX}{}", "11".repeat(REFERENCE_BYTES));
            let target_remains = store.contains(&target);
            let registry_remains = store.contains(REGISTRY_ACCOUNT);
            if target_remains || registry_remains {
                assert!(
                    error.cleanup_pending,
                    "fault {fault_at} left state without cleanupPending"
                );
            }
            if target_remains {
                assert!(
                    registry_remains,
                    "fault {fault_at} left an unregistered target"
                );
            }
        }
    }

    #[test]
    fn identical_run_and_target_references_are_rejected() {
        let random = FixedRandom {
            outputs: Mutex::new(VecDeque::from([
                vec![0x11; REFERENCE_BYTES],
                vec![0x11; REFERENCE_BYTES],
                vec![0x22; SECRET_BYTES],
                vec![0x33; SECRET_BYTES],
            ])),
        };
        let error = execute_probe(&FakeStore::default(), &random).unwrap_err();
        assert_eq!(error.code, ProbeErrorCode::RandomFailure);
        assert!(!error.cleanup_pending);
    }
}
