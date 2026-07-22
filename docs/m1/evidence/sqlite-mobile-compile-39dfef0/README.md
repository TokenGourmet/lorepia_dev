# SQLite/FTS5 mobile compile-only evidence — `39dfef0`

## Shared boundary

- Source commit: `39dfef0ec998f806093722dbb23c46d3239fe077`
- Build isolation: separate detached temporary worktree per target
- Observed outcome: both local build commands exited 0
- Qualifying matrix state: `NOT RUN`; no raw build transcript, retained binary,
  or CI run identity was preserved
- Runtime result: `NOT RUN`

These are non-qualifying local observations about the exact SQLite/FTS5
candidate. Under the M-1 evidence contract, the summaries and hashes below do
not become matrix `PASS` without a retained raw log or artifact and a tester or
CI run identity. They also do not prove app launch, WebView IPC, SQLite file
locking, FTS5 behavior, store policy, signing, or physical-device behavior.

## Android ARM64 debug APK

Commands:

```sh
npm ci
npm run tauri android init -- --ci
npm run tauri android build -- --debug --target aarch64 --apk --ci
```

Environment:

- Node.js 22.23.0, npm 10.9.8
- Rust/Cargo 1.97.1, target `aarch64-linux-android`
- OpenJDK 21.0.11
- Gradle 8.14.3, Kotlin 2.0.21
- Tauri CLI 2.11.4
- Android SDK platform 36, Build Tools 36.1.0
- Android NDK 28.2.13676358 (`r28c`), Clang 19.0.1

Artifact observed before temporary-worktree cleanup:

- package: `dev.lorepia.spike.sqlitefts`
- `minSdkVersion=24`, `targetSdkVersion=36`
- native entry: `lib/arm64-v8a/libspikessqlitefts_lib.so` only
- APK size: 138,767,161 bytes
- APK SHA-256:
  `32c67163f51cc553059e2c5a107d218c1c5bb516911beda954558d3163c4505d`

The generated Tauri/Gradle code emitted deprecated-API and Java 8
source/target warnings, but the build exited 0. No emulator or physical device
was launched.

## iOS ARM64 simulator app

Commands:

```sh
npm ci
npm run tauri ios init -- --ci
npm run tauri ios build -- --debug --target aarch64-sim --ci --no-sign
```

Environment:

- Node.js 22.23.0, npm 10.9.8
- Rust/Cargo 1.97.1
- Xcode 26.6 build 17F113
- CocoaPods 1.16.2 (Homebrew formula 1.16.2_2)
- XcodeGen 2.45.4
- installed simulator runtime: iOS 26.5

Artifact observed before temporary-worktree cleanup:

- format: Mach-O 64-bit arm64 simulator executable
- app bundle size: 87,196 KiB
- executable size: 88,987,688 bytes
- executable SHA-256:
  `14e5223283d22263927934445d663db69ed6c60cceed302cdbd26b6157ce3bcc`
- signing: linker-generated ad-hoc signature without a development team

No simulator was booted and no physical iOS device, development-team signing,
archive, or App Store path was tested.

Both temporary worktrees and their large generated artifacts were removed after
the hashes and metadata above were captured. `npm ci` reported the same three
low-severity advisories on each target.
