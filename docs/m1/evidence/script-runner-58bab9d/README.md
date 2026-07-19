# Script-runner exact-candidate runtime observations: `58bab9d`

This record preserves the local rebuild and runtime observation of the
disposable QuickJS-WASM Worker candidate at exact implementation commit
`58bab9d697533b697b098b1a6130665d1ad7cd04` (tree
`f043978a546e6c212d5ba37844a679c50108b702`). The implementation commit is based
on `f150d38`; documentation is recorded separately so it can name the immutable
implementation subject. Packaged artifact hashes are retained below. Full raw
build logs and screenshots are not committed, so these results remain local
candidate evidence and do not change a physical-mobile cell or close the
JavaScript product gate.

- Local exact-commit rerun: 2026-07-19, approximately 14:21-14:26 KST
- Branch: `agent/m1-script-runner`
- Implementation commit: `58bab9d697533b697b098b1a6130665d1ad7cd04`
- Implementation tree: `f043978a546e6c212d5ba37844a679c50108b702`
- Windows checkout policy fix: `2ab7672a5bf0d9683bb5b883d07578d76e4afdf5`
- Base commit: `f150d38a772cdef0b4cebf6be9bca412b8e34610`
- Tester: local Codex execution and UI operation
- Expected UI result: exact fixed suite `15/15 PASS`

## Locked inputs and emitted artifacts

These hashes identify the committed inputs and exact local outputs used for the
post-commit rerun:

| Input | SHA-256 |
|---|---|
| `spikes/script-runner/package-lock.json` | `99f5e7474db23d8228c61ca1081a5577ac597182b3db79f44a86285fe1458caa` |
| `spikes/script-runner/src-tauri/Cargo.lock` | `f3c578d6c431a43018bf52e25735958b46ef4b6f36566aee5999897c1c2b6797` |
| `spikes/script-runner/fixtures/catalog.json` | `c971791fba72594e79556d5a8ea0fce89ccf781c9bd1e96a3d4adf2e9ca29dfa` |

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| macOS packaged executable `lorepia-script-runner-spike` | 29,144,656 | `969be94fcd8c1f1eb6893edce37614dd818da9212270f2e361f7f7f7b5ad023e` |
| Android universal debug APK | 132,166,770 | `73de102b6183dc582324ede4a8eff3b46519687726e71ea12d5f8b39ea9fbb44` |
| iOS ARM64 simulator executable | 85,904,328 | `9924da0228b7fdd7c0e0b28bd71c722539a2989104e2a63ee59affb6cd1857cf` |
| emitted module Worker | 10,897 | `96208b2d9f796441209d7b9e2646f28d552659875e1cd8454236f7a0bef3a748` |
| emitted QuickJS WebAssembly module | 503,134 | `105c3bed22d457e43e3d1c3c1c6959fda62a8fe06f0fc8a985303c3a2be72232` |

The APK pulled back from the installed Android package matched the built APK
hash. The executable in the installed iOS simulator container matched the built
simulator executable hash.

Local common toolchain inventory was Node.js `22.23.0`, npm `10.9.8`, Rust
`1.97.1`, and Xcode `26.6` build `17F113`. The exact Java/NDK environment used
by the Android build was Homebrew OpenJDK `21.0.11` and Android NDK
`28.2.13676358` from `/Users/codexer/Library/Android/lorepia-sdk`. The raw
tool output is not retained in this evidence directory.

## Hosted exact-candidate checks

The Windows checkout initially converted LF fixtures to CRLF. Portability fix
`2ab7672` pins the fixture directory to LF without changing any fixture or
runtime source blob. The replacement M-1 run produced these script-runner
results:

| Target | Result | Evidence | Runtime limitation |
|---|---|---|---|
| Windows desktop | **PASS** | [compile/test job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674968473/job/88160626055) | No packaged WebView2 launch |
| macOS desktop | **PASS** | [compile/test job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674968473/job/88160626132) | Hosted checks do not replace the packaged-host run below |
| Linux desktop | **PASS** | [compile/test job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674968473/job/88160626121) | No packaged WebKitGTK launch |
| Android ARM64 | **PASS (compile only)** | [APK job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674968473/job/88160626025) | No install or WebView launch in CI |
| iOS ARM64 simulator | **PASS (compile only)** | [simulator-app job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674968473/job/88160626033) | No launch, signing, or physical device in CI |

The complete replacement M-1 matrix finished with all 35 jobs successful.
The exact-implementation
[product scaffold run](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674867356)
also passed all six product jobs, preserving the product execution lockout.

## macOS packaged Tauri WKWebView

Runtime identity:

- macOS 26.5.2 build 25F84
- arm64 physical host
- packaged debug Tauri `.app`; this is a WKWebView runtime, not browser preview

Reproduction build and launch commands, run from `spikes/script-runner`:

```sh
npm run tauri build -- --debug --bundles app --ci
open -na "/Users/codexer/LorePia_dev/spikes/script-runner/src-tauri/target/debug/bundle/macos/LorePia Script Runner Spike.app"
```

Built application path:

```text
/Users/codexer/LorePia_dev/spikes/script-runner/src-tauri/target/debug/bundle/macos/LorePia Script Runner Spike.app
```

Scenario: launch the packaged app, press `전체 경계 실증 실행` once, wait for
the sequential suite to finish, and inspect the final status and all case rows.

Actual: **PASS (`15/15`)**. The packaged Tauri WKWebView reported every fixed
code as expected, including the trusted raw Worker busy loop as
`HOST_TERMINATED` only after its exact invocation-bound `WEDGE_STARTED`
acknowledgement, a nonzero host heartbeat, and the subsequent allowed recovery
case.

The tested executable was `Contents/MacOS/lorepia-script-runner-spike`; its
size and hash are pinned in the artifact table above.

## Android ARM64 emulator WebView

Runtime identity:

- ADB serial: `emulator-5554`
- device/model: `sdk_gphone64_arm64`
- ABI: `arm64-v8a`
- Android 16 / API 36
- build fingerprint:
  `google/sdk_gphone64_arm64/emu64a:16/BE2A.250530.026.F3/13894323:userdebug/dev-keys`
- WebView: `com.google.android.webview 133.0.6943.137`
- installed package: `dev.lorepia.spike.scriptrunner`, version `0.1.0`,
  version code `1000`, target SDK 36

With the Java, SDK, and NDK paths from the toolchain inventory exported,
reproduction build, install, and launch commands from `spikes/script-runner`
are:

```sh
npm run tauri android build -- --debug --target aarch64 --apk --ci
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb shell am start -W -n dev.lorepia.spike.scriptrunner/.MainActivity
```

Built APK path:

```text
/Users/codexer/LorePia_dev/spikes/script-runner/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

The installed package resolved to its normal per-install `base.apk` path under
`/data/app`; that randomized device path is not a stable artifact identity.

Scenario: launch `MainActivity`, press `전체 경계 실증 실행` once, wait for the
sequential suite to finish, and inspect the Android UI result and case rows.

Actual: **PASS (`15/15`)**. The Android emulator WebView displayed all 15 fixed
cases as passing, including external host termination of the deliberately
wedged Worker after its exact `WEDGE_STARTED` acknowledgement and a successful
fresh-Worker recovery afterwards.

## iOS ARM64 simulator WKWebView

Runtime identity:

- simulator UDID: `675F06B1-229D-4C31-8995-4E135FAA6630`
- model: iPhone 17 simulator
- iOS: 26.5
- no-sign ARM64 simulator app; this is not a signed physical-device build

The successful rerun started from a clean generated build destination. Exact
build, install, and launch commands, run from `spikes/script-runner`:

```sh
npm run tauri ios build -- --debug --target aarch64-sim --ci --no-sign
xcrun simctl install 675F06B1-229D-4C31-8995-4E135FAA6630 \
  "/Users/codexer/LorePia_dev/spikes/script-runner/src-tauri/gen/apple/build/arm64-sim/LorePia Script Runner Spike.app"
xcrun simctl launch --terminate-running-process \
  675F06B1-229D-4C31-8995-4E135FAA6630 \
  dev.lorepia.spike.scriptrunner
```

Built simulator application path:

```text
/Users/codexer/LorePia_dev/spikes/script-runner/src-tauri/gen/apple/build/arm64-sim/LorePia Script Runner Spike.app
```

The first pre-fix simulator runtime completed **`14/15 FAIL`**:
`allowed-baseline` returned `ENGINE_INTERRUPTED` because first-use QuickJS/WASM
work consumed the guest's 50 ms execution deadline. That failure was not
reclassified as acceptable. The fix adds one trusted constant-expression
QuickJS warmup under a separate 500 ms deadline before the Worker emits
`READY`; it leaves the untrusted guest deadline at 50 ms. After rebuilding from
the clean generated destination and reinstalling, the identical UI scenario
completed **`15/15 PASS`**. The watchdog result was accepted only after the
current Worker returned its exact invocation-bound `WEDGE_STARTED`
acknowledgement; final fresh-Worker recovery also passed.

A stale ignored `gen/apple/build` destination had previously caused a repeat
build to fail with `Directory not empty (os error 66)`. The stale generated
tree was preserved outside the repository before the clean rebuild. This is a
generated-build hygiene observation, not a script-runner security pass or
failure.

## Evidence limitations and next record

- No full raw macOS/iOS receipt, process-scoped log, build transcript, or
  screenshot is committed. The Android UI hierarchy and local artifact hash
  manifest were inspected during the exact-commit rerun.
- The first exact-implementation
  [M-1 run](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674866736)
  exposed a Windows checkout failure: Git converted the LF-pinned fixture to
  CRLF, changing `allowed.js` from 84 to 88 bytes. Commit `2ab7672` fixes only
  checkout portability by enforcing LF for the hash-pinned fixture directory;
  it does not change fixture or runtime source bytes. All five script-runner
  jobs passed in the replacement
  [M-1 run](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674968473),
  whose complete matrix passed all 35 jobs,
  and the exact-implementation
  [product scaffold run](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29674867356)
  passed all six jobs.
- The Android result is emulator evidence, not a physical-device result.
- No Windows or Linux runtime is recorded.
- The iOS pass is simulator runtime evidence only. Signing, entitlements,
  physical-device Worker/WASM behavior, lifecycle, thermals, and store policy
  remain untested.
- None of the three runs exercised arbitrary imported user source or a product
  API. They ran only the self-authored, hash-pinned diagnostic corpus.
- Product imported JavaScript remains disabled pending extraction and review.

Future promotion evidence must retain complete bounded receipts and raw run
artifacts, add physical-device and Windows/Linux runtime results, and link the
signed tester or CI identity. Until those gates pass, matrix rows must describe
these results as exact-commit local candidate observations only.
