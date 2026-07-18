# Android emulator keychain observation for `38ce4c2`

This is a local emulator observation of the independent M-1 keychain spike. It
does not change the Android physical-device matrix cell and does not prove that
future SQLite, export, or crash-reporting paths exclude secrets.

- Source commit: `38ce4c2c73a897a146859d78229b024e43004ffd`
- Frontend contract parent: `cedbaf56d2cb6feb16a5cda6bb2fcdc2cf327816`
- Observation window: `2026-07-18T20:12:17Z` to `2026-07-18T20:13:00Z`
- Tester: local Codex execution on the named Android emulator

## Build and install identity

The source commit was checked out into a clean detached worktree, built after
the commit existed, and installed with:

```sh
npm run tauri android build -- --debug --target aarch64 --apk --ci
/opt/homebrew/bin/adb -s emulator-5554 install -r \
  src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

| Item | SHA-256 |
|---|---|
| Universal debug APK | `ac80d7b5dabc1c5fcedd836873eaab476db11c29c06dabe48b82b503ebddbff3` |
| APK pulled back from the installed `base.apk` path | `ac80d7b5dabc1c5fcedd836873eaab476db11c29c06dabe48b82b503ebddbff3` |
| `spikes/keychain/package-lock.json` | `192b107b58baa7aef7a4f7ecfae61d717604a6736c04cba6b8802d4bb587d7ef` |
| `spikes/keychain/src-tauri/Cargo.lock` | `819270c267a1ce6e236b36b86caee0d0dde8e4dbed20189e699b777837c99b33` |

Installed package identity was `dev.lorepia.spike.keychain`, version `0.1.0`,
version code `1000`, target SDK `36`.

Local build toolchain: Node `22.23.0`, npm `10.9.8`, Rust `1.97.1`, OpenJDK
`21.0.11`, Android NDK `28.2.13676358` (`r28c`), and Gradle `8.14.3`.

## Runtime identity

- AVD: `Akarisu_API_36`
- Device: `sdk_gphone64_arm64`
- ABI: `arm64-v8a`
- Android: 16 / API 36
- Build fingerprint:
  `google/sdk_gphone64_arm64/emu64a:16/BE2A.250530.026.F3/13894323:userdebug/dev-keys`
- WebView: `com.google.android.webview 133.0.6943.137`

## Scenario and result

The app log was cleared, the package was force-stopped, and its `MainActivity`
was cold-launched. The operator pressed `수명주기 검증 실행` once and then
captured the final UI hierarchy, screenshot, and logs scoped to the new app
process.

Expected:

- the strict frontend parser accepts the native response only when all seven
  lifecycle flags are `true` and `cleanupPending` is `false`;
- the UI reports the Android Keystore-backed encrypted-preferences backend;
- the run ID and reference fingerprint have the bounded hexadecimal formats;
- the app process has no fatal exception or Rust panic.

Actual: **PASS for this emulator scenario**.

- Strict-parser-established lifecycle receipt:
  `absentBeforeCreate=true`, `created=true`, `initialReadMatched=true`,
  `updated=true`, `updatedReadMatched=true`, `deleted=true`, and
  `absentAfterDelete=true`.
- UI status: `키체인 수명주기 검증을 통과했습니다.`
- Backend: `android-keystore-encrypted-preferences`
- Run ID: `0e8fa41e513ddad0cd9c86f53e6c4be7`
- Reference fingerprint: `dce32c6d473dfc6e`
- Previous stale cleanup recovered: `false` (`해당 없음`)
- Cleanup pending: `false` (`아니요`)
- No app fatal exception or Rust panic appears in the process-scoped log.

The UI does not print seven redundant booleans. Its success state is reachable
only after the exact-response parser has verified all seven as literal `true`;
that parser behavior is separately covered by the committed frontend contract
tests. The probe's final native step also verifies that the target credential
returns `NoEntry` after deletion.

## Preserved artifacts

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| [`runtime.png`](runtime.png) | 113,801 | `b5188c06a516ec7b454a119ccd2c8cbbed0d224d83981702395758e22c25cfcc` |
| [`ui.xml`](ui.xml) | 9,020 | `2e7891c789787a0db5d1e0cf1569315eaf1a5cad8489506b80e9ea6c5687180d` |
| [`logcat.txt`](logcat.txt) | 13,172 | `ddf5f352383dc238bdf4e260412cc8727d7a11fd2466f2135985c310a2e13ea2` |

The startup log retains WebView first-run cache reconstruction messages and
emulator graphics-driver warnings. They are not suppressed or reclassified as
keychain failures; the stated result is limited to the app lifecycle scenario
and the absence of an app fatal exception or Rust panic.

## Limitations

- An emulator does not establish physical Android Keystore, lifecycle,
  backup/restore, device-transfer, or hardware-backed key behavior.
- The run did not inject a diagnostic sentinel into future SQLite, export, or
  crash-reporting paths because those product subsystems do not exist in this
  independent spike.
- Credential deletion means the API returned `NoEntry`; it is not a forensic
  secure-erasure claim.
- This record covers one Android image only. Windows, macOS, Linux, physical
  Android, and physical iOS runtime cells remain unchanged.
