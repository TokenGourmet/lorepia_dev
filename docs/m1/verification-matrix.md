# M-1 verification matrix

Last updated: 2026-07-19

This is an evidence index, not a roadmap checkbox list. Every result names the
exact subject commit and run; a later documentation-only commit does not
silently inherit unrecorded runtime evidence. `NOT RUN` is intentional until a
qualifying record described in [`README.md`](README.md) is attached. Never
delete failure evidence when a fallback is chosen. The preserved unsafe
isolation baseline is documented separately in [`isolation.md`](isolation.md).

## Platform-by-capability runtime matrix

| Platform | SQLite / FTS5 | Lua limits | File import | Keychain | Channel stream | Audio |
|---|---|---|---|---|---|---|
| Windows physical machine | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| macOS physical machine | PASS ([host record](evidence/sqlite-macos-39dfef0/)) | PASS ([host record](evidence/lua-macos-9975d80/)) | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| Linux physical machine | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| Android physical device | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| iOS physical device | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |

For each replacement use a link label that makes the level visible, for example `PASS ([device log](evidence/...))` or `FAIL ([device log](evidence/...))`.

## Compile and simulated evidence

These rows are deliberately separate from the runtime matrix.

| Target / vertical | Evidence level | Current state | Evidence |
|---|---|---|---|
| Windows / Channel | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282418); no packaged WebView runtime |
| Windows / keychain | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282405); no credential-service runtime |
| Windows / SQLite/FTS5 | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282427); no file-locking/search runtime |
| Windows / import hardening | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282400); no system picker/runtime import |
| Windows / Lua limits | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282423); fixed diagnostic corpus only |
| Windows / Audio playback | Hosted native compile/test and emitted-fixture identity check | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282406); no audio device or lifecycle exercised |
| macOS / Channel | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282403); hosted checks only |
| macOS / keychain | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282401); no Keychain runtime |
| macOS / SQLite/FTS5 | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282425); separate physical-host record remains below |
| macOS / import hardening | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282433); no system picker/runtime import |
| macOS / Lua limits | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282428); fixed diagnostic corpus only |
| macOS / Audio playback | Hosted native compile/test and emitted-fixture identity check | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282417); no audio device or lifecycle exercised |
| Linux / Channel | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282389); no packaged WebView runtime |
| Linux / keychain | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282424); no Secret Service runtime |
| Linux / SQLite/FTS5 | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282338); no file-locking/search runtime |
| Linux / import hardening | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282415); no system picker/runtime import |
| Linux / Lua limits | Hosted native compile/test | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282351); fixed diagnostic corpus only |
| Linux / Audio playback | Hosted native compile/test and emitted-fixture identity check | PASS | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282361); no audio device or lifecycle exercised |
| Android ARM64 / Channel | Hosted cross-compile to debug APK | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282337); APK existence/hash checked, no install/runtime |
| Android ARM64 / keychain | Hosted cross-compile to debug APK | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282318); APK existence/hash checked, no Keystore runtime |
| Android ARM64 / SQLite/FTS5 | Hosted cross-compile to debug APK | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282326); APK existence/hash checked, no database runtime |
| Android ARM64 / import hardening | Hosted cross-compile to debug APK | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282328); APK existence/hash checked, no picker/import runtime |
| Android ARM64 / Lua limits | Hosted cross-compile to debug APK | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282324); APK existence/hash checked, no Lua runtime |
| Android ARM64 / Audio playback | Hosted cross-compile to debug APK and emitted-fixture identity check | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282344); install, output, lifecycle, and release remain `NOT RUN` |
| iOS ARM64 / Channel | Hosted simulator compile | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282305); `.app`/Info.plist checked, no launch |
| iOS ARM64 / keychain | Hosted simulator compile | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282316); `.app`/Info.plist checked, no signed Keychain runtime |
| iOS ARM64 / SQLite/FTS5 | Hosted simulator compile | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282327); `.app`/Info.plist checked, no database runtime |
| iOS ARM64 / import hardening | Hosted simulator compile | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282342); `.app`/Info.plist checked, no picker/import runtime |
| iOS ARM64 / Lua limits | Hosted simulator compile | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282304); `.app`/Info.plist and vendored symbols checked, no Lua runtime |
| iOS ARM64 / Audio playback | Hosted simulator compile and emitted-fixture identity check | PASS (compile only) | [`d56388e` job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249/job/88151282311); launch, output, lifecycle, and release remain `NOT RUN` |
| macOS arm64 / SQLite/FTS5 exact candidate | Local packaged debug `.app` build and physical-host runtime | PASS | [source `39dfef0` record](evidence/sqlite-macos-39dfef0/) |
| macOS arm64 / import-hardening exact candidate | Local packaged debug `.app` build and physical-host synthetic-core runtime | PASS twice (`26/26` each) | [source `46af753` record](evidence/import-hardening-macos-46af753/); no picker or external file was used, so File import and Archive/import hardening cells remain `NOT RUN` |
| Android ARM64 / import-hardening exact candidate | Local cross-compile to debug APK, no device | PASS (compile only) | [source `46af753` record](evidence/import-hardening-mobile-compile-46af753/); APK metadata and debug signature were inspected, but install/runtime/import remain `NOT RUN` |
| iOS ARM64 / import-hardening exact candidate | Local simulator compile with `--no-sign`, no booted simulator | PASS (compile only) | [source `46af753` record](evidence/import-hardening-mobile-compile-46af753/); simulator-app metadata was inspected, but install/runtime/import remain `NOT RUN` |
| macOS arm64 / Lua-limits exact candidate | Local packaged debug `.app` build and physical-host runtime | PASS twice (`11/11` each) | [source `9975d80` record](evidence/lua-macos-9975d80/); fixed self-authored corpus only, with no imported Lua or product scripting API |
| Android ARM64 / Lua-limits exact candidate | Local cross-compile to debug APK, no device | PASS (compile only) | [source `9975d80` record](evidence/lua-mobile-compile-9975d80/); APK metadata and debug signature were inspected, but install/runtime/Lua execution remain `NOT RUN` |
| iOS ARM64 / Lua-limits exact candidate | Local simulator compile with `--no-sign`, no booted simulator | PASS (compile only) | [source `9975d80` record](evidence/lua-mobile-compile-9975d80/); vendored Lua final-link symbols and app metadata were inspected, but install/runtime/Lua execution remain `NOT RUN` |
| Android ARM64 / SQLite/FTS5 exact candidate | Local cross-compile summary only | NOT RUN | Build exited 0, but no qualifying raw log/artifact or run identity was retained ([observation](evidence/sqlite-mobile-compile-39dfef0/)) |
| iOS ARM64 / SQLite/FTS5 exact candidate | Local simulator-compile summary only | NOT RUN | Build exited 0, but no qualifying raw log/artifact or run identity was retained ([observation](evidence/sqlite-mobile-compile-39dfef0/)) |

### Hosted CI repair history

- The first complete run at `e4c1c65` preserved five Windows failures in
  [run `29669864571`](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29669864571):
  Channel CSP/Cargo-lock checks and the import/Lua/SQLite fixture checks saw
  checkout CRLF bytes, while Keychain failed Windows-only Clippy. Windows audio
  passed. The fix pins byte-identity inputs to LF and removes the needless
  Windows `return`; expected hashes were not rewritten to accept changed bytes.
- A later exact Windows Lua job preserved a second CRLF assumption in a Rust
  manifest/lockfile string assertion
  ([failed job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671131767/job/88150205416)).
  Semantic config checks now normalize CRLF before matching.
- The first 1 MiB Channel rerun preserved a hosted-Windows scheduling timeout
  ([failed job](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671321654/job/88150759252)).
  The integrity test still proves exact bytes, sequence, terminal state, and
  sub-4096-byte events, but uses a bounded 30-second CI-tolerant deadline; it is
  not treated as a five-second performance gate.
- Final implementation subject `d56388e` passed all
  [30 M-1 jobs](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249).
  These repairs do not alter the physical-runtime matrix above.

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
| macOS host | macOS 26.5.2, arm64 | Packaged SQLite/FTS5 host record attached; other capability cells remain separate |
| Apple toolchain | Xcode 26.6 | Toolchain inventory only |
| iOS simulator | iOS 26.5 | Simulator, not a physical iOS device |
| Android emulator | Android 16 / API 36, `sdk_gphone64_arm64`, WebView 133.0.6943.137 | Emulator, not a physical Android device |

## Negative-test matrix

| Test group | Windows | macOS | Linux | Android device | iOS device | Selected fallback / decision |
|---|---|---|---|---|---|---|
| Archive/import hardening | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Regex budget | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Lua budget and stdlib removal | NOT RUN | PASS ([host record](evidence/lua-macos-9975d80/)) | NOT RUN | NOT RUN | NOT RUN | Fixed pure-Lua corpus proved instruction/allocator interruption, forbidden-global absence, and host recovery; this is VM non-exposure, not binary object removal, and imported Lua remains disabled |
| JavaScript watchdog | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | Cooperative same-event-loop watchdog cannot handle a busy loop; imported JS execution remains blocked ([baseline](isolation.md)) |
| iframe IPC and broker denial | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | Unsafe Android emulator baseline preserved; broker candidate retires demonstrated privileged wrappers. Same-process Tauri plugin WebView rejected after shared Channel queue audit; current eight-command spike uses a version-pinned 4096-byte transport mitigation, while imported execution stays off ([isolation](isolation.md), [Channel decision](channel-ipc-boundary.md)) |
| Final HTML sanitizer | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Plugin network default-deny | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Store-Safe imported-code lockout | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | Android/iOS source and simulated-build checks omit executable fixture assets, replace the isolation route, scan output markers, and reject unknown targets (`07ff9c9`, `3f511f2`); packaged/physical-device evidence remains required ([decision](channel-ipc-boundary.md)) |

## Compatibility, performance, and policy decisions

| Gate | State | Required evidence / current decision |
|---|---|---|
| Risu observation notes v1 | NOT RUN | Observation record without copied source |
| Compatibility fixture provenance | NOT RUN | Risu/card/import fixture set and conversion-difference record are not complete |
| SQLite FTS fixture provenance | PASS | One synthetic search fixture has self-authored origin, CC0-1.0 permission, byte size, and canonical hash pinned ([fixture record](../../spikes/sqlite-fts/fixtures/README.md)) |
| Import-hardening fixture provenance | PASS | The 26-case catalog is self-authored, CC0-1.0, and hash-pinned; attack bytes are generated deterministically by the disposable probe ([fixture record](../../spikes/import-hardening/fixtures/README.md)) |
| Compatibility golden behavior and conversion | NOT RUN | Risu/card/import golden set and conversion differences are not complete |
| SQLite FTS golden searches | PASS (`7/7`) | Exact expected/actual IDs are in the native transcript; a packaged-app pass screenshot is separate and raw IPC JSON was not retained ([host record](evidence/sqlite-macos-39dfef0/)) |
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
