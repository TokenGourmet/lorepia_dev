# iOS simulator Keychain entitlement failure

This is an immutable candidate observation for source commit
`4fde100605a4684aae3d3a54314f0cfa021c843f`. It records a real simulator
launch and a failed native Protected Data Keychain call. It is not a
physical-device result and does not change the iOS physical-device matrix cell.

## Result

- Gate: Keychain candidate observation / iOS simulator
- Result: **FAIL**
- UTC timestamp: `2026-07-18T20:24:41.368Z`
- Host: macOS `26.5.2` build `25F84`, arm64
- Toolchain: Xcode `26.6` build `17F113`, Tauri CLI `2.11.4`, Rust `1.97.1`,
  Node.js `22.23.0`, npm `10.9.8`
- Simulator: iPhone 17 Pro, iOS `26.5` build `23F77`
- Simulator UDID: `C1A0BEBE-B01F-4B6E-A7D0-1897C21D0D35`
- Bundle ID: `dev.lorepia.spike.keychain`

Expected: the `ios-protected-data` backend completes the seven-step
absent/create/read/update/read/delete/absent lifecycle and returns
`cleanupPending: false`.

Actual: the first `SecItemCopyMatching` call was denied with OSStatus `-34018`:

> Client has neither application-identifier nor keychain-access-groups entitlements

The frontend therefore displayed the bounded `STORE_FAILURE` message with
`cleanupPending: true`. The latter is the probe's conservative response when it
cannot inspect the cleanup registry; it is not evidence that a credential was
created or remains. The failure happened on the initial registry lookup, before
the target credential lifecycle began.

## Reproduction

The disposable Apple wrapper did not exist in the exact detached worktree. It
was generated and the no-sign simulator build was installed and launched with:

```sh
cd spikes/keychain
npm ci
npm run tauri ios init -- --ci
npm run tauri ios build -- --debug --target aarch64-sim --ci --no-sign
xcrun simctl install C1A0BEBE-B01F-4B6E-A7D0-1897C21D0D35 \
  "src-tauri/gen/apple/build/arm64-sim/LorePia Keychain Spike.app"
xcrun simctl launch --terminate-running-process \
  C1A0BEBE-B01F-4B6E-A7D0-1897C21D0D35 \
  dev.lorepia.spike.keychain
```

The tester then pressed the single `수명주기 검증 실행` button once.

## Build identity

- App executable SHA-256:
  `fb8309f8f1a2ec9e7c5a458c0dbc2ebd1e9abe29a22338d7a6d6a43c397232be`
- `package-lock.json` SHA-256:
  `192b107b58baa7aef7a4f7ecfae61d717604a6736c04cba6b8802d4bb587d7ef`
- `Cargo.lock` SHA-256:
  `819270c267a1ce6e236b36b86caee0d0dde8e4dbed20189e699b777837c99b33`

The no-sign executable was linker ad-hoc signed, had no TeamIdentifier, and the
generated entitlements plist was an empty dictionary. The exact signature
inspection is preserved in [`signature.txt`](signature.txt).

## Preserved artifacts

- [`runtime-failure.png`](runtime-failure.png): simulator UI after the failed call
- [`install-launch.log`](install-launch.log): exact no-entitlement install and launch
- [`securityd.log`](securityd.log): initial Keychain call and the `-34018` denial
- [`signature.txt`](signature.txt): app signature and generated entitlement state
- [`app-manifest.sha256`](app-manifest.sha256): complete no-sign app file manifest
- [`sha256.txt`](sha256.txt): hashes and byte sizes for the preserved artifacts

The screenshot and logs contain no secret, generated account, raw credential,
or diagnostic sentinel.

## Scope and next requirement

This result proves that the `--no-sign` build is compile-only and cannot be used
for Protected Data runtime validation. It does not prove that the Rust lifecycle
is defective, nor does it prove iOS device persistence, locked-device behavior,
access-group behavior, or non-leakage into future product SQLite/export/crash
paths. A development-team-signed build with effective `application-identifier`
and `keychain-access-groups` entitlements must be run next; the full iOS matrix
cell still requires a physical-device evidence record.
