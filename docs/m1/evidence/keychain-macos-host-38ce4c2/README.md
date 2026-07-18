# macOS host keychain observation for `38ce4c2`

This is a local physical-host observation of the independent M-1 keychain
spike. It proves the bounded lifecycle against the login Keychain on the named
Mac, but it does not close the macOS physical-machine matrix cell because the
future product SQLite, export, and crash-reporting paths do not exist here.

- Source commit: `38ce4c2c73a897a146859d78229b024e43004ffd`
- Frontend contract parent: `cedbaf56d2cb6feb16a5cda6bb2fcdc2cf327816`
- Observation and artifact-capture window: `2026-07-18T20:16:09Z` to
  `2026-07-18T20:18:38Z`
- Tester: local Codex execution and Computer Use observation on the named host

## Build identity

The source commit was checked out into the same clean detached worktree used by
the Android exact-commit run, then packaged after the commit existed:

```sh
npm run tauri -- build --bundles app
```

Bundle file manifest:

| File | SHA-256 |
|---|---|
| `Contents/Info.plist` | `8e1c582303941065d040cfa1faa1fb90bdc02e625383577534b94d7631c6ac1c` |
| `Contents/MacOS/lorepia-keychain-spike` | `d59f9c3a55e549426100721ad4fba135772adfaf097e9359ae149096b50d5643` |
| `Contents/Resources/icon.icns` | `3dc10493b7de48a61de58f768f8a5708d3a44a068c148cedf0502b9b9b71ba5d` |
| `spikes/keychain/package-lock.json` | `192b107b58baa7aef7a4f7ecfae61d717604a6736c04cba6b8802d4bb587d7ef` |
| `spikes/keychain/src-tauri/Cargo.lock` | `819270c267a1ce6e236b36b86caee0d0dde8e4dbed20189e699b777837c99b33` |

The local bundle executable was arm64 with bundle identifier
`dev.lorepia.spike.keychain`. Its linker-generated ad-hoc code-sign identifier
was `lorepia_keychain_spike-ab0c5cfc2ad75349`; it was not a notarized
distribution build. Toolchain inventory was Node `22.23.0`, npm `10.9.8`, Rust
`1.97.1`, and Xcode `26.6` build `17F113`.

## Runtime identity

- Host: macOS `26.5.2` build `25F84`, arm64
- Backend selected by the exact binary: `macos-keychain`
- Store boundary: the user's login Keychain
- UI transport: packaged Tauri `tauri://localhost` main WebView

## Scenario and result

The exact packaged executable was launched directly so its stdout and stderr
remained attached to the test terminal. The operator ran the one lifecycle
button twice consecutively and read back the final accessibility tree and
screenshots after each run.

Expected for each run:

- the strict frontend parser accepts the response only when all seven
  lifecycle flags are literal `true` and `cleanupPending` is `false`;
- the backend is `macos-keychain`;
- the run ID and reference fingerprint match their bounded hexadecimal shapes;
- the second run reports no stale item from the first run;
- the app remains alive without a Rust panic or macOS crash report.

Actual: **PASS for both local host lifecycle runs**.

| Run | Run ID | Reference fingerprint | Lifecycle | Stale recovery | Cleanup pending |
|---|---|---|---|---|---|
| 1 | `a4bb31d41db94bb96cb131ea5148a806` | `0fb74df89533efee` | `7/7 true` | `false` | `false` |
| 2 | `2b324b0aead35c72653ec0ee1d47a6fa` | `d1181fb8f011d403` | `7/7 true` | `false` | `false` |

For both rows, `7/7 true` means `absentBeforeCreate`, `created`,
`initialReadMatched`, `updated`, `updatedReadMatched`, `deleted`, and
`absentAfterDelete`. The UI success state is reachable only after the committed
exact-response parser verifies those values. The process remained alive after
the second receipt, produced no stdout/stderr, and created no matching entry in
the host DiagnosticReports directory during the observation.

## Preserved artifacts

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| [`runtime-first.jpeg`](runtime-first.jpeg) | 37,794 | `cb7c5ceaa17fb8f63193ce0b9bf056a301080c3df9ee4344666a15b06472b38c` |
| [`runtime-second.jpeg`](runtime-second.jpeg) | 37,745 | `1be11e67f3aafc37029ea1a202d5e54233940e9c9144397920aa546de6fe16b4` |
| [`error-fault.log`](error-fault.log) | 11,258 | `13e0f1e4a5b56f301d10fe329530e43ea2ded429ffdb1cd927df0bde2b7f1e00` |

The retained error/fault slice includes unrelated ad-hoc-bundle and sandboxed
WebKit framework messages: AppIntents registration, unavailable pasteboard and
system services, view geometry, and the absent DetachedSignatures database. It
also contains WebKit's historical `CRASHSTRING` label for a denied
LaunchServices connection. The app did not terminate, no crash report was
created, and the two later keychain receipts succeeded. These messages are
preserved rather than silently filtered from the record.

## Limitations

- This was an ad-hoc local bundle, not a Developer ID, notarized, sandboxed App
  Store, or Mac App Store build.
- The run proves the login-Keychain API lifecycle only on this host and user
  session. It does not establish locked-Keychain, migration, restore, other
  users, or other macOS versions.
- The probe does not disclose its random secrets or raw credential account, so
  this run had no known diagnostic sentinel with which to scan every host log
  encoding.
- The independent spike contains no product SQLite, settings export, or crash
  upload pipeline. Those non-leakage gates therefore remain `NOT RUN`.
- API `NoEntry` after delete is not a forensic secure-erasure claim.
