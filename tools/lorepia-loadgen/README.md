# lorepia-loadgen

`lorepia-loadgen` creates deterministic, bounded load fixtures and evidence receipts for LorePia.
It is a developer/release-audit tool, not product code, and never accepts provider credentials or
raw prompts.

## Commands

Run large jobs with `--release`:

```sh
cargo run --release -p lorepia-loadgen -- db \
  --messages 1000000 --size 10GiB --branch-profile comb --seed 42 \
  --output /safe/new/path/load.sqlite3 --receipt /safe/new/path/db-receipt.json

cargo run --release -p lorepia-loadgen -- assets \
  --count 100000 --total 100GiB --duplicate-rate 0.9 --seed 42 \
  --output /safe/new/path/assets --receipt /safe/new/path/assets-receipt.json

cargo run --release -p lorepia-loadgen -- stream \
  --requests 129 --ack-profile never --seed 42 \
  --output /safe/new/path/stream.jsonl --receipt /safe/new/path/stream-receipt.json

cargo run --release -p lorepia-loadgen -- verify \
  --db /safe/path/load.sqlite3 --objects /safe/path/assets --full \
  --output /safe/new/path/verify-receipt.json

cargo run --release -p lorepia-loadgen -- bench \
  --db /safe/path/load.sqlite3 --objects /safe/path/assets --seed 42 \
  --warmup 3 --iterations 30 --output /safe/new/path/bench-receipt.json
```

Every output uses exclusive creation. Existing files, directories, and symlinks are rejected;
the tool never overwrites a target. DB and asset generation run an overflow-checked free-space
preflight before creating output.

## Size semantics

- `db --size` is the exact sum of UTF-8 message text bytes. The receipt reports a zero-byte
  tolerance plus the physical DB/WAL/SHM size. Each message remains within the product schema's
  1 MiB limit. Inserts are committed in bounded batches and use the current product schema.
- `assets --total` is the exact active byte total after CAS deduplication. `--count` is the number
  of ingest attempts, so duplicates still exercise the real ingest and reference paths. The
  receipt reports the rounded attainable duplicate count/rate and logical bytes read.
- Sizes accept `B`, `KiB`, `MiB`, `GiB`, `TiB`, or decimal `KB`, `MB`, `GB`, `TB`.

## Evidence boundary

`stream` emits a deterministic concurrency/ACK/byte-reservation schedule. Every JSONL artifact
contains `evidenceClass: MODEL_SCHEDULE_ONLY`, `runtimeEvidence: false`, and
`warning: NOT_TAURI_RUNTIME_EVIDENCE`. It is useful for 128/129 admission and ACK delay/drop
scenario construction, but it cannot satisfy a Tauri runtime or physical-device gate.

`bench` records warmups, iterations, p50/p95/p99/max, query plans, sampled RSS when available,
DB/WAL/SHM/FTS/object sizes, OS/toolchain, full commit, dirty state, and Cargo.lock digest. A dirty
or unknown worktree makes `releaseEvidenceEligible` false; a benchmark receipt never silently
claims release PASS.

## Verification

Quick verification runs SQLite `quick_check`, foreign-key checks, FTS terminal-row reconciliation,
active-path invariants, stream journal ordering, asset ledger reconciliation, and residual staging
checks. `--full` upgrades to `integrity_check`, runs FTS5's external-content integrity command,
hashes every active CAS object, and rejects missing, untracked, symlink, or non-regular entries.

The 100k planning test is metadata-only and does not allocate 100k assets. Million-message and
multi-gigabyte materialization are manual evidence jobs and intentionally stay out of hosted CI.
