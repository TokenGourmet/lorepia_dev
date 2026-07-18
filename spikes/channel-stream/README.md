# M-1 Channel streaming spike

This disposable Tauri 2 + SvelteKit application validates LorePia's native-to-webview streaming contract before the production workspace is created. It deliberately contains only a functional test surface; product visual design and animation are out of scope.

## Protocol under test

- Rust sends typed `started`, `delta`, and one terminal event through `tauri::ipc::Channel`.
- Every event carries a request ID and a contiguous, monotonically increasing sequence number.
- The producer batches mock upstream chunks in a configured 16-50 ms window.
- The frontend acknowledges consumed sequence numbers. A bounded in-flight window prevents an unbounded producer queue, and consumer delay expands the effective batching window without dropping text.
- Cancellation produces one `cancelled` terminal event and preserves the exact partial text and last sequence in the backend snapshot.
- Deterministic failure injection produces one `failed` terminal event with the same recovery snapshot guarantees.

The mock proves the transport and lifecycle mechanics only. It does not claim real provider SSE, mobile physical-device behavior, persistence, or production performance.

## Isolation baseline

The `/isolation` route is a negative-test harness, not proof of a production-safe
plugin runtime. Unsafe-baseline commit `2f8e130` passed the packaged macOS
effect-level suite because direct Tauri transport was not exposed there, but it
failed on an Android 16/API 36 emulator when the iframe invoked
`privileged_probe` and changed native state. The preserved evidence, selected
host-only 256-bit-token Rust broker fallback, and remaining same-event-loop busy
loop blocker are documented in
[`../../docs/m1/isolation.md`](../../docs/m1/isolation.md). Store-Safe mobile
imports keep JavaScript and Lua disabled.

This combined disposable spike still registers the four raw Channel transport
commands for its main window. The broker is the only path to the fixture's
sanitizer and probe sinks, not yet the only native command path. Production work
must broker those remaining commands or move plugin execution to a separately
scoped WebView. Tauri also decodes the outer IPC command arguments before the
Rust broker's size and admission checks run, so those checks do not bound the
first allocation of an oversized direct command. That pre-handler boundary is
another reason this harness is not a production plugin runtime.

## Run checks

From this directory:

```sh
npm ci
npm test
npm audit --audit-level=moderate
npm run check
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
```

Launch the native application with:

```sh
npm run tauri dev
```

Use **Start stream** for normal completion and **Cancel stream** during delivery to verify a partial result. The page exposes request state, sequence and ACK progress, in-flight count, effective batching window, partial text, and the final backend snapshot without decorative UI.

## Evidence boundary

Passing Rust tests validates the deterministic state machine and protocol invariants in-process. A desktop launch validates the local WebView IPC path. Android/iOS compilation or simulator execution is recorded separately and does not satisfy a physical-device M-1 cell. See [`../../docs/m1/README.md`](../../docs/m1/README.md).
