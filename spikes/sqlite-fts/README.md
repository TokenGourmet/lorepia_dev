# M-1 SQLite/FTS5 spike

This disposable Tauri 2 + SvelteKit application validates a file-backed SQLite
database, forward migrations, WAL concurrency, and deterministic Korean search.
It has one functional button and a plain-text receipt. Product visual design,
CSS, and animation are intentionally out of scope.

## Boundary under test

The trusted `main` WebView can invoke only `run_sqlite_m1_probe` with no
arguments. The native probe creates a disposable file database, applies schema
versions 1 and 2, rejects a separately executed future-schema startup, closes
and reopens between migration passes, checks the exact source rows and
reader/writer behavior with a 250 ms busy timeout, and evaluates the fixed
self-authored fixture in
[`fixtures/korean-fts-v1.json`](fixtures/korean-fts-v1.json). Its origin,
license, byte size, and pinned hash are recorded in
[`fixtures/README.md`](fixtures/README.md).

Queries of three or more Unicode scalar values use the FTS5 `trigram`
tokenizer. One- and two-scalar queries use a parameterized `LIKE` fallback
whose wildcard characters are escaped and whose `ORDER BY id LIMIT 64` bounds
the returned rows. The fixture
also checks literal `%`/`_` handling and a SQL-shaped input. A success receipt
contains only bounded evidence, SQLite build metadata, the fixture hash, and
cleanup status.

## Run checks

From this directory:

```sh
npm ci
npm test
npm audit --audit-level=moderate
npm run check
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
```

Launch the native app with:

```sh
npm run tauri dev
```

A unit test or compile proves only source behavior. Physical-platform matrix
evidence still requires an exact build and runtime record on that platform.
