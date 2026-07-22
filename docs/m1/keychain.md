# M-1 keychain vertical boundary

This record defines the independent five-OS keychain spike. It is an
implementation and evidence contract, not a runtime `PASS`. Hosted compilation,
unit tests, a simulator, and an emulator do not replace the five physical-OS
cells in [`verification-matrix.md`](verification-matrix.md).

## Selected store boundary

The spike uses `keyring-core 1.0.0` directly with one official store crate per
target. It does not use the all-in-one `keyring` default, its insecure sample
store, or an automatic plaintext/encrypted-file fallback.

| Target | Store | Required behavior |
|---|---|---|
| macOS | `apple-native-keyring-store 1.0.1`, `keychain` | Development and direct-distribution builds use the login Keychain |
| iOS | `apple-native-keyring-store 1.0.1`, `protected` | App-local access group, cloud sync off; signed physical-device evidence required |
| Windows | `windows-native-keyring-store 1.1.0` | Search feature off; entries request local rather than roaming persistence |
| Linux | `zbus-secret-service-keyring-store 1.0.0`, `crypto-rust` | Use the desktop Secret Service; missing/locked headless service fails closed |
| Android | `android-native-keyring-store 1.0.0` | Ciphertext stays in dedicated SharedPreferences and its key stays in Android Keystore |

The upstream `keyring` documentation recommends that applications needing
explicit platform selection depend on `keyring-core` and the relevant stores
directly. The current integration pattern is cross-checked against the
[official Tauri Keyring Demo](https://github.com/open-source-cooperative/keyring-demo),
but that demo's dependency lock and Tauri version are not LorePia evidence.

## Public IPC contract

The spike exposes exactly one no-argument Tauri command,
`run_keychain_m1_probe`, to the exact `main` WebView. It does not expose general
create, read, update, or delete commands. Rust generates the credential account
and both test secrets, performs the entire probe natively, zeroizes the
probe-owned secret buffers, and returns only this proof shape:

```text
runId: 32 lowercase hexadecimal characters
backend: one fixed non-secret platform label
referenceFingerprint: first 16 lowercase hexadecimal SHA-256 characters
lifecycle:
  absentBeforeCreate
  created
  initialReadMatched
  updated
  updatedReadMatched
  deleted
  absentAfterDelete
staleCleanupRecovered
cleanupPending
```

Success requires every lifecycle field to be `true` and `cleanupPending` to be
`false`. Errors contain only a stable code and `cleanupPending`; raw platform
errors, secrets, credential accounts, and reference fingerprints are not error
fields. The frontend validates the exact object and refuses unknown or
secret-like fields before rendering anything.

## Probe and cleanup lifecycle

One process-wide non-blocking lock serializes backend initialization, stale
cleanup, and the complete probe. A concurrent request returns `PROBE_BUSY`.
All OS-store calls are blocking work and must not execute on the WebView event
loop.

The native sequence is:

1. Recover an earlier target named by the fixed cleanup-registry credential;
   verify it is absent, then remove the registry. Any malformed or unremovable
   registry fails closed before a new target is created.
2. Generate a random lowercase account and verify it does not already exist.
   A collision never deletes the pre-existing credential.
3. Write the target account into the cleanup registry before creating its
   credential, so a process termination can be recovered on the next run.
4. Prove absent, create, exact initial read, update, exact updated read, delete,
   and `NoEntry` after deletion.
5. On every ordinary success or failure path, delete the target, prove
   `NoEntry`, and delete the registry. If target cleanup cannot be proved, keep
   the registry and return `cleanupPending: true`.

This registry handles a previous process termination. The underlying store API
does not make the create/update distinction or registry sequence atomic across
two separate LorePia processes, so multi-process exclusion remains a production
integration requirement.

## Mobile-specific packaging

Android requires an NDK context before its native store can be initialized. The
generated `MainActivity` therefore calls a committed JNI initializer after
`super.onCreate`, and the store remains lazily initialized by the first probe.
CI checks that regeneration did not remove this hook. The dedicated
`lorepia-keyring-v1.xml` preferences file is excluded from both cloud backup and
device transfer using Android's
[backup exclusion rules](https://developer.android.com/identity/data/autobackup).

iOS uses the Protected Data store and its app-local default access group. An
unsigned simulator compile proves only that the toolchain accepts the code; it
does not prove entitlements, persistence, access while locked, or device
Keychain behavior.

An exact `4fde100` no-sign build was also launched on the named iOS 26.5
simulator to test that boundary. Its first Keychain lookup failed with OSStatus
`-34018` because the linker ad-hoc app had neither `application-identifier` nor
`keychain-access-groups` entitlements. The result is preserved as `FAIL`, not
reclassified as a blocked or successful runtime test
([candidate record](evidence/keychain-ios-simulator-4fde100/)). A signed build
with effective development-team entitlements is required before the lifecycle
can be evaluated on iOS; physical-device evidence remains a separate gate.

## Evidence this spike does not claim

- Unit tests use a fake store and prove lifecycle, cleanup, concurrency, and
  serialization behavior only. They do not exercise an OS credential service.
- A successful local macOS probe can update only the named macOS evidence row.
  It does not cover Windows, Linux, Android, or iOS.
- Deletion means the credential API subsequently returns `NoEntry`; it is not a
  forensic secure-erase claim about flash media.
- The independent spike has no product SQLite, export, or crash-reporting
  pipeline. Therefore it cannot by itself prove the full requirement that
  secrets are absent from those future subsystems.
- Physical evidence still needs a known diagnostic sentinel and scans of logs,
  crash output, SQLite, and exported settings in their relevant encodings. The
  production command must continue to keep that sentinel out of WebView IPC.
- Linux without a usable Secret Service must be preserved as `FAIL`. The
  encrypted-file fallback in the v2 plan needs a separate design and its own
  evidence before any affected product profile can pass.

The first Android emulator and macOS physical-host lifecycle observations are
preserved as bounded candidate records ([Android](evidence/keychain-android-emulator-38ce4c2/),
[macOS](evidence/keychain-macos-host-38ce4c2/)). They passed all seven lifecycle
assertions and cleanup on their named environments. The unsigned iOS simulator
attempt is preserved separately as the entitlement failure above. The Android,
macOS, and iOS physical-platform matrix cells remain `NOT RUN` because the
required physical runtime, broader non-leakage, and distribution-profile
evidence is not complete.

## Upstream references

- [`keyring-core 1.0.0` API and store selection](https://docs.rs/crate/keyring-core/1.0.0)
- [`keyring 4.1.5` feature and target dependencies](https://docs.rs/crate/keyring/4.1.5/source/Cargo.toml.orig)
- [official demo Android JNI initialization](https://github.com/open-source-cooperative/keyring-demo/blob/5ce3d08786d752f7b14cf341a5357572f00ceddb/src-tauri/src/lib.rs#L62-L98)
- [Windows credential blob limit](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw)
- [Freedesktop Secret Service Item API](https://specifications.freedesktop.org/secret-service/latest/org.freedesktop.Secret.Item.html)
