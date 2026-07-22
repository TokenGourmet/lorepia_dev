# Lua-limit mobile compile-only evidence — `9975d80`

## Scope and result

- Android ARM64 debug APK: `PASS` for local cross-compilation only
- iOS ARM64 simulator app: `PASS` for local compilation only
- Source commit: `9975d80521c61d5c25bc9509c609439cfa371d58`
- Build isolation: clean detached temporary worktrees at the exact source commit
- Android evidence window: 2026-07-18T23:01:57Z through
  2026-07-18T23:05:17Z
- iOS evidence window: 2026-07-18T23:01:58Z through
  2026-07-18T23:06:55Z
- Build host: Mac mini `Mac16,10`, Apple M4, 16 GB memory; macOS 26.5.2
  build 25F84
- Tester: OpenAI Codex task `/root`; the target builds were delegated to
  `/root/import_repo_pattern_scout` and `/root/sqlite_docs_ci`

All six dependency-install, wrapper-generation, and target-build commands
exited 0. The retained inspections identify an ARM64-only debug APK and an
arm64 iOS Simulator `.app`. Source lock hashes were unchanged, and the exact
source worktrees had no tracked changes after either build.

The Android command logs include explicit `EXIT_CODE=0` wrappers. The iOS logs
were retained without equivalent exit-code wrappers; their normal completion
output, emitted app, final-link inspection, manifest verification, and clean
tracked state corroborate the orchestrator-observed zero exits.

This record does **not** prove installation, launch, WebView IPC, Lua execution,
limit enforcement, lifecycle behavior, physical-device behavior, release
signing, Store-Safe approval, or App Store / Play Store readiness. No Android
device or emulator was used. No iOS simulator was booted, and no physical iOS
device was used. The physical Android and iOS Lua cells therefore remain
`NOT RUN`.

## Locked inputs

- `package-lock.json`:
  `049d114c3b4c672e38bb24a897bdd2702371b7ad7bebc7ae3f106465a73d93b5`
- `Cargo.lock`:
  `0f99c5dc2d5faad92b9926fe67b0c53a1d7a821059f91bac67942d2d746a3307`
- `fixtures/catalog.json` (1,376 bytes):
  `9ea567d6901ec39412e73f439ee9ea7d47538baea4d1a92cd409c9f3e9b97db5`

## Android ARM64 debug APK

Commands run from `spikes/lua-limits`:

```sh
npm ci
npm run tauri android init -- --ci
npm run tauri android build -- --debug --target aarch64 --apk --ci
```

Environment:

- macOS 26.5.2 build 25F84, arm64 host
- Node.js 22.23.0, npm 10.9.8
- Rust/Cargo 1.97.1, target `aarch64-linux-android`
- Tauri CLI 2.11.4
- OpenJDK 21.0.11
- Gradle 8.14.3
- Android compile/target SDK 36
- Android NDK 28.2.13676358 (`r28c`)

The [baseline](android-baseline.log) is 1,375 bytes with SHA-256
`38c1acd15a7b3e270631df1f8c3baee1ff1dc587e8dd4a1ca2244cfb804b4637`.
The retained [build session](android-build-session.log) is 19,650 bytes with
SHA-256
`f1e982c99d7bd8d43dbd15a6971d08b92a987bbb26ca9981052dca70525c6b73`.
The [artifact inspection](android-apk-inspection.log) is 13,599 bytes with
SHA-256
`4fd32d356eef171de0c9d74e80d6cc54eed6d7395e70ef35245cbdea499dc0c3`.
Terminal CR, ANSI escape, and backspace control bytes were removed from the
retained logs, and trailing whitespace was trimmed, without changing command
output ordering.

Evaluated APK metadata:

- APK size: 137,673,749 bytes
- APK SHA-256:
  `66788efe85680191b6ef43a4ca4ed96434b6316203c5c7530a4327001c5ee790`
- package/version: `dev.lorepia.spike.lualimits`, `0.1.0` / `1000`
- compile/target/min SDK: `36` / `36` / `24`
- native ABI: `arm64-v8a` only
- native entry: `lib/arm64-v8a/libspikeslualimits_lib.so`
- native library size: 130,507,720 bytes
- native library SHA-256:
  `b5b3037b49ca6161d75615f4462d085360068d1e3de06d6ff9517a10ff8cf640`
- native format: ELF64 AArch64 shared object, debug information present
- dynamic dependencies: Android, `dl`, `log`, `m`, and `c` system libraries

ZIP integrity and four-byte alignment checks passed. APK Signature Scheme v2
verification passed with one Android Debug signer whose certificate SHA-256 is
`81cb32ca842b661d9010756e04ab9a3e4431f2b368bf1fd7cf6668ad8c0ff2e2`.
That is a development signature and makes no distribution-signing claim. The
build retained non-blocking Java 8 source/target, generated API deprecation,
and Gradle 9 compatibility warnings.

## iOS ARM64 simulator app

Commands run from `spikes/lua-limits`:

```sh
npm ci
npm run tauri ios init -- --ci
npm run tauri ios build -- --debug --target aarch64-sim --ci --no-sign
```

The build itself ran from 2026-07-18T23:02:49Z through
2026-07-18T23:04:37Z.

Environment:

- macOS 26.5.2 build 25F84, arm64 Mac mini `Mac16,10`, Apple M4, 16 GB
- Node.js 22.23.0, npm 10.9.8
- Rust/Cargo 1.97.1, target `aarch64-apple-ios-sim`
- Tauri CLI 2.11.4
- Xcode 26.6 build 17F113, CocoaPods 1.16.2
- installed simulator runtime: iOS 26.5 build 23F77

The retained [build session](ios-build-session.log) is 5,699 bytes with
SHA-256
`742ff74c31cfc218eff40db6326e4e558e421a3b15dd05a000a7b77ef784ee6f`.
CR/ANSI controls and trailing whitespace were removed, the worktree path was represented as
`[WORKTREE]`, and local simulator identifiers were represented as
`[REDACTED_DEVICE_ID]`, without changing command output ordering.

Evaluated simulator-app metadata:

- app contents: 9 regular files, 88,843,674 total file bytes
- executable size: 88,566,312 bytes
- executable SHA-256:
  `03ef7395b7a3e9fea5b48e860536aaf2056043c92dbb78c015424aae06ae319e`
- executable format: thin Mach-O 64-bit arm64
- platform/SDK/minimum OS: `IOSSIMULATOR` / `26.5` / `14.0`
- Mach-O header includes `NOUNDEFS`
- bundle identifier/version: `dev.lorepia.spike.lualimits`, `0.1.0`
- `Info.plist` SHA-256:
  `b9b4e687166b3ac9beae9953fe0dfc3b56c519492707f0672b9a17fd3c442d1c`
- sorted nine-file app manifest SHA-256:
  `832435a92e232d08592a9f41d9676ac50b9d75912a71bc3ccc324f5d8ae9ed26`

The [Lua link inspection](ios-lua-link-inspection.log) records 342 defined Lua
C symbol lines and zero unresolved Lua symbol lines. The
[artifact inspection](ios-app-artifact-inspection.log) contains no external Lua
dynamic library. Generated `libapp.a` contains 32 vendored Lua C object members;
it is 347,550,280 bytes with SHA-256
`8901695371088704e8d3413adda81834151bc4a3c6071fe4ab2709d527fdabab`.
This proves that the vendored Lua implementation reached this exact simulator
executable. It does not prove that every compiled Lua standard-library opener
is exposed: the probe VM initializes only its documented allowlist.

The executable had a linker-generated ad-hoc signature with CDHash
`e795f65d05484b8a84ce8caa56727ec437c255be`. It had no development team,
resource seal, or entitlements. Strict verification exited 1 because resources
were unsealed, so this `--no-sign` record makes no distribution-signing claim.

## Parent failure and fixed link boundary

The parent implementation commit `8ac5f4d` compiled Rust but failed the final
iOS link with unresolved `_lua*` symbols. The bounded
[parent failure capture](ios-parent-link-failure-8ac5f4d.log) records that its
`libapp.a` had zero Lua C object members while the separately built vendored
`liblua5.4.a` had 32, and that upstream metadata requested
`static:-bundle=lua5.4`. The generated Xcode project linked only `libapp.a`.

The partial parent archive itself is intentionally not retained here. After
the text capture was made, a later temporary-worktree experiment collided with
the hard-linked archive copy, so its later bytes could no longer be treated as
immutable evidence. The 2,886-byte pre-collision text capture, SHA-256
`67705a04df39f8b39bcd19bea39603aaa0c281c9a78a932a04e881f8d55e83e3`,
is used only to document the original failure mechanism. It is not treated as
a successful artifact or runtime result.

Commit `9975d80` adds an iOS-only bundling bridge that copies the already-built
vendored archive under a private name and emits Rust's `+bundle` modifier. The
successful final-link measurements above are the qualifying evidence for that
fix. Desktop and Android dependency selection are unchanged.

## Retention, normalization, and limitations

The 137.7 MB APK and 88.8 MB simulator executable are not committed or linked
from an external artifact store. This record retains normalized command logs,
artifact metadata, source and artifact hashes, and manifest verification. The
[log validation](normalized-log-validation.txt) records zero CR, ANSI escape,
and backspace bytes for every retained `.log`. The
[evidence manifest](evidence-sha256.txt) can be verified from this directory
with `shasum -a 256 -c evidence-sha256.txt`.

Both `npm ci` runs reported three low-severity development advisories. The
source commit separately passed its moderate-threshold audit and production
dependency audit. Neither mobile target was installed or run, so Lua policy,
memory, deadline, recovery, IPC, and imported-code behavior all remain outside
this record.
