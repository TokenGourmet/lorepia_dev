# lorepia-lorebook

Pure, UI-independent lorebook admission and selection. The crate emits opaque
text for `PromptCompileInput::lorebook`; dependency direction never runs from
the prompt compiler back into lorebook selection.

## Contract

- Catalogs contain at most 100,000 entries and have cumulative byte limits.
- Search receives a bounded recent tail (at most 512 turns), never an entire
  conversation. Failed messages are excluded. Partial-message inclusion is an
  explicit setting and defaults to exclusion in product adapters.
- Literal conditions use case-aware Aho-Corasick indexes. An imported regex
  `required_literal` is retained as bounded compatibility metadata but is not
  trusted as a correctness prefilter: every enabled entry containing regex is
  a candidate. Catalog-wide regex count/pattern-byte limits and per-selection
  evaluation, scan-byte, and match-count ceilings bound that fail-safe path.
- Text normalization is locale-independent `NFKC -> Unicode scalar lowercase
  (unless case-sensitive) -> Unicode whitespace collapse`. No stemming or
  language-specific word-boundary guesses are made.
- Regex uses Rust's linear-time engine with compile size limits plus cumulative
  per-selection scan-byte and match-count ceilings.
- Matching entries sort by priority descending, source scope (`chat`, card,
  global), order ascending, then ID ascending. Greedy budget admission is
  stable. `reserved_tokens` must already include any per-entry framing cost.
- Probability decisions are derived from a supplied seed and entry ID using
  SHA-256. There is no ambient randomness and catalog iteration order cannot
  change an entry's draw.
- Lore content is never expanded, parsed as a macro, assigned a role, or
  executed. Recursion is absent rather than merely depth-limited.
- Cache identity includes the internal catalog generation, public catalog
  revision, chat and branch revisions, exact bounded search slice, all
  selection settings, seed, tokenizer ID, and tokenizer revision. Catalog
  replacement rebuilds indexes and clears the cache.
- Imported JSON is a closed, versioned schema. The default import entry point
  disables every entry; preserving enabled flags requires explicit
  `ImportTrust::LocallyTrusted` authority.
- `Debug` and errors expose counts, revisions, limits, and short digests, never
  lore, prompt, key, regex, summary, or chat content.

Selection is synchronous CPU work. A Tauri adapter must execute it on a bounded
worker (`spawn_blocking`), not the WebView/main thread.

## Extreme-plan evidence mapping

The integration test names retain the source IDs from
`LorePia_Extreme_Test_Plan_2026-07-20.md`.

| IDs | Automated evidence | Boundary still outside this crate |
| --- | --- | --- |
| LORE-001..006 | 0/1/1k/10k normal tests; explicit metadata-only 100k test; all/none/overlap/order/key-bound/Korean normalization | device memory ceilings |
| LORE-007..011 | catastrophic-shaped regex corpus, compile/scan/match limits, invalid import, and proof that an incorrect imported hint cannot hide a real regex match | device-specific latency profiling |
| LORE-012..014 | macro/variable text remains byte-literal and output cap is enforced | none in the selector |
| LORE-015..016 | exact boundary over supplied token reservations; tokenizer revision changes cache identity | provider-specific tokenization must produce the reservations and still pass `lorepia-prompt`'s final exact-token gate |
| LORE-017..021 | bounded million-turn tail contract, missing/corrupt summary fallback, branch identity, partial/failed policies | SQLite tail query, summary corruption detection, and branch adapter integration |
| LORE-022..024 | scope/tie ordering, seeded byte determinism, log-safe Debug/errors | application logging audit |
| LORE-025 | 100k selective entries are indexed, one-key search yields one candidate, and the smoke test runs selection on a worker thread | Tauri adapter wiring and real UI/main-thread tracing |

Run the normal suite:

```sh
cargo test --locked -p lorepia-lorebook
```

Run the 100k metadata/index smoke explicitly (it writes no fixture):

```sh
cargo test --locked -p lorepia-lorebook \
  lore_001_and_025_one_hundred_thousand_entries_use_index_on_a_worker \
  -- --ignored --exact
```
