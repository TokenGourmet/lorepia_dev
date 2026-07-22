# M-1 SQLite/FTS5 vertical boundary

This record defines the disposable five-OS SQLite/FTS5 spike. It is an
implementation and evidence contract, not a runtime `PASS`. Hosted compilation,
unit tests, a simulator, and an emulator do not replace the five physical-OS
cells in [`verification-matrix.md`](verification-matrix.md).

The spike answers one narrow question: can the selected bundled SQLite build
migrate and reopen a file-backed database, provide the required WAL behavior,
and return deterministic Korean substring-search results on each target? It is
not LorePia's product database. The full branching-chat schema and production
database API remain M1 work and are not frozen by this probe.

## Exact probe boundary

The spike uses `rusqlite 0.40.1` with default features disabled and the bundled
SQLite feature enabled. It exposes one no-argument Tauri command,
`run_sqlite_m1_probe`. The WebView cannot provide a database path, SQL text,
migration, fixture, or search term. Rust owns the fixed app-local filename
`lorepia-m1-sqlite-fts-probe.sqlite3` and the committed fixture
[`spikes/sqlite-fts/fixtures/korean-fts-v1.json`](../../spikes/sqlite-fts/fixtures/korean-fts-v1.json).
The fixture is synthetic and self-authored; its per-file origin, `CC0-1.0`
permission, byte size, and canonical SHA-256 are pinned in the adjacent
[`fixtures/README.md`](../../spikes/sqlite-fts/fixtures/README.md).

The command returns bounded proof metadata and deterministic golden-result
evidence, not a general query result or database handle. The proof includes the
runtime SQLite version, sorted SQLite compile options, `trigram` as the selected
tokenizer, an executed future-schema rejection, the SHA-256 of the exact
fixture, and `shortQueryLimit: 64`. Neither
success nor error IPC exposes the app-local path, filename, SQL text, or raw
SQLite error. A process-wide non-blocking lock covers initialization,
migrations, the concurrency scenario, search checks, close, and cleanup. A
concurrent invocation returns `PROBE_BUSY` rather than opening a second probe
lifecycle.

This deliberately small command surface must not be reused as the product
repository or exposed as arbitrary SQL IPC.

## Search contract

The probe fixes the following behavior for the committed golden fixture:

- A non-empty query of three or more Unicode scalar values uses FTS5 with the
  `trigram` tokenizer. The term is encoded as one quoted `MATCH` phrase, with
  embedded quote characters escaped for FTS5. Results use deterministic
  `ORDER BY rowid` where the rowid equals the logical fixture ID; this path does
  not fall back to a full-table `LIKE` scan.
- A query of exactly one or two Unicode scalar values does not use trigram
  `MATCH`, because FTS5 trigram tokens cannot represent those substrings. It
  uses the source text column with `LIKE ... ESCAPE '\'`, after escaping `\`,
  `%`, and `_` as literal characters, and is bounded by the exact suffix
  `ORDER BY id LIMIT 64`.
- Empty input is invalid. The fixed golden cases prove Korean matches, stable
  ordering, no duplicate rows, no false wildcard expansion in the short-query
  fallback, and repeatability after close and reopen.

The one- and two-character fallback is a bounded compatibility decision for
this probe, not approval for an unbounded product-table scan. Product indexing,
pagination, normalization, ranking, and query-API policy remain M1 decisions.

## Migration and file lifecycle

The native probe performs this lifecycle:

1. Before opening a new run, remove only the fixed probe database and its exact
   `-wal` and `-shm` siblings. Failure to establish a clean start fails closed.
2. Create a version-99 database at the same fixed path, close and reopen it,
   prove the migration runner returns `MIGRATION_FAILURE` without changing the
   version, then clean those same three owned files. The success receipt pins
   this executed branch as `futureSchemaRejected: true`.
3. Apply v0-to-v1 to create only `schema_meta`, `probe_marker`, and
   `fixture_records`, load the fixture, commit it, close every connection, and
   reopen the file to prove the ordered `(id, title, raw_text)` rows match the
   fixture exactly, not merely by count.
4. Prove both `ENABLE_FTS5` and creation of a temporary FTS5 table using the
   `trigram` tokenizer before migration. Absence maps to `FTS_UNAVAILABLE`.
   Then apply the probe's forward v1-to-v2 migration. V2 adds an
   external-content FTS5 trigram table, its insert/update/delete synchronization
   triggers, and a one-time index rebuild for the existing rows.
5. Close the v2 connection, reopen a new connection, and run migration startup
   again to prove v2 is an idempotent no-op.
6. Close and reopen again, then prove the exact source rows, index, and
   deterministic golden searches survived the lifecycle.
7. Close all connections and remove only the fixed database, `-wal`, and `-shm`
   files. The result reports `cleanupPending` if absence cannot be proved.

The v1 and v2 labels above belong only to the disposable probe. They do not
represent versions of the data model in `LorePia_기술계획서_v2.md`, and they
must not be copied into production migration history by implication.

## WAL and concurrency acceptance

WAL mode must be active after reopen, not merely requested by a pragma. Using
two independent native connections, the probe demonstrates all of the
following:

- a reader retains its established snapshot while a writer commits through the
  other connection;
- a new read observes the committed value after the earlier snapshot ends;
- an intentionally contended test write temporarily disables waiting to observe
  an immediate `SQLITE_BUSY`; after the lock is released, the connection
  restores the normal 250 ms timeout and performs exactly one successful retry;
  and
- all committed values remain present after both connections close and the
  database reopens.

An infinite busy timeout, an unbounded retry loop, shared-connection-only tests,
or an in-memory database cannot satisfy this acceptance. Exact source/unit
tests must pin the retry count/deadline used by the implementation.

## Acceptance evidence

A qualifying physical-platform record must contain the normal M-1 evidence
fields plus:

- the exact app commit and dependency-lock hashes;
- physical OS/build and hardware identity;
- runtime SQLite version and the sorted `PRAGMA compile_options` output;
- confirmed `journal_mode=wal` and tokenizer `trigram`;
- fixture SHA-256 and deterministic expected/actual IDs for every golden query;
- migration, two-connection snapshot, busy/retry, reopen-persistence, and final
  cleanup results; and
- raw logs or an artifact link that does not contain unrelated user database
  contents.

Desktop CI can establish source tests and native compilation on its named
runner. Android CI builds an ARM64 debug APK, and iOS CI builds an ARM64
simulator app without launching it. Those mobile jobs are compile-only. A local
simulator or emulator run remains simulated evidence. None of these can update
a physical SQLite/FTS5 matrix cell.

## Evidence this spike does not claim

- It does not implement or freeze the product `characters`, `chats`, branching
  `messages`, `request_state`, lorebook, provider, module, asset, or settings
  schema. That full schema remains M1.
- It does not define the product repository API, connection pool, IPC surface,
  pagination, backup/import format, downgrade handling, corruption recovery, or
  long-running multi-process locking policy.
- The fixed fixture and `LIMIT 64` short-query path do not prove product-scale
  latency or the v2 plan's p95 targets.
- Cleanup proves that the named probe database and its WAL/SHM siblings are
  absent through ordinary filesystem APIs. It is not a forensic secure-erase
  claim.
- No visual design, animation, or product interaction pattern is part of this
  vertical.

## Upstream references

- [SQLite FTS5 and the trigram tokenizer](https://www.sqlite.org/fts5.html)
- [SQLite write-ahead logging and reader/writer concurrency](https://sqlite.org/wal.html)
- [`rusqlite 0.40.1` API documentation](https://docs.rs/rusqlite/0.40.1/rusqlite/)
