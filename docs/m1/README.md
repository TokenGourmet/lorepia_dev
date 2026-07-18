# M-1 verification contract

M-1 exists to remove architecture risk before LorePia's production workspace and plugin API are frozen. A demo, a successful compile, or an undocumented manual check is not enough to close it.

The current state is recorded in [`verification-matrix.md`](verification-matrix.md). The preserved cross-platform unsafe isolation baseline and selected fallback are recorded in [`isolation.md`](isolation.md). Every claim must point to reproducible evidence from the exact commit being evaluated.

## Result vocabulary

Use only these states in the matrix:

- `NOT RUN`: no qualifying execution evidence exists.
- `PASS`: the acceptance criteria ran on the named environment and all assertions passed.
- `FAIL`: the test ran and at least one criterion failed. Preserve the failure evidence.
- `BLOCKED`: execution could not start because a named prerequisite was unavailable. This is not a pass and does not close M-1.

A result record must contain the commit SHA, UTC timestamp, OS and version, hardware or runner image, exact command/scenario, expected result, actual result, raw log or artifact link, and tester/CI run identity. A local summary without raw evidence remains `NOT RUN`.

## Evidence levels are not interchangeable

| Level | What it proves | What it does not prove |
|---|---|---|
| Source/unit test | Pure logic behaved under the test runtime | Tauri IPC, WebView, OS service, audio, or device behavior |
| Native compile | The target toolchain accepted the code | App launch or runtime behavior |
| Simulator/emulator | Behavior in that simulator image | Physical device WebView, keychain, audio, lifecycle, thermals, or performance |
| Physical-device smoke | The named build ran on the named device | Other devices or broad performance compliance |
| Physical-device measured test | The named scenario met its threshold on the named device | Untested OS/device versions |

CI names mobile jobs as compile-only. A green mobile compile job must never be copied into a real-device cell in the matrix.

## Six vertical capabilities

Each capability must be executed on Windows, macOS, Linux, a physical Android device, and a physical iOS device.

1. **SQLite/FTS5**
   - Create, migrate, close, and reopen a file-backed database.
   - Prove persistence and the intended concurrent read/write behavior.
   - Insert the fixed Korean fixture and return the golden FTS query results in a deterministic order.
   - Record the tokenizer and SQLite build options used on that platform.
2. **Lua limit enforcement**
   - Run an allowed fixture successfully.
   - Terminate an infinite/over-budget fixture at whichever comes first: the 50 ms deadline or instruction cap, without terminating the host.
   - Prove dangerous standard libraries such as `os` and `io` are unavailable.
   - Record peak memory and the configured memory ceiling.
3. **File import**
   - Select or open the approved fixture through the platform-appropriate path and read identical bytes/hash.
   - Reject traversal, oversized, malformed, and unsupported inputs without writing outside the staging area.
   - On mobile, exercise the real system document picker on a physical device.
4. **Keychain**
   - Create, read, update, and delete a unique test secret using the OS credential service.
   - Prove the secret is absent from SQLite, logs, crash output, and exported settings.
   - Linux headless failure must be recorded as `FAIL`; a selected encrypted-file fallback needs its own test evidence before that product profile can pass.
5. **Tauri Channel streaming**
   - Preserve monotonically increasing `seq` with no duplicate or dropped payload under a deliberately slow consumer.
   - Keep normal batches inside the 16-50 ms design window and demonstrate bounded buffering/backpressure.
   - Cancellation must stop upstream work, emit one terminal outcome, and prevent later data chunks.
   - Interrupted streams must retain the exact last sequence and partial payload required for recovery.
6. **Audio**
   - Load, play, pause/resume, seek, stop, and release the approved local fixture.
   - Prove app background/foreground behavior and resource release on mobile.
   - A headless compile or API mock cannot pass this runtime gate.

## Negative-test gate

All cases below require an executable regression test or recorded OS/WebView scenario. The host must remain within the declared resource bound, deny the prohibited effect, and produce a user-diagnosable error.

| Area | Required attacks and pass condition |
|---|---|
| Archive/import | Zip bomb, `../` and absolute-path traversal, symlink escape, oversized entry/count, malformed PNG chunk; no out-of-staging write and bounded CPU/memory/disk |
| Regex | Catastrophic-backtracking compatibility pattern and oversized input; terminate at the 10 ms policy bound and report the rejected pattern |
| Lua | Infinite loop, recursion/allocator pressure, and forbidden-library access; terminate at the configured cap while the host remains responsive |
| JavaScript | Busy loop/unresponsive iframe; watchdog disables or reloads the module while the host remains usable on the tested WebView |
| IPC/broker | Direct Tauri invoke, forged `postMessage`, undeclared permission, rate-limit bypass, and default network access; every request is denied without privileged side effects |
| Network | Undeclared origin, redirect, DNS-rebinding target, and direct browser request; default deny remains effective and approved access only traverses the broker |
| Rendering | Sanitizer-bypass fixtures through every HTML-producing hook; final output contains no executable script, handler, unsafe URL, or app-DOM escape |

If the native design fails, retain `FAIL` and document the selected fallback, owner, consequence, and regression test. M-1 closes only after the selected product profile passes its corresponding defense; prose saying that risk is accepted is insufficient.

## Compatibility and performance gate

Before freezing `specs/plugin-api.md`:

- Record the first Risu behavior-observation set without copying source code.
- Add only self-authored, explicitly permitted, or compatible open-license fixtures, including provenance and license metadata.
- Run golden behavior tests over every fixture and preserve conversion differences.
- Name the low-end Android device and five-year-old Windows reference machine, OS versions, power mode, dataset, warm-up, sample count, and measurement command.
- Measure p95 from raw samples; do not average percentile values.
- Evaluate every threshold in section 3 of the v2 plan and retain the raw samples.

The plugin API remains provisional until these gates and the relevant isolation tests are complete.

The unsafe-baseline isolation result does not close this gate: Android emulator
execution demonstrated a privileged native side effect, while the macOS result
only proved transport absence on that runtime. See [`isolation.md`](isolation.md).

## Store-Safe hard rule

Until written policy clearance exists, Store-Safe builds must not execute imported JavaScript **or imported Lua**. Imported packages may be inspected and quarantined as inert data, but executable payloads remain disabled. Declarative templates and data binding are the only imported behavior allowed in this profile.

The selected host-only 256-bit-token Rust broker fallback addresses the
privileged sanitizer/probe command path, but not a synchronous busy loop sharing
the host event loop. The disposable combined spike still exposes four Channel
transport commands to its one Tauri window; those commands must be brokered or
moved to a separately scoped WebView before this can become a production plugin
runtime.
That unresolved blocker is preserved in [`isolation.md`](isolation.md).

Enabling either language requires all of the following:

1. A dated policy review naming the store, guideline version, reviewer, decision, and submission constraints.
2. Passing real-device isolation, broker, resource-limit, sanitizer, and network-denial evidence on the affected mobile platform.
3. A profile-specific negative test proving that an unapproved build cannot re-enable execution through a manifest, import, migration, or stale setting.
4. An explicit review decision recorded in the matrix; absence of a decision means disabled.

## M-1 exit decision

M-1 may close only when all statements below are true:

- All 30 platform-by-capability cells contain `PASS` or preserved `FAIL` evidence; none remain `NOT RUN` or `BLOCKED`.
- Every preserved `FAIL` has a selected fallback whose own acceptance and negative tests pass on the affected product profile.
- Every negative-test row has executable or recorded platform evidence with no unresolved prohibited effect.
- Compatibility observations, fixture provenance, and golden tests are complete enough to make the plugin API decision.
- Performance reference hardware and raw p95 evidence are recorded.
- The Store-Safe JS/Lua decision is explicit per mobile platform and the disabled-by-default regression test passes.
- The review records one decision: proceed to M0, revise the architecture, or stop the affected platform/profile.

## CI scope

`.github/workflows/m1.yml` performs:

- Windows/macOS/Linux: `npm ci`, frontend contract tests, Svelte/TypeScript check, frontend build, Rust format, Rust tests, Clippy with warnings denied, and Rust check.
- Android: debug ARM64 APK compilation on a hosted runner.
- iOS: debug ARM64 simulator compilation on a hosted macOS runner.

Hosted CI does not claim audio output, keychain UI/service behavior, WebView isolation, document-picker behavior, or physical-device smoke. Those remain matrix work with real-device evidence.
