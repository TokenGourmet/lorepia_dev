# M-1 independently terminable script-runner candidate

This record defines the disposable `spikes/script-runner` experiment. It tests
whether LorePia can execute a bounded, pure JavaScript hook outside the trusted
WebView event loop and terminate it from the host. It is not the product card
API, an imported-script feature, or five-OS runtime approval.

## Decision at this checkpoint

**Status: `RETAIN AS CONDITIONAL CANDIDATE; PRODUCT EXECUTION REMAINS OFF`.**
The fixed 15-case suite passes in an actual Tauri WKWebView on the macOS arm64
host, in the installed APK on the available Android ARM64 emulator, and in an
iOS 26.5 simulator WKWebView after the engine warmup regression was fixed. The
candidate therefore demonstrates that the synchronous iframe busy-loop failure
has a workable replacement on those three runtimes.

This does not enable imported JavaScript. The current product has no
source-to-runner path, and Store-Safe profiles continue to treat executable card
content as inert. Product extraction, a reviewed versioned hook contract,
Windows/Linux runtime evidence, physical Android/iOS evidence, and written
store-policy review remain gates. The exact implementation subject is
`58bab9d697533b697b098b1a6130665d1ad7cd04`. Its local runtime observations,
artifact hashes, and remaining evidence limitations are indexed in
[`script-runner-58bab9d`](evidence/script-runner-58bab9d/).

## Candidate architecture boundary

The trusted Tauri WebView is only a controller and receipt renderer. It never
evaluates the fixture source. Each invocation creates a fresh static module
Worker; that Worker creates a fresh QuickJS-WASM engine, executes one case, and
is then terminated. Only the fixed case identifier, protocol version, and a
128-bit invocation identifier cross from the controller to the Worker. Source,
input JSON, paths, and engine limits are bundled test inputs and do not cross
that boundary.

The native surface is deliberately empty:

- Rust registers no Tauri command or invoke handler;
- the sole capability file grants no permission;
- the frontend has no `@tauri-apps/api` dependency;
- the runner uses no Tauri Channel, event, command, source transport, or
  process-global Tauri Channel fetch queue;
- the trusted Worker loader fetches only its bundled same-origin WASM asset;
  the guest QuickJS global has no `fetch` or other network API; and
- CSP denies frames and objects, permits only a same-origin Worker, and permits
  WebAssembly compilation for the fixed engine asset.

This means the candidate does not depend on the process-global Tauri Channel
fetch queue and does not deserialize imported source in Rust before applying a
limit. The only controller/Worker messages are schema-strict and capped at 4096
bytes. A successful output is size-checked inside QuickJS, hashed inside the
Worker, and represented across the Worker boundary by byte count and SHA-256,
not the raw value.

## Termination and resource contract

The fixed limits in `runner-contract.ts` are:

| Boundary | Limit |
|---|---:|
| Source UTF-8 | 64 KiB |
| Input JSON UTF-8 | 16 KiB |
| Output JSON UTF-8 | 16 KiB |
| Controller/Worker message | 4096 bytes |
| QuickJS engine memory | 8 MiB |
| QuickJS stack | 256 KiB |
| Imported WebAssembly memory | 16 MiB initial, 32 MiB maximum |
| Trusted pre-ready engine warmup | 500 ms deadline |
| QuickJS interrupt deadline | 50 ms |
| Raw-Worker wedge acknowledgement | 250 ms deadline |
| Host execution watchdog | 500 ms |
| Worker boot watchdog | 20 s |

Before the Worker announces `READY`, a trusted constant expression warms the
new engine under its separate 500 ms cap. This prevents first-use compilation
latency from consuming the untrusted guest's 50 ms budget; the guest deadline
is not widened. The independent host watchdog calls `Worker.terminate()` from
outside the execution Worker if no bounded receipt arrives. The suite includes
a trusted raw Worker busy loop that does not enter QuickJS so this fallback
cannot pass merely because the engine's cooperative interrupt works. That case
must first return an exact, invocation-bound `WEDGE_STARTED` acknowledgement
within 250 ms; only then does the host arm the 500 ms termination watchdog.
Silence, a dropped request, or a forged acknowledgement is
`CONTRACT_FAILURE`, not termination proof. The controller admits one Worker at
a time and terminates the Worker on success, failure, cancellation, timeout,
or malformed message.

The WebAssembly maximum bounds the engine's linear memory; it is not a claim
that the browser process has a 32 MiB resident-set ceiling. Source, input,
output, message size, one-at-a-time admission, and per-invocation Worker
disposal are separate bounds around the remaining host allocation surface.
Product work must preserve that distinction and add platform measurements.

## Fixed 15-case acceptance suite

The self-authored, hash-pinned fixtures and trusted watchdog harness prove:

1. one allowed pure JSON transform;
2. infinite-loop engine interruption, then a clean recovery invocation;
3. recursive stack pressure interruption, then recovery;
4. allocator pressure interruption, then recovery;
5. absence of Tauri, DOM, network, Worker, Node, and Deno globals, then
   recovery;
6. oversized-output rejection before raw output crosses the Worker boundary,
   then recovery;
7. stable redaction of a thrown raw error, then recovery; and
8. host termination of a deliberately wedged raw Worker while the trusted
   host heartbeat advances, then recovery.

All recovery cases create a new Worker and engine. A result is `PASS` only when
all 15 case identifiers return the exact stable outcome code; the watchdog case
must also report at least one host heartbeat tick.

## Host-message contract hardening after the runtime candidate

The current source adds unit regressions around the controller boundary beyond
the fixed runtime corpus. The controller now requires the exact top-level
`RESULT` key set and treats a message deserialization failure as
`CONTRACT_FAILURE`, terminating that invocation. Tests also cover malformed,
forged, extra-key, oversized, cross-invocation, duplicate, and replayed
`READY`, `RESULT`, and `WEDGE_STARTED` messages; cancellation before readiness;
boot timeout; runtime error; and fresh-instance recovery after each failure
class.

These are host-controller unit results, not new WebView runtime evidence. The
packaged macOS, Android-emulator, and iOS-simulator observations below remain
tied to exact commit `58bab9d`. The hardened source must be committed, packaged,
and rerun before those environments can be claimed for the new implementation.
It also does not prove Tauri Channel queue ownership: this candidate does not
use that queue, and any future streaming transport still needs its own
destination and invocation binding.

## Current runtime observations

| Platform/runtime | Result | What it establishes | Limitation |
|---|---|---|---|
| macOS 26.5.2 build 25F84, arm64 physical host, packaged Tauri WKWebView | **PASS (`15/15`)** | The actual packaged macOS Tauri WebView completed every fixed case, including external Worker termination and post-termination recovery | Exact implementation commit and executable hash are pinned; a complete raw receipt/log bundle is not committed |
| Android 16 / API 36 `sdk_gphone64_arm64`, WebView `133.0.6943.137`, installed debug APK | **PASS (`15/15`)** | The Android emulator WebView completed every fixed case, including external Worker termination and recovery; the pulled installed APK matched the built APK hash | Emulator only; not a physical-device or store-policy result |
| iPhone 17 simulator, iOS 26.5, no-sign ARM64 simulator app | **PASS (`15/15`)** | The simulator WKWebView completed every fixed case after QuickJS warmup moved before Worker readiness, including exact `WEDGE_STARTED` acknowledgement, external termination, and recovery; the installed executable matched the built hash | Simulator only; the initial pre-fix run was `14/15` and no signed physical-device behavior is proved |
| Windows WebView2 | `NOT RUN` | Nothing at runtime | Hosted Windows compile/test and emitted-boundary verification passed for `2ab7672`, but no packaged WebView2 runtime was launched |
| Linux WebKitGTK | `NOT RUN` | Nothing at runtime | Hosted Linux compile/test and emitted-boundary verification passed for `2ab7672`, but no packaged WebKitGTK runtime was launched |

These observations retire neither the preserved unsafe iframe baseline nor its
failure record. They select a different execution boundary. They also do not
promote the Android physical-device or iOS physical-device cells in
[`verification-matrix.md`](verification-matrix.md).

## Product extraction gate

The product now has a
[metadata-only quarantine contract](../m0/imported-executable-quarantine.md)
that can identify inert JavaScript content without accepting source or naming
hooks. This is non-executable importer preparation only. The runner remains in
this disposable spike, and the actual product extraction gate below is still
open.

Imported JavaScript remains disabled until a product change separately proves
all of the following:

1. The product accepts only a reviewed, versioned pure-data hook contract and
   enforces source/input limits before creating the Worker.
2. Imported source never executes in the main WebView or native Tauri process
   and never traverses a Tauri command, Channel, event, or process-global Tauri
   Channel fetch queue.
3. The product preserves fresh-engine isolation, one-at-a-time admission,
   bounded receipts, engine interruption, external termination, and recovery.
4. Network, DOM, Tauri, filesystem, nested Worker, Node, and platform globals
   remain absent under product packaging and negative tests.
5. Windows, Linux, physical Android, and signed physical iOS runs pass the
   applicable termination, memory, transport, lifecycle, and recovery cases.
6. Store-Safe non-enablement tests and dated Android/iOS policy reviews approve
   the exact shipped profile.

If a target cannot load the fixed Worker/WASM asset, enforce the deadline, or
terminate and recover, preserve `FAIL`; do not fall back to same-event-loop
`eval`, a sandboxed iframe, or a raw native IPC source path.

## Reproduction and CI scope

From `spikes/script-runner`:

```sh
npm ci
npm test
npm audit --audit-level=moderate
npm run check
npm run build
npm run verify:built
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
```

Desktop CI runs those source, contract, frontend, and Rust checks. Android CI
cross-compiles a debug ARM64 APK, and iOS CI compiles a debug ARM64 simulator
app. All five script-runner jobs passed in the exact-candidate
[M-1 run](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674968473),
whose complete matrix passed all 35 jobs,
and the exact-implementation
[product scaffold run](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674867356)
passed all six jobs. Hosted compilation does not launch a WebView and cannot
replace the local runtime observations or future physical-device evidence.

## Evidence this spike does not claim

- It does not accept arbitrary imported source through a product API.
- It does not define card hook names, compatibility semantics, persistence,
  migrations, UI rendering, or native capabilities.
- It does not prove useful-script performance or compatibility beyond the fixed
  self-authored corpus.
- It does not prove physical mobile lifecycle, thermals, process memory, or
  store acceptance.
- It does not establish Windows, Linux, or physical iOS runtime support; the
  iOS observation is limited to one no-sign simulator image.
