# Product storage layout (current state)

This document records the storage layout implemented in the current checkout.
It is not a proposal that silently reserves files or tables. In particular,
LorePia does **not** currently have `lorepia-derived.sqlite3`, one database per
chat, or one database per character.

## Files and authorities

The native product resolves all paths below Tauri's app-local data directory.
Callers do not provide these paths.

```text
<app-local-data>/
├── lorepia.sqlite3                 # product records, schema v3
├── lorepia.sqlite3-wal             # runtime WAL when present
├── lorepia.sqlite3-shm             # runtime shared-memory file when present
├── lorepia.sqlite3.lock            # exclusive process lease
└── assets/
    ├── assets.sqlite3              # authoritative asset catalog, schema v4
    ├── assets.sqlite3-wal          # runtime WAL when present
    ├── assets.sqlite3-shm          # runtime shared-memory file when present
    ├── objects/aa/bb/<sha256>      # immutable content-addressed objects
    ├── .staging/                   # crash-recoverable admission work
    └── quarantine/                 # rejected or recovery-owned objects
```

Authority is deliberately split by domain:

- `lorepia.sqlite3` is authoritative for chats, branching messages, selected
  paths, stream/request durability, non-secret app preferences, complete-message
  FTS, and the bounded HTML render cache.
- `assets/assets.sqlite3` is authoritative for asset object metadata,
  references, aggregate quota counters, staging/quarantine recovery state, and
  renewable backup/import sessions. Normal startup audits catalog rows but does
  not walk the complete object tree.
- `assets/objects/` owns the immutable bytes addressed by SHA-256. User-provided
  names never select object paths.
- API keys are not stored in either database. They remain in the operating
  system credential vault.

The product database is currently schema v3 with explicit v1-to-v2-to-v3
migrations. The asset catalog is currently schema v4 with explicit migration
from exact versions 1, 2, and 3. Both stores verify their expected schema and
reject unknown future versions.

## Transaction boundary

Each database provides atomic transactions only for the records it owns. The
application does not `ATTACH` the asset catalog to the product database, cannot
enforce a cross-file foreign key, and does not claim crash atomicity across the
two databases. Asset owner identifiers are logical references in the asset
catalog, not SQLite foreign keys into `lorepia.sqlite3`.

This is why the architecture does not describe either SQLite file as the one
source of truth for all product data. The two authorities and CAS objects must
be coordinated explicitly whenever a future product operation spans them.

## FTS and rebuildable data

The FTS5 index and renderer-versioned HTML cache currently live in
`lorepia.sqlite3`. Complete-message changes update FTS through product-database
triggers, preserving transaction-local consistency. The render cache has an
explicit size bound and eviction API.

There is no derived database today. A future `lorepia-derived.sqlite3` may be
considered only for data that can be rebuilt completely from authoritative
state, such as embeddings or selected caches. Splitting FTS or another cache
requires all of the following first:

- a durable outbox or equivalent source-revision stream;
- a persisted rebuild watermark and restart-safe rebuild;
- UI acceptance of eventual consistency;
- deletion of the derived file without data loss;
- measured improvement to latency, backup size, or maintenance cost.

Until those conditions are implemented and reviewed, the derived database is a
future option, not part of the filesystem or backup contract.

## WAL and compaction

Both SQLite authorities use WAL independently. A WAL or SHM file may exist
during normal operation, and one database's checkpoint says nothing about the
other database. The product store has bounded periodic checkpoint telemetry and
escalation; the asset catalog has its own connection and WAL lifecycle. Neither
database is placed on a network filesystem.

Deleting rows makes pages reusable but does not promise immediate shrinkage of
the database file. The current product exposes no automatic full-`VACUUM` or
incremental-vacuum contract. Any future compact operation must be explicit,
cancellable where possible, free-space preflighted, and justified by measured
freelist and file-size evidence rather than run after every deletion.

## Backup v1

The implemented backup is a versioned directory package at a caller-selected
destination, not a ZIP and not an assumed `backups/` child of app-local data.

```text
<backup-package>/
├── manifest.json
├── manifest.sha256
├── progress.json
├── receipts/compatibility.json
├── data/product.sqlite3
├── data/assets/assets.sqlite3
└── data/assets/objects/aa/bb/<sha256>
```

`data/product.sqlite3` is produced with SQLite's Online Backup API. The asset
export pins the active object set, snapshots `assets.sqlite3`, then copies and
verifies the immutable objects in bounded pages. Journals make interrupted
export and restore resumable while their leases remain valid; restore validates
hashes and both schemas in a sibling staging directory before same-filesystem
publication.

The two database snapshots are taken in this order:

1. product database;
2. pinned asset catalog.

They are sequential, non-atomic cuts. Pins prevent deletion of objects selected
by the asset-catalog snapshot, but they cannot prove that product rows and asset
references represent one instant. `BACKUP-010` remains unclaimed until a
product-wide mutation generation or proven cross-store snapshot coordinator is
implemented. Backup format v1 also caps represented assets at 100,000 objects;
the one-million-asset target fails closed rather than being claimed.

## Growth and future sharding

Raw database size alone is not a sharding trigger. A 10 GiB fixture must be
measured for query and chat-entry p95, checkpoint behavior, backup/restore time,
migration time, integrity checking, and temporary free-space needs. Missing
indexes, offset pagination, long transactions, uncontrolled BLOB reads, and WAL
starvation should be corrected before introducing more database files.

Do not create one database per chat or character. If measurements eventually
justify physical sharding, prefer a profile/workspace boundary that keeps data
requiring atomic mutation together. That decision also requires explicit
migration, backup manifests, fan-out search limits, and recovery behavior.

## Source contracts and evidence

- Product schema and migrations: [`lorepia-storage`](../../crates/lorepia-storage/src/migration.rs)
- Asset authority and CAS: [`lorepia-assets`](../../crates/lorepia-assets/README.md)
- Backup package and open limitations: [`lorepia-backup`](../../crates/lorepia-backup/README.md)
- Product IPC boundary: [Product SQLite storage](../m0/product-storage.md)

Useful upstream constraints are documented by SQLite's
[limits](https://www.sqlite.org/limits.html),
[`ATTACH DATABASE`](https://www.sqlite.org/lang_attach.html),
[WAL](https://www.sqlite.org/wal.html), and
[Online Backup API](https://www.sqlite.org/backup.html) documentation.

Host tests establish source-level behavior only. Physical Android/iOS runtime,
packaged Windows/Linux runtime, large real-fixture performance, and
cross-database atomic backup remain separate evidence gates.
