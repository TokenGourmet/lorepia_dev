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

| Target | Evidence level | Current state | Evidence |
|---|---|---|---|
| Windows | Hosted native compile/test | NOT RUN | Await first `M-1 verification / Desktop` run |
| macOS | Hosted native compile/test | NOT RUN | Await first `M-1 verification / Desktop` run |
| Linux | Hosted native compile/test | NOT RUN | Await first `M-1 verification / Desktop` run |
| Android ARM64 | Hosted cross-compile to debug APK | NOT RUN | Await first `Android compile (no device)` run |
| iOS ARM64 | Hosted simulator compile | NOT RUN | Await first `iOS simulator compile (no device)` run |

## Isolation baseline observations (not physical-device evidence)

These rows preserve the behavior of unsafe-baseline commit `2f8e130`. They do
not replace or change any physical-device matrix cell above.

| Platform/runtime | Evidence level | Result | Evidence and limitation |
|---|---|---|---|
| Locally packaged macOS release `.app` | Local packaged-app effect test | PASS (`11/11`, probe `1 -> 1`) | [`isolation.md`](isolation.md); direct invoke transport was not exposed, so this is not ACL-denial proof |
| Android 16 / API 36 `sdk_gphone64_arm64`, WebView `133.0.6943.137` | Local emulator runtime | FAIL (probe `1 -> 2`) | [`isolation.md`](isolation.md); sandboxed iframe reached `privileged_probe`; screenshot hash retained but raw `/tmp` file is not in the repository |

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
| iframe IPC and broker denial | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | Unsafe Android emulator baseline preserved; candidate removes the retired privileged wrappers and uses a host-only 256-bit-token Rust broker, but the combined spike still has four raw Channel transport commands ([record](isolation.md)) |
| Final HTML sanitizer | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Plugin network default-deny | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | None |
| Store-Safe imported-code lockout | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | Imported JS and Lua remain disabled |

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
