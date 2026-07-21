# LorePia asset store

This crate owns immutable, content-addressed media and its SQLite catalog. The catalog is the
authority for normal startup and cleanup; walking the complete object directory is never a startup
operation.

## Filesystem boundary

- On Unix, the root, object shards, staging directory, and quarantine directory are opened as
  directory descriptors. File creation, open, link, unlink, directory enumeration, and quarantine
  moves are relative to those descriptors and use no-follow opens. A later pathname or symlink swap
  therefore cannot redirect an operation outside the already-open boundary. A root pathname that no
  longer identifies the opened root fails closed before a new catalog connection is opened.
- On Windows, reparse points are rejected and directory/file volume plus file identities are checked
  before use and after path-based opens. If Windows cannot provide those identities, the operation
  fails closed.
- Other non-Unix targets have no claimed no-follow contract and refuse to open the store until an
  equivalent stable directory identity implementation exists.

User-provided names never choose storage paths. Object locators come only from validated SHA-256
hashes; internal staging and quarantine names use a restricted ASCII alphabet.
SQLite main, journal, WAL, and shared-memory paths are rejected when they are symlinks or Windows
reparse points, and the bundled SQLite connection uses its no-follow open flag.

## Catalog validation and startup cost

Schema version 4 compares every `sqlite_schema` row (tables, explicit and implicit indexes, and
triggers, including exact SQL) with a canonical reference schema. Exact version 1, 2, and 3
catalogs are migrated in order. The v2-to-v3 migration releases legacy backup references because
they have no lease proving that their export is still resumable. The v3-to-v4 migration converts
legacy import references to expired sessions so startup can durably remove their catalog rows and
object files. Snapshot/restore validation applies the same exact-schema check.

Every catalog open and snapshot validation recalculates object count, active bytes, references,
missing objects, quarantine rows, and staging rows from their authoritative tables and compares them
with `asset_totals`. This is **O(N) in catalog rows and DB-only**. It deliberately catches a forged or
stale quota ledger before admission decisions. Startup also processes only the normally tiny set of
durable quarantine intents. It does **not** scan the object tree; filesystem discovery remains an
explicit, bounded shard-reconciliation API.

The `a_hundred_thousand_catalog_rows_do_not_trigger_an_object_tree_scan_on_startup` debug test
measured 32.6 ms for the `AssetStore::open` portion on the 2026-07-20 arm64 macOS development
host. This is evidence of the DB-only path, not a latency SLA; storage speed, SQLite build mode, and
row distribution will change the number.

Backup snapshot references are tied to a durable, renewable 24-hour lease. At most 128 sessions
can be live; store open and snapshot creation each run one complete, bounded stale-session pass.
Exporters renew before every 512-object page and before long verification phases. Cancellation can
therefore resume while its lease is current, explicit abandon releases immediately, failed catalog
snapshot creation rolls back immediately, and an expired partial is restarted from a fresh cut.

Temporary import ownership uses a separate durable 24-hour lease with the same 128-session bound.
The lease is registered before its staging directory is created, renewed at bounded admission
steps, and removed in the same transaction that promotes the exact expected set of unique hashes.
Opening another importer preserves live sessions; expired or legacy orphan sessions are rolled back.
Catalog snapshots strip both backup and import operational sessions and their temporary references.

## Crash ordering

- Staging cleanup closes and unlinks the temporary file, syncs the staging directory, and only then
  deletes its catalog recovery row. A failed unlink or directory sync leaves the row for retry.
- Quarantine move and purge operations persist an intent before changing the filesystem, sync the
  affected directories, persist the moved phase, and atomically finalize catalog state. Startup
  deterministically rolls forward every recorded phase. If neither side of a move exists, or both
  sides are different files, startup fails closed instead of guessing.

Tests inject failures at every durable quarantine boundary and between staging directory sync and
row deletion. A 100,000-row fixture verifies the DB audit without permitting an object-tree scan.
