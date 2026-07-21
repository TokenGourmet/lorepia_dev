# Product asset-store status boundary

This record covers one deliberately read-only WebView product command. It wires
the bounded `lorepia-assets` catalog into the Tauri shell without exposing asset
ingestion, object reads, export, cleanup, reconciliation, import, backup, or
lorebook behavior. “Read-only” describes the caller-visible IPC action, not a
promise of zero internal filesystem writes: the first native `AssetStore::open`
may create its owned directories and catalog, apply a supported schema migration,
and recover an interrupted quarantine intent. None of those targets or policies
is caller-selectable.

## State and command ownership

Tauri manages one `AssetStoreState`. Setup records only the native-owned
`<app-local-data>/assets` path. It does not open or audit the catalog. The first
`get_asset_store_status` call opens the store and reads its aggregate statistics
on a blocking worker, so the catalog's O(N)-in-rows startup audit never runs on
the setup, WebView, or main thread.

The command has no caller-controlled input. In particular, the WebView cannot
supply a filesystem path, asset hash, owner, MIME type, quota, page size, source
bytes, or maintenance cursor. The capability remains bound exactly to the
trusted `main` WebView and gains no filesystem or dialog plugin permission.

## Fixed product limits

The native adapter owns these version-1 limits:

| Resource | Ceiling |
| --- | ---: |
| One object | 1,073,741,824 bytes |
| Total active objects | 9,223,372,036,854,775,807 bytes |
| Image width | 16,384 pixels |
| Image height | 16,384 pixels |
| Image area | 67,108,864 pixels |

The total is the signed-64-bit SQLite catalog arithmetic ceiling, not a storage
target or a promise that a device can hold that much data. Filesystem capacity
and free space remain the practical ceiling. The one-object guard admits the
1 GiB validation target, while this read-only slice neither exercises nor claims
large-store product support.

IPC cannot override these values. The response contains only contract/schema
versions, availability, the fixed limits, a fixed allowlisted error code, and
aggregate catalog counters. Counts and byte totals are decimal strings so they
cannot lose precision in a JavaScript number. The fully serialized response is
limited to 4,096 bytes and contains no native path or underlying I/O, SQLite, or
filesystem error text.

## Deliberately absent

This slice adds no `lorepia-import`, `lorepia-backup`, or `lorepia-lorebook`
dependency to the product app and no command from those crates. It does not
return object hashes or paths, read object bytes, create asset references, run
mark/sweep, or establish a product character/card ownership schema. Those are
separate product-coordinator decisions and cannot be inferred from a healthy
asset status response.

Tests cover lazy opening, the exact native root, fixed limits, decimal counters,
future-schema and corrupt-catalog closure, the 4,096-byte ceiling, path/error
redaction, and the exact command/capability set. Source tests and compilation are
not five-OS runtime or large-store evidence.
