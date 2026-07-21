# Asset/import extreme-test coverage

This is an implementation coverage map for `ASSET-001` through `ASSET-040` in
`LorePia_Extreme_Test_Plan_2026-07-20.md`. It does not turn source-level tests into a
five-OS or physical-device `PASS`.

Status vocabulary:

- **AUTOMATED**: a focused, deterministic local test exercises the named failure boundary.
- **PARTIAL**: a useful contract is automated, but the requested scale, crash point, corpus, or
  platform evidence is still missing.
- **REMAINS**: no qualifying automated evidence exists yet.

| ID | Status | Current evidence and exact residual |
|---|---|---|
| ASSET-001 | PARTIAL | Load generation covers 0/1 edges and a 100k metadata plan; real 100/10k/100k/1M object materialization has not run. |
| ASSET-002 | PARTIAL | CAS paths are deterministically sharded as `objects/aa/bb/hash` and verified by loadgen, but the 100k physical-object directory distribution has not run. |
| ASSET-003 | REMAINS | No 1M one-byte-file materialization; CAS also rejects unsupported arbitrary one-byte payloads. |
| ASSET-004 | REMAINS | Configurable streaming limits exist; 100MB/1GB/2GB single-object runs are absent. |
| ASSET-005 | REMAINS | Quota ledger and load generator exist; 10GB/100GB stores have not been generated. |
| ASSET-006 | PARTIAL | CAS duplicate ingestion and loadgen duplicate planning are automated; the exact 90% large-scale run is absent. |
| ASSET-007 | REMAINS | Corrupt content at a hash path is quarantined, but a test-only true hash-collision injection seam does not exist. |
| ASSET-008 | AUTOMATED | `ref_batches_are_transactional_and_mark_sweep_only_removes_orphans` covers object-without-ref cleanup. |
| ASSET-009 | AUTOMATED | `a_missing_referenced_object_is_retained_in_catalog_and_can_be_repaired` covers ref-without-object recovery. |
| ASSET-010 | AUTOMATED | Corrupt object content is detected, quarantined, and repaired by focused CAS tests. |
| ASSET-011 | AUTOMATED | CAS and product importer cancellation/error tests assert zero residual staging entries, active objects, bytes, and temporary references after a later-entry failure. |
| ASSET-012 | REMAINS | No process-kill injection immediately before/after atomic rename. |
| ASSET-013 | PARTIAL | Every admitted object and its UUID import-session reference commit together; final-owner promotion is one transaction. Rollback preserves deduplicated objects with another reference, deletes only session-orphaned active objects, and a post-unlink/pre-commit failure is recovered on reopen. A true process-kill matrix around every SQLite/filesystem boundary remains absent. |
| ASSET-014 | PARTIAL | Final source symlinks use `O_NOFOLLOW` on Unix, generated output names never join archive paths, and CAS symlink tests exist; adversarial swap-race stress is absent. |
| ASSET-015 | REMAINS | No Windows junction/reparse-point runtime test. |
| ASSET-016 | PARTIAL | Parent, nested parent, POSIX absolute, drive, backslash, colon, NUL/control, dot, and prefix forms are rejected; exhaustive platform separator mutation remains. |
| ASSET-017 | PARTIAL | Non-NFC and selected Unicode/case aliases are rejected with portable collision keys; exhaustive Unicode caseless tables remain. |
| ASSET-018 | AUTOMATED | Device families `CON/PRN/AUX/NUL/CLOCK$/CONIN$/CONOUT$/COM1-9/LPT1-9`, including superscript aliases, are tested. |
| ASSET-019 | AUTOMATED | ZIP64 EOCD/sentinel/extra-field forms are rejected before extraction. |
| ASSET-020 | AUTOMATED | Data-descriptor flags are rejected in bounded central/local preflight. |
| ASSET-021 | AUTOMATED | Reused/overlapping local-header offsets are rejected; data ranges are checked for overlap. |
| ASSET-022 | AUTOMATED | Nested `.zip` entries are rejected as unsupported file types and are never recursively opened. |
| ASSET-023 | AUTOMATED | A generated fixture asserts an actual ratio above 1000:1 and is rejected during central-directory preflight. |
| ASSET-024 | AUTOMATED | Central/local name, flags, method, CRC, size, offset, and data-start divergence fail closed. |
| ASSET-025 | AUTOMATED | Encrypted entry flags produce a stable rejection before payload access. |
| ASSET-026 | AUTOMATED | PNG signature, chunk bounds/order, CRC, required chunks, trailing data, and decoder checksum failures are tested. |
| ASSET-027 | AUTOMATED | A CRC-correct 100000x100000 PNG dimension bomb is rejected before decode allocation. |
| ASSET-028 | AUTOMATED | APNG `acTL/fcTL/fdAT` chunks are rejected; animated frame allocation is never entered. |
| ASSET-029 | AUTOMATED | Aggregate ancillary metadata is bounded before allocation; oversized EXIF and compressed ICC forms are rejected. |
| ASSET-030 | AUTOMATED | SVG entries, including active content, are not allowlisted and are never stored or rendered. |
| ASSET-031 | PARTIAL | CAS requires bounded magic contracts for PNG/JPEG/WebP/GIF; a broad malformed WebP/GIF/AVIF corpus is absent and AVIF is not allowlisted. |
| ASSET-032 | PARTIAL | CAS requires bounded WAV/MP3/Ogg/FLAC magic contracts; a broad malformed metadata corpus is absent. |
| ASSET-033 | AUTOMATED | `import_reader<R: Read>` succeeds without `Seek`; input is first copied through a bounded generated staging file. |
| ASSET-034 | PARTIAL | A reader returning `PermissionDenied` mid-copy is tested and cleans staging; physical Android/iOS picker revocation remains. |
| ASSET-035 | REMAINS | No physical removable-storage detach run. |
| ASSET-036 | REMAINS | Write errors fail closed, but a real ENOSPC injection during source/CAS streaming has not run. |
| ASSET-037 | AUTOMATED | Cancellation during source copy, extraction, and after the first successful CAS ingest returns `IMPORT_CANCELLED`; the latter rolls the UUID session back to zero active objects/bytes/refs. |
| ASSET-038 | AUTOMATED | Exactly one of 100 simultaneous requests is held in admission and the other 99 receive `IMPORT_BUSY`. |
| ASSET-039 | REMAINS | Thumbnail cache eviction/rebuild is outside the current CAS/import product slice. |
| ASSET-040 | AUTOMATED | A 100k-row restart test proves startup reads the catalog ledger without scanning the object tree. |

Focused reproducible gates:

```sh
cargo test --locked -p lorepia-import
cargo test --locked -p lorepia-assets
cargo clippy --locked -p lorepia-assets -p lorepia-import --all-targets -- -D warnings
```

The scale, process-kill, Windows reparse-point, removable-storage, ENOSPC, malformed multimedia
corpus, and physical-picker rows above remain release blockers until their own artifacts exist.
