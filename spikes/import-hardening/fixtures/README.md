# Import-hardening fixture catalog

`import-cases-v1.json` is the pinned catalog for the disposable M-1 import-hardening probe. The Rust host generates every source byte deterministically at runtime so the repository does not carry copied character cards or opaque malicious archives.

- Origin: self-authored for LorePia.
- License: CC0-1.0.
- Catalog bytes: 4,236 bytes, including the final LF.
- Catalog SHA-256: `484a313423d4e91c792818fb64097d96f8efb7c4a31befe96a1d3f739bfe5eb2`.
- Cases: 26 in the committed order.
- Deterministic valid ZIP: 665 bytes, SHA-256 `485733d6f60763ef1e2e63b4595debc63500d2b67321ea0e4ffa3084b611dc0f`, 217 uncompressed bytes.
- Deterministic direct PNG: 70 bytes, SHA-256 `ff36b8831e688e8fb5a511d916e82621821f67ce1c1c8ee204c395702c5a1a04`, 1x1 RGBA.
- Scope: container, path, resource-bound, PNG structure, staging, and Store-Safe inert-code defenses only.
- Excluded: Character Card V2/V3 semantic conversion, product import limits, database writes, backup restore, document-picker behavior, and imported code execution.

Changing the catalog requires updating the SHA-256 pinned by both the Rust and TypeScript contracts.
