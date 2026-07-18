# LorePia M-1 import-hardening spike

This disposable Tauri 2 + Rust + Svelte probe exercises a fixed, self-authored
archive/PNG defense corpus. It has one no-argument native command and one plain
button. It intentionally contains no product UI styling, animation, file
picker, Character Card conversion, database integration, or imported-code
runtime.

The native probe verifies the catalog hash, generates all 26 source cases,
checks bounded ZIP and PNG parsing, rejects cross-platform path ambiguity and
unsafe entry types, quarantines JavaScript and Lua as inert bytes, publishes a
fully validated staging tree atomically within the process-owned probe area,
reopens the index and objects to recheck hashes, checks an outside sentinel,
and removes all owned state.
Expected hostile-input rejections are reported as evidence inside the bounded
success receipt.

## Run the locked checks

```sh
npm ci
npm test
npm audit --audit-level=moderate
npm run check
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
```

Run the local app with `npm run tauri dev`, then use the single probe button.
The app displays only fixed proof metadata and fixed error labels.

The full boundary, exact resource ceilings, evidence rules, and explicit
non-claims are in [`docs/m1/import-hardening.md`](../../docs/m1/import-hardening.md).
The catalog provenance and pinned hash are in [`fixtures/README.md`](fixtures/README.md).

A successful local or CI run does not prove platform document-picker behavior
or a physical-device File import matrix cell.
