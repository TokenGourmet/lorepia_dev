# M-1 import-hardening vertical boundary

This record defines the disposable archive/PNG defense spike. It is a source
and evidence contract, not a five-OS runtime `PASS`. Hosted tests, compilation,
a simulator, or an emulator cannot replace the physical-platform file-import
cells in [`verification-matrix.md`](verification-matrix.md). In particular, the
mobile cells require the real system document picker on physical devices.

The spike answers one narrow question: can a small native core reject the
required hostile container, path, size, PNG, and imported-code cases while its
fixed outside sentinel remains byte-for-byte unchanged and no per-case staging
state remains? It does not implement Character Card V2/V3 semantics, conversion
reports, database insertion, backup restore, or product import limits. Those
remain M3 and M6 work.

## Command and fixture boundary

The Tauri surface is exactly one no-argument command,
`run_import_hardening_m1_probe`. The WebView cannot provide a path, URI, ZIP,
PNG, logical entry name, limit, staging directory, or expected result. Rust
generates the deterministic self-authored cases named in
[`import-cases-v1.json`](../../spikes/import-hardening/fixtures/import-cases-v1.json),
whose origin and `CC0-1.0` permission are recorded in the adjacent
[`README.md`](../../spikes/import-hardening/fixtures/README.md). Both native and
frontend contracts pin the catalog SHA-256.

The generated accepted ZIP is exactly 665 bytes with SHA-256
`485733d6f60763ef1e2e63b4595debc63500d2b67321ea0e4ffa3084b611dc0f`
and 217 total uncompressed bytes. The accepted 1x1 RGBA PNG is exactly 70
bytes with SHA-256
`ff36b8831e688e8fb5a511d916e82621821f67ce1c1c8ee204c395702c5a1a04`.
Rust regenerates and compares those values before returning success; the
frontend parser independently requires the same values.

Expected negative results are entries in the successful proof receipt. A probe
harness failure returns only a protocol version, fixed error code, and
`cleanupPending`; neither path names, attacker strings, source bytes, nor native
error text cross IPC. Success is likewise fixed metadata and booleans, and the
serialized response is tested against a 4096-byte ceiling. A process-wide
non-blocking lock returns `PROBE_BUSY` for a concurrent lifecycle.

## Reproducible probe limits

These values are deliberately small M-1 test-policy values. They are not
LorePia product limits and must not be copied into the M3 importer by
implication.

| Resource | Exact ceiling |
|---|---:|
| Source bytes | 2,097,152 |
| Archive entries | 32 |
| One uncompressed entry | 524,288 bytes |
| Total uncompressed entries | 1,048,576 bytes |
| Entry and aggregate compression ratio | 100:1 |
| Logical path / component / depth | 240 bytes / 64 bytes / 8 components |
| Streaming scratch buffer | 16,384 bytes |
| PNG bytes / chunks / one chunk | 524,288 / 64 / 262,144 bytes |
| PNG width / height / pixels | 2,048 / 2,048 / 4,194,304 |
| PNG decoded allocation | 16,777,216 bytes |
| Generated logical-name index | 16,384 bytes |
| Serialized IPC response | 4,096 bytes |

Archive metadata is checked before the first staging write: entry count,
individual and checked aggregate size, and both individual and aggregate
compression ratios. Bounded extraction then checks actual entry and aggregate
output, requires every actual entry length to equal its declared length, and
fails if either byte ceiling is crossed. Only Stored and Deflate are accepted;
overlapping entry data, encryption, unsupported compression, symlinks, and
non-regular entry types fail closed.

Before the `zip` dependency sees the archive, the probe walks the end record,
central directory, local headers, extra-field TLVs, and checked data ranges.
It rejects ZIP64, multi-disk archives, data-descriptor mode, prefixed or
self-extracting containers, malformed extra fields, reused or overlapping
local ranges, and any central/local name, flag, method, CRC, or size mismatch.
The dependency's parsed offsets and metadata are then cross-checked against
that bounded plan as defense in depth.
Nested archive extraction is not part of this vertical.

## Cross-platform logical path policy

The probe rejects rather than repairs ambiguous names. A logical name must be
valid UTF-8 and already NFC-normalized, use `/` separators only, remain within
the exact byte/depth ceilings, and contain no empty, `.`, `..`, NUL/control,
absolute, drive, UNC, colon, Windows-forbidden, trailing-dot/space, or reserved
device component. Reserved aliases include `CLOCK$`, `CONIN$`, `CONOUT$`, and
the `COM1`-`COM9`/`LPT1`-`LPT9` families including their superscript-digit
aliases. A deliberately conservative uppercase expansion, lowercase expansion,
then NFC key detects exact and selected cross-platform case aliases; this is a
probe policy, not a claim of complete Unicode caseless matching. File/directory
prefix conflicts such as `a` and `a/b` are rejected.

Logical names are never joined to a disk destination. Valid payloads stream to
host-generated object names, and a bounded index records logical-name-to-hash
metadata. The probe never calls a generic archive `extract` helper.

## PNG and Store-Safe policy

Direct PNG input and archive PNG entries use one validator. It checks the
signature, checked chunk boundaries, every CRC, `IHDR` first/exactly once with
length 13, only the recognized critical chunks, at least one `IDAT`, exactly
one zero-length terminal `IEND`, no trailing bytes, and the declared resource
ceilings. All ancillary chunks and chunk types with a lowercase reserved bit
fail closed in this disposable proof because Character Card metadata policy is
not frozen. The version-pinned PNG decoder then runs with checksum and
allocation limits before the payload can be published.

The accepted archive contains self-authored JavaScript and Lua markers. They
are hashed and quarantined as inert objects only. No JS/Lua runtime dependency,
evaluation, WebView injection, module activation, or callback exists in this
spike; the receipt requires two executable-looking entries and zero executed
entries. This enforces the Store-Safe JS/Lua-off decision without approving a
future execution model.

## Staging and cleanup lifecycle

For each case the native lifecycle is:

1. verify the fixed catalog and generate the source;
2. perform bounded source detection and full archive-metadata preflight;
3. create a fresh app-owned staging directory under the fixed probe root;
4. stream validated payloads to generated object names and create the bounded
   index;
5. rename the complete staging tree to an absent same-parent publish name;
6. reopen the index and every object to verify hashes; and
7. remove the published tree and prove no staging entry remains.

A fixed sentinel outside the staging tree is compared byte-for-byte after every
accepted or rejected case. Startup and final cleanup touch only the exact
probe-owned root and sentinel. If final cleanup also fails after an earlier
probe error, that earlier bounded code is preserved with `cleanupPending: true`.
A cleanup failure after otherwise successful work returns `CLEANUP_FAILURE`.

This lifecycle establishes ordinary-process behavior under the process lock.
It does not prove descriptor-relative no-follow safety against a hostile local
process racing the filesystem, Windows reparse-point resistance, crash/power
loss durability, secure erase, or a hard wall-clock CPU kill. Those require
separate platform-specific implementation and evidence before production use.

## Current executable cases

The pinned catalog covers a valid inert archive and direct PNG, source and
format rejection, parent/absolute/Windows traversal forms, non-NFC and reserved
names, exact/case/prefix collisions, symlink entry, unsupported compression,
entry-count/entry-size/total-size/compression-ratio bounds, malformed archive,
bad/truncated/trailing/oversized PNG structures, and an unsupported file type.
Changing a case, order, expected outcome, or error code changes the pinned
catalog hash and must update both native and TypeScript contracts.

Additional native mutation tests exercise the strict container boundary,
including ZIP64 and multi-disk markers, data-descriptor flags, prefixed
containers, malformed extras, reused and overlapping ranges, central/local
header divergence, ancillary PNG chunks, reserved-bit chunk names, portable
case aliases, and expanded Windows device aliases. These tests are source-level
evidence; they do not add cases to the 26-case runtime receipt.

## Qualifying platform evidence

A physical-platform record must add the normal M-1 evidence fields plus the
exact catalog hash, serialized receipt, source hashes, picker/open path used,
sentinel and cleanup results, configured limits, and raw artifact/log link. A
qualifying runtime must select or open an approved fixture whose exact bytes,
SHA-256, and provenance are retained with that evidence, then run the same
native core. Android and iOS require the real system document picker on a
physical device.

The current no-argument synthetic command proves neither picker behavior nor
platform URI/path access. Desktop CI may establish source tests and native
compilation; Android CI builds an ARM64 APK, and iOS CI builds an ARM64
simulator target. Those remain compile evidence and do not change any physical
File import or Archive/import hardening cell.

No visual design, animation, or product interaction pattern is part of this
vertical.

## Upstream references

- [`zip` 8.6.0 archive API and path warnings](https://docs.rs/zip/8.6.0/zip/read/struct.ZipFile.html)
- [`ZipArchive` overlap and aggregate-size inspection](https://docs.rs/zip/8.6.0/zip/read/struct.ZipArchive.html)
- [`png` 0.18.1 decoder limits and checksum configuration](https://docs.rs/png/0.18.1/png/struct.Decoder.html)
- [`unicode-normalization` NFC support](https://docs.rs/unicode-normalization/0.1.25/unicode_normalization/)
- [`flate2` Rust backend](https://docs.rs/flate2/1.1.9/flate2/)
