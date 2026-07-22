# Product SQLite storage

This record describes the current product-owned persistence slice. It is an
implementation contract and local test record, not physical-device evidence.
The complete current filesystem and cross-store ownership map is recorded in
the [product storage layout](../architecture/product-storage-layout.md).

## Ownership and command boundary

The native application opens `lorepia.sqlite3` under Tauri's app-local data
directory. One process owns the database lease; a second app instance fails
closed instead of treating the first process's live request as abandoned.
SQLite runs with foreign keys, WAL mode, and a bounded busy timeout.

The trusted main WebView receives only typed commands for storage status,
creating/listing/deleting chats, loading messages, and reading/updating app
preferences. Branch selection, request journaling, and render-cache APIs remain
native storage capabilities; they are not direct WebView commands. There is no
raw SQL, arbitrary path, migration, credential, asset-object, or backup command.

The asset store is a separate authority. Its catalog is
`assets/assets.sqlite3`, and its immutable content-addressed objects live below
`assets/objects/`. The product shell currently exposes only the bounded
asset-status command described in [Product asset-store status
boundary](product-assets.md). API keys remain exclusively in the OS credential
vault.

## Product schema v3

The current product database schema is version 3. Startup has reviewed forward
migrations from exact v1 and v2 schemas; an unknown future version, an
unversioned non-empty database, or a modified schema fails closed.

Schema v3 stores:

- chats with a character ID, title, timestamps, and optimistic revision;
- branching user and assistant messages with parent, sibling, depth, and
  completion state;
- the selected active message path for each chat;
- native-owned provider request progress, delivery/durability/ACK sequence
  state, and cumulative usage;
- selected provider, per-provider model IDs, theme, and default chat mode;
- an FTS5 trigram index of complete messages;
- a bounded, renderer-versioned HTML cache.

The product database has no API-key, raw provider-error, asset BLOB, asset
catalog, embedding, or separate derived-database table. Provider error bodies
are discarded by the runtime; storage receives only bounded app-owned failure
codes. Schema evolution requires an explicit migration and exact post-migration
schema validation.

## Streaming and branching durability

Starting a turn inserts the user message, empty assistant message, selected
branch path, and running request state transactionally. The bridge checkpoints
visible text/refusal, provider response identity, delivery/durability/ACK
progress, and cumulative usage at bounded intervals. Reasoning text,
credentials, control tokens, and raw remote errors are not persisted.

The final checkpoint and complete/cancelled/failed state transition are atomic
inside `lorepia.sqlite3`. Only after that transaction succeeds may the matching
terminal Channel event be sent. On startup, any remaining running request is
marked interrupted with the stable `APP_RESTARTED` code while its last committed
partial response remains available.

Branch mutations and active-path selection are also product-database
transactions. This protects chat/message/request invariants inside the product
database; it does not create atomic transactions with the separate asset
catalog.

## WAL lifecycle and bounded access

The app serializes product storage work through a native admission gate and
does not time out a mutation after it has begun. Reads use validated cursors and
bounded pages. The current first-chat loader scans at most 10,000 chats and
restores at most 10,000 messages or 16 MiB of message text before failing
closed.

Product WAL maintenance runs every 60 seconds. It records passive-checkpoint
telemetry, attempts `RESTART` after 64 MiB of frame payload, and permits an
emergency `TRUNCATE` only after the restart proves there is no blocking reader
and no remaining frame, with a 512 MiB emergency threshold. WAL and SHM files
are runtime state, not corruption by themselves.

## Backup boundary

The repository contains a version-1 directory-package backup engine for the
product database, asset catalog, and content-addressed objects. It does not
create a monolithic ZIP and is not currently exposed as a product WebView
command.

The product database snapshot and asset-catalog snapshot are sequential cuts,
not one cross-database atomic snapshot. Asset pins protect the exact objects in
the catalog cut, but a product mutation that spans both authorities may race
between the two snapshots. `BACKUP-010` therefore remains unclaimed. See the
[storage layout](../architecture/product-storage-layout.md#backup-v1) and the
[`lorepia-backup` contract](../../crates/lorepia-backup/README.md).

## Verification boundary

Workspace tests cover v1-to-v3 migration, schema initialization and tamper
rejection, WAL reopen and maintenance behavior, lease/concurrency behavior,
preferences conflicts, chat persistence and search, branch/active-path
invariants, render-cache bounds, stream checkpoint sequencing, terminal
atomicity, restart recovery, and online snapshots. Asset and backup crates have
their own catalog, CAS, snapshot, manifest, cancellation, and restore tests.

These source tests and host compilation do not establish Android/iOS
physical-device runtime behavior, packaged Windows/Linux runtime behavior, the
real 10 GiB database plus 100 GiB asset load gate, or cross-store snapshot
atomicity.
