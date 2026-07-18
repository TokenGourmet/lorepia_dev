# M-1 verification matrix

Last updated: 2026-07-19

This is the evidence index for the current commit, not a roadmap checkbox list. `NOT RUN` is intentional until a qualifying record described in [`README.md`](README.md) is attached. Replace a state with `PASS`, `FAIL`, or `BLOCKED` plus a direct evidence link; never delete failure evidence when a fallback is chosen. The preserved unsafe isolation baseline is documented separately in [`isolation.md`](isolation.md).

## Platform-by-capability runtime matrix

| Platform | SQLite / FTS5 | Lua limits | File import | Keychain | Channel stream | Audio |
|---|---|---|---|---|---|---|
| Windows physical machine | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| macOS physical machine | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| Linux physical machine | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| Android physical device | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| iOS physical device | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |

For each replacement use a link label that makes the level visible, for example `PASS ([device log](evidence/...))` or `FAIL ([device log](evidence/...))`.

## Compile and simulated evidence

These rows are deliberately separate from the runtime matrix.

| Target / vertical | Evidence level | Current state | Evidence |
|---|---|---|---|
| Windows / Channel | Hosted native compile/test | NOT RUN | Await first `Desktop compile/test (Windows / channel-stream)` run |
| Windows / keychain | Hosted native compile/test | NOT RUN | Await first `Desktop compile/test (Windows / keychain)` run |
| macOS / Channel | Hosted native compile/test | NOT RUN | Await first `Desktop compile/test (macOS / channel-stream)` run |
| macOS / keychain | Hosted native compile/test | NOT RUN | Await first `Desktop compile/test (macOS / keychain)` run |
| Linux / Channel | Hosted native compile/test | NOT RUN | Await first `Desktop compile/test (Linux / channel-stream)` run |
| Linux / keychain | Hosted native compile/test | NOT RUN | Await first `Desktop compile/test (Linux / keychain)` run |
| Android ARM64 / Channel | Hosted cross-compile to debug APK | NOT RUN | Await first `Android compile (channel-stream, no device)` run |
| Android ARM64 / keychain | Hosted cross-compile to debug APK | NOT RUN | Await first `Android compile (keychain, no device)` run |
| iOS ARM64 / Channel | Hosted simulator compile | NOT RUN | Await first `iOS simulator compile (channel-stream, no device)` run |
| iOS ARM64 / keychain | Hosted simulator compile | NOT RUN | Await first `iOS simulator compile (keychain, no device)` run |

## Isolation baseline observations (not physical-device evidence)

These rows preserve the behavior of unsafe-baseline commit `2f8e130`. They do
not replace or change any physical-device matrix cell above.

| Platform/runtime | Evidence level | Result | Evidence and limitation |
|---|---|---|---|
| Locally packaged macOS release `.app` | Local packaged-app effect test | PASS (`11/11`, probe `1 -> 1`) | [`isolation.md`](isolation.md); direct invoke transport was not exposed, so this is not ACL-denial proof |
| Android 16 / API 36 `sdk_gphone64_arm64`, WebView `133.0.6943.137` | Local emulator runtime | FAIL (probe `1 -> 2`) | [`isolation.md`](isolation.md); sandboxed iframe reached `privileged_probe`; screenshot hash retained but raw `/tmp` file is not in the repository |

## Broker candidate observations (not physical-device evidence)

These rows evaluate exact candidate commit
`a42d33d4e843da353f042d435133d3ac5f988fa4`. They do not replace or change any
physical-device or cross-platform capability cell above.

| Platform/runtime | Evidence level | Result | Evidence and limitation |
|---|---|---|---|
| macOS 26.5.2 arm64, locally packaged release `.app` | Local packaged-app effect test | PASS (`18/18`) | [candidate record](evidence/a42d33d-local-isolation.md); direct iframe Tauri transport was absent, so this is not ACL-decision proof |
| Android 16 / API 36 `sdk_gphone64_arm64`, WebView `133.0.6943.137` | Local emulator runtime | **FAIL** (`15/18`, `18/18` complete) | [candidate record](evidence/a42d33d-local-isolation.md); three direct native callbacks timed out and remain `INCONCLUSIVE`; final monitored sink delta had no extra effect |

## Keychain candidate observations (not physical-device evidence)

Each row evaluates the exact source commit named by its candidate record. These
observations do not replace or change any physical-device keychain cell above.

| Platform/runtime | Evidence level | Result | Evidence and limitation |
|---|---|---|---|
| macOS 26.5.2 build 25F84, arm64 physical host | Local packaged-app OS-store lifecycle | PASS twice (`7/7`, cleanup proved) | [candidate record](evidence/keychain-macos-host-38ce4c2/); ad-hoc bundle and login Keychain only, with no product SQLite/export/crash-path sentinel scan |
| Android 16 / API 36 `sdk_gphone64_arm64`, WebView `133.0.6943.137` | Local emulator OS-store lifecycle | PASS (`7/7`, cleanup proved) | [candidate record](evidence/keychain-android-emulator-38ce4c2/); installed APK hash matched the built APK; emulator only, with no product SQLite/export/crash-path sentinel scan |
| iPhone 17 Pro simulator, iOS 26.5 build 23F77 | Local no-sign simulator OS-store attempt | **FAIL** before lifecycle (`-34018`) | [candidate record](evidence/keychain-ios-simulator-4fde100/); linker ad-hoc app had no effective Keychain entitlements, so this proves the no-sign runtime limitation only |

## Available local environments (not pass evidence)

These environments have been identified for upcoming runs. Inventory alone does not change a matrix cell.

| Environment | Identified version | Evidence limitation |
|---|---|---|
| macOS host | macOS 26.5.2, arm64 | No capability scenario/result attached yet |
| Apple toolchain | Xcode 26.6 | Toolchain inventory only |
| iOS simulator | iOS 26.5 | Simulator, not a physical iOS device |
| Android emulator | Android 16 / API 36, `sdk_gphone64_arm64`, WebView 133.0.6943.137 | Emulator, not a physical Android device |

## Negative-test matrix

| Test group | Windows | macOS | Linux | Android device | iOS device | Selected fallback / decision |
|---|---|---|---|---|---|---|
| Archive/import hardening | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Regex budget | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Lua budget and stdlib removal | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| JavaScript watchdog | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | Cooperative same-event-loop watchdog cannot handle a busy loop; imported JS execution remains blocked ([baseline](isolation.md)) |
| iframe IPC and broker denial | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | Unsafe Android emulator baseline preserved; broker candidate retires demonstrated privileged wrappers. Same-process Tauri plugin WebView rejected after shared Channel queue audit; current eight-command spike uses a version-pinned 4096-byte transport mitigation, while imported execution stays off ([isolation](isolation.md), [Channel decision](channel-ipc-boundary.md)) |
| Final HTML sanitizer | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Plugin network default-deny | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Store-Safe imported-code lockout | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | Android/iOS source and simulated-build checks omit executable fixture assets, replace the isolation route, scan output markers, and reject unknown targets (`07ff9c9`, `3f511f2`); packaged/physical-device evidence remains required ([decision](channel-ipc-boundary.md)) |

## Compatibility, performance, and policy decisions

| Gate | State | Required evidence / current decision |
|---|---|---|
| Risu observation notes v1 | NOT RUN | Observation record without copied source |
| Fixture provenance | NOT RUN | Per-file origin, permission/license, hash |
| Golden behavior tests | NOT RUN | Expected/actual result for every fixture |
| Android reference device fixed | NOT RUN | Model, SoC/RAM, OS/build, power mode |
| Windows reference machine fixed | NOT RUN | Model, CPU/RAM, OS/build, power mode |
| Raw samples and p95 report | NOT RUN | Dataset, warm-up, sample count, raw samples, calculation |
| Plugin API freeze | BLOCKED | Blocked by observations, fixtures, golden tests, and isolation evidence |
| Android Store-Safe decision | JS/LUA OFF | Unsafe emulator isolation baseline plus absent real-device defenses; imported JS and Lua remain disabled ([record](isolation.md)) |
| iOS Store-Safe decision | JS/LUA OFF | Written policy clearance and real-device defenses do not yet exist; same-event-loop JS blocker remains open ([record](isolation.md)) |
| M-1 exit review | BLOCKED | Exit contract in `docs/m1/README.md` is not yet satisfied |

## Evidence record template

Create one immutable record per execution. Keep large raw artifacts in the CI artifact store or an approved evidence store and link them here.

```text
Gate/cell:
Result: PASS | FAIL | BLOCKED
Commit SHA:
UTC timestamp:
OS/build:
Hardware/runner image:
Device/simulator identifier:
Toolchain and dependency lock hashes:
Exact command or manual scenario:
Fixture hashes:
Expected:
Actual:
Raw log/artifact URL:
Tester or CI run URL:
Fallback/issue (if failed):
```
