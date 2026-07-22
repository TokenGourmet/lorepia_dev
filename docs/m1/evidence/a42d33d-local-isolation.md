# Local isolation observation for `a42d33d`

This record is a local packaged-app/emulator observation, not physical-device
M-1 evidence. It evaluates exact source commit
`a42d33d4e843da353f042d435133d3ac5f988fa4` and preserves failures as failures.

- Observation window: `2026-07-18T19:03:00Z` to `2026-07-18T19:08:08Z`
- Tester: local Codex execution on the named macOS host and Android emulator
- Plugin execution policy: imported JavaScript and Lua remain disabled

## Build identity

Both artifacts were rebuilt after the source commit was created.

| Artifact | Build command | SHA-256 |
|---|---|---|
| macOS arm64 release executable | `npm run tauri -- build --bundles app` | `de3a4ce30e547b7ff5ce31679c71485f07ae1c6318987ea3b2fee26549fe7cce` |
| Android universal debug APK containing the ARM64 library | `tauri android build --debug --target aarch64 --apk --ci` | `bd476c657d53b3aff3c1a33d7a5399658609e1409e5303f548720e5c4b781a6f` |

The hash reported for the installed Android `base.apk` matched the local APK
hash exactly.

## macOS packaged release observation

- Host: macOS `26.5.2` build `25F84`, arm64.
- Toolchain inventory: Xcode `26.6` build `17F113`.
- Launch hygiene: the previously running app process was quit before launching
  the newly built bundle.
- Scenario: open `/isolation`, rotate to broker generation 2, require the stale
  generation-1 token audit, run the full negative suite, then stop watchdog
  pong responses.
- Result: `18/18 PASS`, `18/18` complete.
- Stale token: `INVALID_HOST_TOKEN`; probe and sanitizer counters unchanged
  during that audit.
- Retired raw commands: the top frame attested the exact 8-command surface v2
  and observed all retired wrapper calls denied.
- Direct iframe broker/registration calls: all three reported that the Tauri
  invoke transport was absent. This is effect-level evidence, not ACL-decision
  evidence.
- Authorized suite sink delta: probe `1 -> 2`; sanitizer `0 -> 1`. No extra
  monitored sink effect occurred.
- Watchdog: last issued ping `27`, last accepted pong `25`, two consecutive
  deadline misses, state `disabled`, iframe URL `about:blank`.
- Final screenshot: JPEG, 50,621 bytes, SHA-256
  `0c5233bb0e0db0608801be3d49bd50a977354940d28889ad5922ac2a3a62a863`.
  It was produced in the automation service's temporary directory and was not
  copied to a durable evidence store.

## Android emulator observation

- Device: `sdk_gphone64_arm64`, Android 16 / API 36, build fingerprint
  `google/sdk_gphone64_arm64/emu64a:16/BE2A.250530.026.F3/13894323:userdebug/dev-keys`.
- WebView: `133.0.6943.137`.
- Scenario: install the exact APK with `adb install -r`, force-stop and relaunch
  the app, open `/isolation`, rotate to broker generation 2, run the full
  negative suite, then stop watchdog pong responses.
- Result: `15/18 PASS`, `18/18` complete; overall result **FAIL**.
- Preserved failures:
  - `direct-broker-missing-token-denied`: `FAIL: INCONCLUSIVE`, native callback
    timed out.
  - `direct-broker-wrong-token-denied`: `FAIL: INCONCLUSIVE`, native callback
    timed out.
  - `direct-registration-takeover-denied`: `FAIL: INCONCLUSIVE`, native callback
    timed out.
- Timeout interpretation: a missing callback is not proof of denial. Android's
  per-frame Tauri callback behavior left the response semantics inconclusive.
- Effect audit despite those timeouts: stale generation-1 token audit passed;
  the exact 8-command v2 retired-wrapper audit passed; authorized suite sink
  delta was probe `1 -> 2` and sanitizer `0 -> 1`, with no extra monitored sink
  effect.
- Watchdog: last issued ping `57`, last accepted pong `55`, two consecutive
  deadline misses, state `disabled`, iframe `src` changed to `about:blank`.
- Final screenshot stream: PNG, 161,861 bytes, SHA-256
  `9023367a03fe49cd6eba55d918fd76980a2b5fba876ca510c6f0735660081e2f`.
  The stream was hashed directly and the raw image was not retained.

This emulator run does not change the Android physical-device matrix cell.

## Remaining limits

- The combined spike still exposes four raw Channel commands to its one Tauri
  window.
- Tauri command arguments are allocated and deserialized before the Rust
  broker's size/admission checks.
- Browser structured clone materializes iframe messages before the host-side
  fixed-window gate runs.
- Runtime rate-limit abuse and iframe self-navigation egress were not exercised
  in these observations.
- The cooperative watchdog does not preempt a synchronous busy loop sharing the
  host event loop.
- Windows, Linux, physical Android, physical iOS, and the remaining M-1
  capabilities were not run.
