# Secret sentinel artifact scanner

This developer/release tool scans explicit artifact roots for a known test
sentinel and common textual encodings without loading whole files. It rejects
symlinks and special files, detects matches split across 64 KiB chunks, and
writes a receipt containing only relative-path hashes, stable variant names,
offsets, and counts. The sentinel value and matching bytes are never written to
the receipt.

Keep the sentinel file outside every scan root:

```sh
node tools/secret-sentinel-scan/scan.mjs \
  --sentinel-file /private/tmp/lorepia-known-sentinel \
  --root /path/to/artifacts \
  --root /path/to/logs \
  --receipt /path/to/new/secret-scan-receipt.json
```

Exit status is `0` when no variant is found, `2` when a match is found, and `1`
for an invalid or incomplete scan. A passing receipt is useful only for the
exact roots listed by the invoking release job; it is not evidence for crash
dumps, device logs, exports, or binaries that were not supplied.
