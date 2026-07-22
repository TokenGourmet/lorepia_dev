# Fixture provenance

## `korean-fts-v1.json`

- Origin: self-authored synthetic Korean text created specifically for the
  LorePia M-1 SQLite/FTS5 probe on 2026-07-19.
- External source material: none. No third-party application source, card,
  conversation, or copyrighted prose was copied.
- Permission and license: dedicated to the public domain under `CC0-1.0` for
  unrestricted fixture reuse.
- Byte size: `1698 bytes` (UTF-8, including the final newline).
- SHA-256:
  `b5e8b2f2fdcf40d33dbb5eca555c982700e3cc1559dfe3adc878d85e2380b674`
- Purpose: deterministic Korean trigram, one/two-character literal fallback,
  wildcard escaping, and SQL-shaped literal-input regression checks.

The frontend and Rust probe both pin the hash and ordered golden result IDs.
Changing any fixture byte requires updating those contracts in the same
reviewed change; a syntactically valid but different fixture must fail closed.
