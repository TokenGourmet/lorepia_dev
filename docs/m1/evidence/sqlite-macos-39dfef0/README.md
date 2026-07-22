# macOS SQLite/FTS5 packaged-app evidence — `39dfef0`

## Scope and result

- Gate/cell: macOS physical host / SQLite and FTS5
- Result: `PASS`
- Source commit: `39dfef0ec998f806093722dbb23c46d3239fe077`
- Execution time: 2026-07-18T21:09:18Z
- Evidence level: locally packaged debug `.app`, launched on the physical host
- Retained runtime launch: one packaged-app launch with screenshot artifact
- Tester identity: OpenAI Codex task `/root`; UI operator was the Computer Use
  runtime
- Excluded claims: release signing/notarization, Windows/Linux behavior,
  Android/iOS runtime, product schema, product-scale latency, and secure erase

The app was built in a detached worktree at the exact source commit. The main
repository worktree was not used as a build input. Computer Use launched the
packaged app, activated its only functional button, read back the plain-text
receipt, and closed the app. No UI styling or animation was added for this run.

## Host and toolchain

- macOS 26.5.2 build 25F84, arm64
- Mac mini `Mac16,10`, Apple M4, 16 GB memory
- Xcode 26.6 build 17F113
- Node.js 22.23.0, npm 10.9.8
- Rust 1.97.1 (`8bab26f4f68e0e26f0bb7960be334d5b520ea452`)
- SQLite runtime 3.53.2

Locked-input hashes:

- `package-lock.json`:
  `7244be7e7c89864f05f13247a6031cde77660048b850c442bce8922093b42141`
- `Cargo.lock`:
  `0fe34d374742ff899d41e6735576b6eed0c7f6e74b65cc214e07a408f13f5652`
- `korean-fts-v1.json`:
  `b5e8b2f2fdcf40d33dbb5eca555c982700e3cc1559dfe3adc878d85e2380b674`

## Exact build and test commands

From `spikes/sqlite-fts` in the detached worktree:

```sh
npm ci
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
npm run tauri build -- --debug --bundles app --ci
```

Results:

- frontend contract tests: `91/91 PASS`
- Rust tests: `11/11 PASS`
- packaged bundle: `LorePia SQLite FTS Spike.app`
- app bundle size: 31,256 KiB
- executable size: 31,895,752 bytes
- executable SHA-256:
  `4f86c155904322edd212dfa5a6a31fcc99515c67f96986728907230640b541cd`
- `npm ci` reported three low-severity advisories; the configured moderate
  threshold was not crossed

## Runtime receipt

The retained packaged-app launch returned the following bounded result:

- protocol version 1; schema version 2
- applied migrations `[1, 2]`; reopened v2 startup was an idempotent no-op
- a version-99 database was created, closed, reopened, rejected without a
  downgrade, and cleaned (`futureSchemaRejected: true`)
- ordered source `(id, title, raw_text)` rows matched the fixture exactly after
  reopen
- `journal_mode=wal`, normal busy timeout 250 ms
- independent reader/writer snapshot behavior passed
- immediate `SQLITE_BUSY` was observed and exactly one retry passed after the
  lock was released
- tokenizer `trigram`; bounded one/two-scalar policy
  `escaped-like-bounded`, result limit 64
- insert/update/delete FTS synchronization and FTS integrity check passed
- injection-shaped fixture term was treated as a literal
- `cleanupPending: false`

The native diagnostic transcript accepted these exact golden results. The
packaged-app screenshot separately proves that the strict frontend parser
accepted its IPC receipt, but a raw packaged IPC JSON capture was not retained:

| Query ID | Expected IDs | Actual IDs |
|---|---:|---:|
| `q-fts-euneunhan` | `[1]` | `[1]` |
| `q-fts-doseogwan` | `[1]` | `[1]` |
| `q-fts-jeonggeojang` | `[2]` | `[2]` |
| `q-like-byeol` | `[2]` | `[2]` |
| `q-like-bit` | `[1, 2, 4]` | `[1, 2, 4]` |
| `q-like-escaped-wildcards` | `[5]` | `[5]` |
| `q-fts-injection-literal` | `[]` | `[]` |

The native diagnostic transcript from the same commit and host returned this
nonempty, sorted, duplicate-free compile-option array containing the exact
`ENABLE_FTS5` token. The list below is not presented as a raw packaged IPC
capture:

```text
ATOMIC_INTRINSICS=1
COMPILER=clang-21.0.0
DEFAULT_AUTOVACUUM
DEFAULT_CACHE_SIZE=-2000
DEFAULT_FILE_FORMAT=4
DEFAULT_FOREIGN_KEYS
DEFAULT_JOURNAL_SIZE_LIMIT=-1
DEFAULT_MMAP_SIZE=0
DEFAULT_PAGE_SIZE=4096
DEFAULT_PCACHE_INITSZ=20
DEFAULT_RECURSIVE_TRIGGERS
DEFAULT_SECTOR_SIZE=4096
DEFAULT_SYNCHRONOUS=2
DEFAULT_WAL_AUTOCHECKPOINT=1000
DEFAULT_WAL_SYNCHRONOUS=2
DEFAULT_WORKER_THREADS=0
DIRECT_OVERFLOW_READ
ENABLE_API_ARMOR
ENABLE_COLUMN_METADATA
ENABLE_DBSTAT_VTAB
ENABLE_FTS3
ENABLE_FTS3_PARENTHESIS
ENABLE_FTS5
ENABLE_LOAD_EXTENSION
ENABLE_MEMORY_MANAGEMENT
ENABLE_RTREE
ENABLE_STAT4
HAVE_ISNAN
MALLOC_SOFT_LIMIT=1024
MAX_ATTACHED=10
MAX_COLUMN=2000
MAX_COMPOUND_SELECT=500
MAX_DEFAULT_PAGE_SIZE=8192
MAX_EXPR_DEPTH=1000
MAX_FUNCTION_ARG=1000
MAX_LENGTH=1000000000
MAX_LIKE_PATTERN_LENGTH=50000
MAX_MMAP_SIZE=0x7fff0000
MAX_PAGE_COUNT=0xfffffffe
MAX_PAGE_SIZE=65536
MAX_SQL_LENGTH=1000000000
MAX_TRIGGER_DEPTH=1000
MAX_VARIABLE_NUMBER=32766
MAX_VDBE_OP=250000000
MAX_WORKER_THREADS=8
MUTEX_PTHREADS
SOUNDEX
SYSTEM_MALLOC
TEMP_STORE=1
THREADSAFE=1
USE_URI
```

The full array and golden IDs are preserved in the verbatim
[`native-receipt-transcript.txt`](native-receipt-transcript.txt). It was emitted
by adding only a diagnostic `eprintln!` to the existing test in the disposable
worktree after the packaged-app run. That diagnostic line was not committed and
did not alter the production probe or the already-built app. The packaged UI
shows `PASS` only after its strict parser accepts those exact fields and IDs.
The transcript SHA-256 is
`977cb383600b2277f1810aa642f6a8a1dcf9aeaaf84de595116381ba7794d39a`
(4,295 bytes).

## Cleanup and retained artifact

After the retained app exit, an exact filename search under the macOS
Application Support root found none of the following:

- `lorepia-m1-sqlite-fts-probe.sqlite3`
- `lorepia-m1-sqlite-fts-probe.sqlite3-wal`
- `lorepia-m1-sqlite-fts-probe.sqlite3-shm`

The retained screenshot is [`runtime-pass.jpeg`](runtime-pass.jpeg), SHA-256
`27908cdaa3f192251950f04191ee4aafeca0eb99caa56b2384d354cbf9daf4ab`,
59,221 bytes. It shows the plain receipt through the fixture hash and the start
of the compile-option list; the diagnostic receipt above preserves the full
array.
