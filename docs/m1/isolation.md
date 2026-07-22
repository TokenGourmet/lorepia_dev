# M-1 isolation unsafe-baseline record

This record preserves the cross-platform isolation result from commit `2f8e130`.
That commit is the **unsafe baseline**, not an approved plugin-runtime design. The
observations below do not change any physical-device cell in
[`verification-matrix.md`](verification-matrix.md).

## Decision summary

- The locally packaged macOS release `.app` completed the effect-level fixture suite with `11/11`
  passing results and the native probe count remained `1 -> 1`. The iframe did
  not expose a direct Tauri invoke transport on that runtime, so this is **not**
  proof that a Tauri ACL rejected an otherwise valid privileged request.
- The Android emulator exposed the transport. The sandboxed iframe invoked
  `privileged_probe` successfully and changed the native side-effect count from
  `1 -> 2`. The direct-invoke negative test therefore **failed**.
- The immediate fallback retires the demonstrated privileged raw wrappers and
  routes their allowed operations through a Rust broker protected by a
  host-only, cryptographically random 256-bit session token. The combined
  disposable spike still has six raw stream-lifecycle commands. A later
  Tauri 2.11.5 source audit found that a separately scoped same-process Tauri
  WebView is also insufficient because large Channel data uses an ACL-exempt,
  process-global fetch queue without WebView ownership binding. See
  [`channel-ipc-boundary.md`](channel-ipc-boundary.md).
- Tauri deserializes the outer command envelope before the Rust broker runs.
  Consequently, the broker's 512 KiB request bound, rate limit, and in-flight
  cap do not prevent the first allocation and parse of an oversized direct IPC
  argument. A separately scoped plugin WebView or an earlier transport-level
  admission/size boundary is still required before production use.
- The host applies a fixed-window admission gate before reading iframe message
  data and routes unknown message types without enumerating their keys. Browser
  structured clone still materializes a `postMessage` payload before the host
  handler runs, so this bounds decoder work but not the first browser-side
  allocation. A separate execution/transport boundary remains required for a
  hard memory bound.
- That fallback does not solve a synchronous busy loop in an iframe sharing the
  host event loop. A cooperative ping/pong watchdog cannot run while the shared
  event loop is blocked. Imported JavaScript execution therefore remains
  blocked pending a terminable execution boundary and platform evidence.
- Store-Safe mobile builds keep imported JavaScript and imported Lua **OFF**.
  An executable iframe in the privileged main WebView is not an allowed
  fallback.

## Expected invariant

The production invariant is that an imported plugin frame cannot invoke a raw
Tauri command or cause its native side effect. The current combined spike does
not yet meet that full invariant. For each retired privileged wrapper, a passing
direct-invoke test requires both:

1. the invocation is rejected or the transport is absent; and
2. the independently measured native side-effect counter is unchanged.

Transport absence is useful effect-level evidence, but it does not exercise the
ACL decision itself. A runtime that exposes the transport must still deny the
request without changing native state.

## Preserved execution records

Both observations were made at `2026-07-18T17:21:11Z` against commit `2f8e130`.

| Runtime | Evidence level | Result | Native probe count | Interpretation |
|---|---|---|---|---|
| Locally packaged macOS release `.app` | Local packaged-app runtime | `PASS` at effect level (`11/11`) | `1 -> 1` | Direct invoke transport was not exposed; this does not prove ACL denial |
| Android 16 / API 36 emulator, `sdk_gphone64_arm64`, WebView `133.0.6943.137` | Emulator runtime | `FAIL` | `1 -> 2` | The iframe successfully reached `privileged_probe` |

### macOS packaged-app observation

- Commit: `2f8e130`
- UTC: `2026-07-18T17:21:11Z`
- Scenario: launch the locally packaged release `.app`, open the isolation route,
  and run the complete negative-test fixture.
- Expected: all prohibited effects are absent and the native probe count is
  unchanged.
- Actual: the effect-level fixture reported `11/11 PASS`; the probe count was
  `1 -> 1`; direct invoke reported that the Tauri transport was not exposed in
  the frame.
- Limitation: because no valid direct request reached the ACL, this result must
  not be described as an ACL-denial proof. The exact launch transcript and a
  retained raw screenshot were not attached to this record.

### Android emulator failure

- Commit: `2f8e130`
- UTC: `2026-07-18T17:21:11Z`
- Runtime: Android 16 / API 36 emulator, `sdk_gphone64_arm64`.
- WebView: `133.0.6943.137`.
- Expected: direct `privileged_probe` invocation is denied and the native count
  remains `1 -> 1`.
- Actual: direct `privileged_probe` invocation succeeded and the native count
  changed from `1 -> 2`; the negative test reported `FAIL`.
- APK SHA-256:
  `e3d02a7e78cfe567211fcb3535964bf6689d09ff6c2622dff835f3ac0e37a004`.
- Screenshot SHA-256:
  `a4a2ed8ebc8f77908f8d2ee0fcc8d54c93f9b8914c3db7d900cae6e92ec4843a`.
- Screenshot retention: the screenshot was captured under `/tmp` and was not
  copied into the repository. The hash preserves its recorded identity but is
  not a retrievable raw artifact; future qualifying runs must retain the file
  in an approved evidence store.

The local Android SDK and emulator reproduction sequence was:

```sh
cd /Users/codexer/LorePia_dev/spikes/channel-stream

export JAVA_HOME="$(brew --prefix openjdk@21)/libexec/openjdk.jdk/Contents/Home"
export ANDROID_HOME="/Users/codexer/Library/Android/lorepia-sdk"
export NDK_HOME="/Users/codexer/Library/Android/lorepia-sdk/ndk/28.2.13676358"
export PATH="$ANDROID_HOME/platform-tools:$PATH"

npm run tauri -- android build --debug --target aarch64 --apk --ci

adb devices
adb -s emulator-5554 install -r \
  /Users/codexer/LorePia_dev/spikes/channel-stream/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb -s emulator-5554 shell am force-stop dev.lorepia.spike.channelstream
adb -s emulator-5554 shell am start -n \
  dev.lorepia.spike.channelstream/.MainActivity
```

At unsafe-baseline commit `2f8e130`, open the isolation route and run the
negative-test suite. Record the historical `direct-tauri-invoke-blocked` result
and the independently queried native probe count before and after the iframe
attempt. Candidate builds use the newer retired-command audit described below.

## Candidate broker observation

Commit `a42d33d4e843da353f042d435133d3ac5f988fa4` was rebuilt and exercised as a
locally packaged macOS release app and an Android 16/API 36 emulator APK. The
full immutable summary, artifact hashes, exact failures, sink counters, and
watchdog observations are in
[`evidence/a42d33d-local-isolation.md`](evidence/a42d33d-local-isolation.md).

| Runtime | Result | Key interpretation |
|---|---|---|
| macOS 26.5.2 arm64, locally packaged release `.app` | `18/18 PASS` | Stale token, exact command surface, broker policy, sink delta, and watchdog passed; direct iframe Tauri transport was absent, so this is not ACL-decision proof |
| Android 16/API 36 `sdk_gphone64_arm64`, WebView `133.0.6943.137` | **FAIL**, `15/18 PASS`, `18/18` complete | Three direct native calls timed out and remain `INCONCLUSIVE`; the final sink audit found no extra monitored effect; emulator evidence does not change a physical-device cell |

This candidate retires the raw sanitizer/probe wrappers demonstrated by the
unsafe baseline, but it does not close M-1 or establish a production plugin
runtime.

## Selected fallback and remaining blocker

The selected replacement for raw iframe-to-Tauri invocation has these target
requirements. They are exit criteria, not a claim that the current candidate
already proves every item:

1. Privileged raw wrappers are removed immediately. Before production use, all
   remaining raw app commands must be unreachable from imported execution. A
   second stock Tauri WebView in the same process does not satisfy this item.
2. The trusted host creates a fresh CSPRNG 256-bit session token. The token is
   never placed in the plugin URL, iframe DOM, plugin-readable storage, or
   plugin `postMessage` payload.
3. A Rust broker rejects missing, malformed, stale, replayed, or incorrect
   tokens before dispatch and exposes only a typed method allowlist with bounded
   payloads, rate limits, and explicit permissions.
4. Regression tests attempt both raw commands and the broker with absent,
   guessed, stale, and replayed tokens, while independently checking that no
   prohibited native side effect occurred.

The candidate local observations cover missing/wrong tokens, registration
takeover attempts, replayed request IDs, and token rotation/staleness. Android
direct-call response semantics remain inconclusive because the native callback
timed out. Runtime rate-limit abuse and iframe self-navigation egress remain
unverified.

The broker's internal admission order is covered by source tests, including
lifecycle calls and lock contention. Those tests begin after Tauri has decoded
the command arguments, so they are not evidence that oversized direct IPC is
bounded at the WebView transport boundary.

This fallback addresses the demonstrated native-command path only. It cannot
make a same-event-loop iframe preemptible. Imported JavaScript remains a blocker
until its execution can be terminated independently while the host stays
responsive under a busy loop. The Store-Safe profile therefore continues to
import JavaScript and Lua as inert/quarantined data only.

Commit `f7a3270` additionally keeps each current Channel event and JSON command
response within a 4096-byte application budget, below the audited Tauri
large-payload threshold. Terminal events no longer repeat the accumulated text;
the final snapshot returns byte-length and SHA-256 receipts. This prevents the
current ten-command spike from entering the shared large-response queue, but
it is a version-pinned mitigation rather than a general Tauri isolation fix.

## Evidence needed to replace this baseline

- Retained raw logs and screenshots for the exact candidate commit.
- Runtime negative tests on Windows, macOS, Linux, a physical Android device,
  and a physical iOS device.
- Proof that every state-changing raw app command is unavailable to imported
  execution; the research iframe still shares the privileged `main` WebView
  label and cannot satisfy this exit condition on Android.
- A transport-level payload/admission test proving oversized direct IPC is
  rejected before Tauri constructs the full command argument. Stock Tauri's
  same-process WebView capability split is not sufficient evidence.
- Missing/wrong/stale/replayed 256-bit broker-token tests with an unchanged
  native side-effect counter.
- A busy-loop test proving that the host remains usable and can terminate the
  imported-code execution context.
- A profile regression test proving Store-Safe mobile builds cannot re-enable
  imported JavaScript or Lua through a manifest, migration, import, or stale
  setting.
