# lorepia-backup

Product backup/restore is a versioned directory package, never a monolithic ZIP:

```text
manifest.json
manifest.sha256
progress.json
receipts/compatibility.json
data/product.sqlite3
data/assets/assets.sqlite3
data/assets/objects/aa/bb/<64-lowercase-hex-hash>
```

The product database uses SQLite's Online Backup API. Asset export briefly pins all active
catalog rows, snapshots that catalog, and then streams immutable hash-addressed objects while
normal asset additions/reference deletions continue. Additions after the catalog snapshot are
excluded; snapshot pins prevent deletion of included objects until final verification.
Pins use a renewable 24-hour lease and a bounded 128-session maintenance contract. Export renews
the lease at object-page and verification boundaries. Cancellation remains resumable while current;
`abandon_export` releases pins and deletes the partial immediately; permanent validation failures do
the same automatically; stale partials are discarded and restarted from a fresh snapshot. A failed
catalog snapshot never retains pins. Operational backup references from any session are removed
from the exported catalog so restore cannot create immortal ghost references.

The product database snapshot and the asset-catalog snapshot are currently two sequential cuts,
not one cross-database atomic cut. Object/ref consistency inside the asset catalog is verified, but
an owner mutation spanning the product and asset databases can still race between those cuts.
`BACKUP-010` therefore remains unclaimed until a product-wide mutation generation or a proven
cross-store snapshot coordinator exists; the manifest text documents the order and is not treated
as proof of same-instant consistency.

Both export and restore journals are fsync'd after every phase. Export resumes from the last
verified hash cursor. Restore writes a sibling staging directory, validates every size/hash and
both SQLite databases, then uses same-filesystem renames. Explicit replacement renames old data
aside before publishing new data, so crashes in either rename window can roll forward or roll
back without overwriting the old tree in place.

Journal, checksum, receipt, and manifest replacement uses a randomized same-directory temporary
file opened with exclusive-create semantics, fsync, and atomic platform replacement. Predictable
`.tmp` paths are never opened or truncated, including when an attacker pre-creates a symlink.

Manifest processing is deliberately bounded and fail-closed. Version 1 accepts and emits at most
32 MiB of canonical manifest JSON, 100,004 entries (100,000 asset objects plus the two databases
and two receipts), and 16 MiB of aggregate UTF-8 entry paths. Export serializes and hashes the
manifest directly to disk. Restore checks the on-disk byte count first, then performs a zero-copy
entry/path-budget pass before allocating owned entries; package tree enumeration and subsequent
validation collections are capped by the same contract. Oversized declared or actual manifests
are rejected instead of being loaded opportunistically.

The extreme-plan target of one million assets is therefore **not met by format v1**. A package at
that scale fails closed. Claiming the one-million-asset GO requires a later sharded or genuinely
streaming manifest format plus mobile runtime evidence; raising these caps alone is not evidence.
The snapshotted catalog count is checked before any object copy, with a second defensive guard in
the copy loop, so an oversized catalog cannot consume one million-file copy work before rejection.

Normal tests use injected accounting instead of pretending to allocate 110 GiB. The real
`10 GiB database + 100 GiB assets` case is an explicit ignored load gate:

```sh
cargo test -p lorepia-backup real_10gib_database_100gib_assets_load_gate \
  -- --ignored --exact
```

Provision the real fixture and replace the guard test in dedicated load infrastructure before
recording BACKUP-002 as runtime PASS. Portable path/hash tests are source-level evidence only;
Windows, Android, and macOS runtime restore receipts remain separate release evidence.
