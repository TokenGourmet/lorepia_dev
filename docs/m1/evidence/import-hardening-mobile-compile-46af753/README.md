# Import-hardening mobile compile-only evidence — `46af753`

## Scope and result

- Android ARM64 debug APK: `PASS` for local cross-compilation only
- iOS ARM64 simulator app: `PASS` for local compilation only
- Source commit: `46af7539dfc564ba2fc4c8009401878965799fc3`
- Build isolation: detached temporary worktrees at the exact source commit
- Android UTC window: 2026-07-18T22:22:44Z through
  2026-07-18T22:23:56Z
- iOS UTC window: 2026-07-18T22:21:53Z through
  2026-07-18T22:23:11Z
- Build host: Mac mini `Mac16,10`, Apple M4, 16 GB memory; macOS 26.5.2
  build 25F84
- Tester: OpenAI Codex task `/root`; the Android build was delegated to
  `/root/import_android_compile`

Both target builds exited 0 and produced artifacts whose architecture,
metadata, and hashes were inspected before temporary-worktree cleanup. The
retained normalized build sessions include the source identity, environment,
commands, warnings, artifact path, and final exit state.

Expected: each documented mobile command exits 0 and emits an artifact for the
requested target architecture. Actual: Android emitted an ARM64-only debug APK,
and iOS emitted an arm64 simulator `.app`; both commands exited 0.

This record does **not** prove installation, launch, WebView IPC, file-picker or
URI behavior, external-file handling, import results, cleanup, resource limits,
OS-store policy, release signing, or physical-device behavior. No Android
device/emulator was used. No iOS simulator was booted, and no physical iOS
device was used. Therefore the physical File import and Archive/import
hardening matrix cells remain `NOT RUN`.

## Locked inputs

- `package-lock.json`:
  `becfe28991b1977f9a825df51761ff028b84c4a80511b9d3abcc8d6ee27ada99`
- `Cargo.lock`:
  `a55919478e93b45740329e9f64432ddf146b51d151fae9915b526bdf3cc9fc19`
- `import-cases-v1.json` (4,236 bytes):
  `484a313423d4e91c792818fb64097d96f8efb7c4a31befe96a1d3f739bfe5eb2`

## Android ARM64 debug APK

Commands run from `spikes/import-hardening`:

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
- Android SDK/compile SDK 36, Build Tools 36.1.0
- Android NDK 28.2.13676358 (`r28c`)

The retained [normalized build session](android-build-session.log) is 23,689
bytes with SHA-256
`8a7b3b9804894432cab926ca011b9282e3207aa6022bd8ce2356a55b680e20cc`.
ANSI terminal control codes were removed and the local hostname was replaced
with `[REDACTED_HOSTNAME]` after capture, without changing command output
ordering. The [artifact inspection](android-apk-inspection.log) is 9,825
bytes with SHA-256
`e3315a75519578e4482c36a017617abc59ff0a099f83adead4ffd2cda017ad15`.

Evaluated APK metadata:

- APK size: 142,155,091 bytes
- APK SHA-256:
  `b90586ab22d106db12c2f99c9ec03b5ea95b939da25ae401211e59462ca31d97`
- package/version: `dev.lorepia.spike.importhardening`, `0.1.0` / `1000`
- compile/target/min SDK: `36` / `36` / `24`
- native ABI: `arm64-v8a` only
- native entry: `lib/arm64-v8a/libspikesimporthardening_lib.so`
- native library size: 134,989,032 bytes
- native library SHA-256:
  `1a8915686a2cc27f025b9b25a81e69b67b3d149cfc4cf6e7bfabce31ee516199`
- native format: ELF64 AArch64 shared object, debug information present

APK Signature Scheme v2 verification passed with one Android Debug signer.
That is a development signature and makes no distribution-signing claim. The
first inspection invocation omitted `JAVA_HOME` and emitted no verifier result;
the retained retry used OpenJDK 21 and passed. The build emitted non-blocking
warnings for generated deprecated Android APIs, Java 8 source/target support,
and Gradle features that will be incompatible with Gradle 9.

## iOS ARM64 simulator app

Commands run from `spikes/import-hardening`:

```sh
npm ci
npm run tauri ios init -- --ci
npm run tauri ios build -- --debug --target aarch64-sim --ci --no-sign
```

Environment:

- macOS 26.5.2 build 25F84, arm64 host
- Node.js 22.23.0, npm 10.9.8
- Rust/Cargo 1.97.1, target `aarch64-apple-ios-sim`
- Tauri CLI 2.11.4
- Xcode 26.6 build 17F113
- CocoaPods 1.16.2
- installed simulator runtime: iOS 26.5

The retained [normalized build session](ios-build-session.log) is 7,718 bytes
with SHA-256
`bd325c17e60a032f08f00c33854b65f73bbf89fcfed22770ca32cd57ff45bbdd`.
CR/ANSI terminal control codes were removed and local destination identifiers
were replaced with `[REDACTED_DEVICE_ID]` after capture, without changing
command output ordering. The concise [artifact inspection](ios-app-inspection.log)
is 1,857 bytes with SHA-256
`f7cd02e192a8d14bf0c972a46c832faaf14a71e31b96cef3bdf3d01e06ba63d1`
and records the post-build metadata inspected at 2026-07-18T22:26:55Z.

Evaluated simulator-app metadata:

- app bundle size: 88,492 KiB; 9 regular files
- executable size: 90,312,784 bytes
- executable SHA-256:
  `404db0e5aae05e99fe2000deb04cfab800e8b45e3be0f22917bee3c28cdbbda1`
- executable format: Mach-O 64-bit arm64
- `Info.plist` size: 1,334 bytes
- `Info.plist` SHA-256:
  `fc4ce1e88ea90e87efe08773741162d7f18de42113272d3b448fb2f63c77c9a8`
- bundle identifier: `dev.lorepia.spike.importhardening`
- platform/SDK/minimum OS: `iPhoneSimulator` / `iphonesimulator26.5` / `14.0`
- sorted regular-file manifest SHA-256:
  `3400d3b94104f35940ef8d6d0d65457bf368830dd6453f45026c359996ab90a3`

The executable had a linker-generated ad-hoc signature with CDHash
`8fe1aeed15a8ec7fb9e88c71b3b293f1119b6dd5`. Strict deep verification
returned exit 1 because bundle resources were unsealed. No development team or
distribution signature was used, and this record makes no signing-pass claim.

## Artifact retention and warnings

The 142 MB APK and 88,492 KiB (about 86.4 MiB) simulator `.app` are not
committed or linked from an external artifact store. This record retains their
complete normalized build sessions, inspection output, hashes, and tester
identity. The detached worktrees and generated artifacts are retained only
through this evidence commit and are then removed.

Both `npm ci` invocations reported three low-severity development advisories.
The source commit separately passed the repository's moderate-threshold audit
and runtime-only audit; those source checks are not repeated as mobile runtime
claims here.
