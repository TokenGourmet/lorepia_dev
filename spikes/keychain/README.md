# M-1 keychain spike

This disposable Tauri 2 + SvelteKit application tests one native credential
store lifecycle on each LorePia target. It is intentionally a functional probe,
not a password-management screen. Product visual design and animation are out
of scope.

## Boundary under test

The trusted `main` WebView can invoke only `run_keychain_m1_probe` with no
arguments. Rust generates a random credential account and two distinct UTF-8
test secrets, then performs:

```text
absent -> create -> exact read -> update -> exact read -> delete -> NoEntry
```

Secrets and the raw credential account stay native. The success response
contains only an independent run ID, a platform backend label, a truncated
SHA-256 account fingerprint, lifecycle booleans, and cleanup status. Native
errors are reduced to bounded codes without platform text.

The spike uses these official `keyring-core` stores:

- macOS login Keychain
- iOS Protected Data
- Windows Credential Manager with local persistence
- Linux Secret Service over zbus, with no automatic file fallback
- Android Keystore-backed encrypted SharedPreferences

A credential-store cleanup registry lets the next run recover a probe target
left by process termination. Before deleting a target, its current value must
match one of the hashes owned by that registry. This is same-process serialized
but not an atomic multi-process transaction.

## Run checks

From this directory:

```sh
npm ci
npm test
npm audit --audit-level=moderate
npm run check
npm run build
npm run verify:android-wrapper
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
```

Launch the native app with:

```sh
npm run tauri dev
```

The single button runs the OS-store probe. A successful unit test or compile is
not OS credential-service evidence; unit tests use a fake backend.

On iOS, `tauri ios build --no-sign` is compile-only. Do not install that output
to claim Keychain runtime evidence: the app has no effective
`application-identifier` or `keychain-access-groups` entitlement and the first
Protected Data call fails with OSStatus `-34018`. Before a simulator or device
run, inspect the installed app with `codesign -d --entitlements :-` and require
both entitlements from a valid development-team signing configuration.

## Evidence boundary

A physical-platform result needs the exact commit, raw run log, OS/hardware,
expected and actual lifecycle, and tester identity. This standalone spike has
no product SQLite, export, or crash-reporting subsystem, so it cannot by itself
close the separate non-leakage requirements for those future paths. See
[`../../docs/m1/keychain.md`](../../docs/m1/keychain.md).
