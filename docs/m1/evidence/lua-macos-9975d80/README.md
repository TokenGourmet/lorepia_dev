# macOS Lua-limit packaged-app evidence — `9975d80`

## Scope and result

- Gate: macOS physical host / Lua limit enforcement and Lua negative cases
- Result: `PASS` twice through the packaged strict frontend parser
- Source commit: `9975d80521c61d5c25bc9509c609439cfa371d58`
- Retained runtime UTC window: 2026-07-18T23:03:32.520Z through
  2026-07-18T23:03:44.919Z
- Evidence level: locally packaged debug `.app`, launched on the physical host
- Tester: OpenAI Codex task `/root`; UI operator was the Computer Use runtime

The app was built in a clean detached worktree at the exact source commit.
Computer Use launched that bundle, activated its only functional button twice
in one app process, read the plain success receipt after each invocation, and
closed the app. Each invocation passed the strict frontend contract for the
fixed 11-case corpus: six allowed/recovery sentinels, three hostile limit
interruptions, and two absent-global checks.

This record qualifies the named macOS Lua runtime capability and the macOS Lua
budget/forbidden-library negative scenario. It does not establish Windows,
Linux, Android, or iOS runtime behavior; imported-Lua execution; a product
scripting API; hard real-time termination; native-callback preemption; release
signing; notarization; or Store-Safe approval.

`stdlib removal` means that only `math`, `string`, `table`, and `utf8` exist in
the fixture VM's global table. The vendored Lua archive still contains compiled
`luaopen_*` implementation symbols. This is runtime non-initialization and
non-exposure, not binary-level removal of those object files.

## Host and locked inputs

- macOS 26.5.2 build 25F84, arm64
- Mac mini `Mac16,10`, Apple M4, 16 GB memory
- Xcode 26.6 build 17F113
- Node.js 22.23.0, npm 10.9.8
- Rust 1.97.1 (`8bab26f4f68e0e26f0bb7960be334d5b520ea452`)

Locked-input hashes:

- `package-lock.json`:
  `049d114c3b4c672e38bb24a897bdd2702371b7ad7bebc7ae3f106465a73d93b5`
- `Cargo.lock`:
  `0f99c5dc2d5faad92b9926fe67b0c53a1d7a821059f91bac67942d2d746a3307`
- `fixtures/catalog.json` (1,376 bytes):
  `9ea567d6901ec39412e73f439ee9ea7d47538baea4d1a92cd409c9f3e9b97db5`

## Exact build and artifact

From `spikes/lua-limits` in the detached worktree:

```sh
npm ci
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
npm run tauri build -- --debug --bundles app --ci
```

The retained [normalized build session](exact-build-session.log) is 15,918
bytes with SHA-256
`8924c185b8f7e4cf97ad7a6f96174cbdd0276cfbcefc3edca5aa45316072abbf`.
CR and ANSI terminal controls were removed after capture without changing
command output ordering. It records:

- frontend contract tests: `49/49 PASS`;
- native tests: `12/12 PASS`;
- exact clean source commit and locked-input hashes; and
- packaged bundle: `LorePia Lua Limits Spike.app`.

`npm ci` reported three low-severity development advisories. The source commit
separately passed the moderate-threshold audit, and its production dependency
audit reported zero vulnerabilities.

Evaluated bundle metadata:

- bundle size: 30,136 KiB; three regular files
- executable size: 30,749,992 bytes
- executable SHA-256:
  `02785a6cfd66ca0d5eae80bad3933c8c478ba561a5efb8a8c8f623cfb4b69684`
- executable format: Mach-O 64-bit arm64
- `Info.plist` size: 1,021 bytes
- `Info.plist` SHA-256:
  `4c60322f9011f5c13f7a904010f263adffd643b9261ea3a8de45b78ca95daf04`
- bundle identifier: `dev.lorepia.spike.lualimits`
- sorted relative bundle-file manifest SHA-256:
  `4590ed35b39c012dff47ea8afbb0c9333775a1abc1e3cc0f256995665b3d1903`

The [artifact inspection](artifact-inspection.log) is 6,564 bytes with
SHA-256
`ba48098b47d079840c3e0e8a3ed8980793e7bd6abda65657f631782017c6f5a4`.
The executable was linker ad-hoc signed with CDHash
`0328ee293f3f1cd9b41d38434976f06ece5326d8`. Strict deep bundle
verification exited 1 because resources were not sealed. The app launched
locally, but this record makes no distribution-signing claim.

## Packaged runtime

Both sequential button activations visibly returned:

- policy `m1-lua-limits-v1`;
- fixture-catalog hash
  `9ea567d6901ec39412e73f439ee9ea7d47538baea4d1a92cd409c9f3e9b97db5`;
- `Lua 5.4 / mlua 0.12.0`;
- eight completed/verified cases and three limit interruptions;
- 50 ms, 100,000-instruction, and 8,388,608-byte policies;
- dangerous global/protected-call/coroutine surface absent; and
- host recovery after hostile fixtures.

The [first UI transcript](runtime-ui-first.txt) and
[second UI transcript](runtime-ui-second.txt) are each 1,172 bytes with the
same SHA-256
`b8d90c99f51e0cbe0661df267f24566093904f31cb57bfeee2280e65a6b94a76`.
The [first screenshot](runtime-first.png) is 49,242 bytes with SHA-256
`f0fa9a8480fafe2d7082e96ae403231e9bb6813b80482e00189c72538c2db393`;
the [second screenshot](runtime-second.png) is 49,243 bytes with SHA-256
`edab968daccdfbe482688162682121dbc86e3280ae488913a8cc19467f167d4a`.
The screenshots and accessibility transcripts prove packaged strict-parser
acceptance; they are not raw IPC captures.

The 92-byte [runtime window](runtime-window.log) has SHA-256
`6f116108e17c0ed3eb8816fd45bac81f73b246a0f8a374adc96e62062da4c1ca`.
After close, the executable was no longer running and no matching LorePia Lua
DiagnosticReports filename existed. That [postcheck](runtime-postcheck.log) is
397 bytes with SHA-256
`3d0aeebe9766f81ed471bc53280f9d6bf356cdb778e6b10339514e98e6640ba3`.

Two sequential successes prove state recovery and lock release between calls.
They do not exercise the concurrent `PROBE_BUSY` path; that remains a native
regression-test result.

## Native measured receipt

After the packaged artifact had been built, hashed, and run, one diagnostic
`println!` was added to the exact committed receipt-shape test. The targeted
test passed, the line was removed, and `git diff --exit-code` confirmed that the
source returned to the exact commit. Therefore
[`native-receipt.json`](native-receipt.json) is a same-source, same-host native
measurement, not a capture of either packaged WebView invocation.
The later [source-restoration check](source-restoration-check.log), captured
before temporary-worktree cleanup, independently records the exact HEAD, empty
tracked status, and successful worktree and index diffs.

The receipt is 3,139 bytes with SHA-256
`dd01fc154189444ef46b33b42495ceae7ae5069f6cf2ddb01469c59a378d2d84`,
below the 4,096-byte IPC ceiling. The independent
[invariant check](receipt-validation.log) passed exact order, policy, result,
instruction, recovery, and memory relationships.

| Case | Outcome / code | Result | Elapsed µs | Hooks / estimate | Used / observed peak bytes |
|---|---|---:|---:|---:|---:|
| `allowed-baseline` | `ALLOWED / ALLOWED_RESULT` | 55 | 62 | 0 / 0 | 18,328 / 18,328 |
| `infinite-loop` | `INTERRUPTED / INSTRUCTION_LIMIT` | — | 496 | 100 / 100,000 | 17,555 / 17,619 |
| `recovery-after-infinite-loop` | `ALLOWED / ALLOWED_RESULT` | 55 | 12 | 0 / 0 | 18,328 / 18,328 |
| `recursive-pressure` | `INTERRUPTED / INSTRUCTION_LIMIT` | — | 4,034 | 100 / 100,000 | 1,616,968 / 4,526,920 |
| `recovery-after-recursive-pressure` | `ALLOWED / ALLOWED_RESULT` | 55 | 22 | 0 / 0 | 18,328 / 18,328 |
| `allocator-pressure` | `INTERRUPTED / MEMORY_LIMIT` | — | 803 | 0 / 0 | 8,345,612 / 8,345,612 |
| `recovery-after-allocator-pressure` | `ALLOWED / ALLOWED_RESULT` | 55 | 14 | 0 / 0 | 18,328 / 18,328 |
| `forbidden-globals-absent` | `ABSENT / FORBIDDEN_GLOBALS_ABSENT` | — | 22 | 0 / 0 | 19,308 / 19,308 |
| `recovery-after-forbidden-globals` | `ALLOWED / ALLOWED_RESULT` | 55 | 9 | 0 / 0 | 18,328 / 18,328 |
| `bypass-surfaces-absent` | `ABSENT / BYPASS_SURFACES_ABSENT` | — | 11 | 0 / 0 | 18,590 / 18,590 |
| `recovery-after-bypass-surfaces` | `ALLOWED / ALLOWED_RESULT` | 55 | 8 | 0 / 0 | 18,328 / 18,328 |

Every row used the 8,388,608-byte ceiling. For every row,
`instructionEstimate == hookCount * 1,000`, used memory was no greater than the
observed peak, and the observed peak stayed below the ceiling. All five defense
booleans were true. The receipt session is retained separately so the temporary
diagnostic source delta and successful targeted test remain auditable.

## Integrity

The [log validation](normalized-log-validation.txt) records zero CR, ANSI
escape, and backspace bytes for every retained `.log`. The
[evidence manifest](evidence-sha256.txt) covers every retained evidence file
except this README and the manifest itself. Verify it from this directory with
`shasum -a 256 -c evidence-sha256.txt`.

## Limitations

- The 50 ms deadline is a cooperative hook check, not a hard preemptive timer.
  This fixed pure-Lua corpus exposed no native callback between hook points.
- The 8 MiB value bounds the Lua allocator, not Rust, WebView, native stack, or
  whole-process memory.
- The diagnostic command accepts no imported source, path, fixture, or limit.
  This record cannot justify enabling imported Lua.
- No filesystem or network API was exposed, but this is not proof for a future
  API that adds callbacks, userdata, modules, or I/O.
- Other physical platforms and all mobile lifecycle, backgrounding, thermal,
  store-policy, and release-signing scenarios remain separate work.
