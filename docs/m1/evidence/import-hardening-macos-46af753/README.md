# macOS import-hardening packaged-app evidence — `46af753`

## Scope and result

- Gate: macOS physical host / disposable synthetic import-hardening core
- Result: `PASS` twice; all 26 fixed outcomes matched on each invocation
- Source commit: `46af7539dfc564ba2fc4c8009401878965799fc3`
- Retained runtime UTC window: 2026-07-18T22:18:12Z through
  2026-07-18T22:18:41Z ([line-ending-normalized window log](runtime-window.log))
- Evidence level: locally packaged debug `.app`, launched on the physical host
- Tester: OpenAI Codex task `/root`; UI operator was the Computer Use runtime

The app was built in a detached worktree at the exact source commit. Computer
Use launched the retained bundle, activated its only functional button twice,
read back the plain receipt after each invocation, and closed the app. Both
invocations reported two accepted sources, 24 expected hostile-input
rejections, two inert script entries with zero execution, an unchanged outside
sentinel, and no pending cleanup.

This is not a physical File import or Archive/import hardening matrix pass. The
command generated its own fixed bytes and used no document picker, external
file, URI, or Character Card. It also does not prove Windows/Linux/mobile
runtime, hostile-local-process no-follow safety, Windows reparse resistance,
crash durability, a hard CPU deadline, release signing, or notarization.

## Host and locked inputs

- macOS 26.5.2 build 25F84, arm64
- Mac mini `Mac16,10`, Apple M4, 16 GB memory
- Xcode 26.6 build 17F113
- Node.js 22.23.0, npm 10.9.8
- Rust 1.97.1 (`8bab26f4f68e0e26f0bb7960be334d5b520ea452`)

Locked-input hashes:

- `package-lock.json`:
  `becfe28991b1977f9a825df51761ff028b84c4a80511b9d3abcc8d6ee27ada99`
- `Cargo.lock`:
  `a55919478e93b45740329e9f64432ddf146b51d151fae9915b526bdf3cc9fc19`
- `import-cases-v1.json` (4,236 bytes):
  `484a313423d4e91c792818fb64097d96f8efb7c4a31befe96a1d3f739bfe5eb2`

## Exact build and artifact

From `spikes/import-hardening` in the detached worktree:

```sh
npm ci
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
npm run tauri build -- --debug --bundles app --ci
```

The retained [normalized terminal session](exact-build-session.log) is 8,273
bytes with SHA-256
`e414452cef5c9d65d5fe2634fe7c62ab7c4041dd106811b4fd05784aa6c10dc9`.
CR/ANSI terminal control codes were removed after capture; command output and
ordering were preserved. It records successful completion of all four commands:

- frontend contract tests: `106/106 PASS`
- Rust tests: `25/25 PASS`
- packaged bundle: `LorePia Import Hardening Spike.app`
- npm reported three low-severity development advisories; no moderate-or-higher
  audit threshold was crossed

Evaluated bundle metadata:

- bundle size: 31,256 KiB
- executable size: 31,897,536 bytes
- executable SHA-256:
  `9c71fc620e001e6e1e1fee9aaba96083d35bc0c4765741e60169f6a7d38f60e7`
- `Info.plist` SHA-256:
  `d5025b9a45e01170f57101b24dc14983d61fb2da4abf5393f0bb2cf84864039c`
- sorted relative bundle-file digest:
  `2c6a238a40d634aa55ef1cceb1243ba0edb6ffb4746befe16774a97764a43be5`

The 31 MiB `.app` itself is not committed or linked from an external artifact
store. This record retains its build log, hashes, runtime screenshot, and
transcripts only.

The debug executable was linker ad-hoc signed, with CDHash
`ef8cdf5517ee363957149c01d6600f870180562f`. Strict deep bundle verification
returned exit 1 because the bundle resources were not sealed. The app still
launched locally, but this record makes no distribution-signing claim.

## Runtime and cleanup

The strict frontend parser accepted the native result twice in the same app
launch during the retained UTC window above. That 108-byte normalized log has
SHA-256
`9eb8cd6c30ba8ee7a2a9fca8b45b10337329fa65df63799fa777be383d1bc4c6`.
The retained [second-pass screenshot](runtime-pass-utc-window.jpeg) is 78,887
bytes with SHA-256
`dad70c0d76c9e01b6f6fdead024668825639fb7085b6082075e4a2cdddff785b`.
The visible text is preserved separately in the [UI transcript](runtime-ui-transcript.txt).
An earlier [supplemental screenshot](runtime-pass.jpeg) shows the same receipt
but is not used to make a timestamp claim.

After the app exited, the exact app-local parent was empty. Both
`lorepia-m1-import-hardening-probe-v1` and
`lorepia-m1-import-hardening-outside-sentinel-v1` were absent.

The [native receipt](native-receipt.json) contains all case IDs, expected codes,
limits, golden hashes, and defense booleans. It was emitted by one
diagnostic-only `eprintln!` added after the packaged artifact had been built and
hashed, then removed. The targeted test passed against the exact committed
native source plus that logging line. It is not claimed as a raw capture of the
packaged WebView IPC payload; the screenshot separately proves that the
packaged strict frontend parser accepted its payload.
