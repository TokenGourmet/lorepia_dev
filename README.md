# LorePia

LorePia is a local-first, cross-platform AI character chat client in the M-1 risk-removal phase. The current repository contains disposable vertical spikes used to prove or reject the architecture in [`LorePia_기술계획서_v2.md`](LorePia_기술계획서_v2.md); it is not yet the product application.

The current spikes exercise mock SSE-to-Tauri-Channel streaming, an independent
five-OS credential-store lifecycle, a file-backed SQLite/FTS5 lifecycle, a
bounded archive/PNG import-hardening lifecycle, and a constrained Lua 5.4
runtime. They are intentionally functional and minimal. Product UI, visual
design, and animation are outside the implementation scope here and remain
owner-authored work.

## Current scope

- Prove the Tauri 2 + Rust + Svelte toolchain on Windows, macOS, Linux, Android, and iOS.
- Verify Channel sequencing, batching, cancellation, backpressure, and partial-result behavior.
- Verify that OS credential services can complete a native-only
  absent/create/read/update/delete lifecycle without exposing secret material to
  WebView IPC.
- Verify SQLite migration, reopen persistence, WAL read/write behavior, and
  deterministic Korean substring search without freezing the M1 product schema.
- Verify bounded ZIP/PNG handling, cross-platform path rejection, inert imported
  scripts, staged publication, and exact cleanup without freezing the M3 card
  importer or product limits.
- Verify that a fixed diagnostic Lua corpus is bounded by instruction, deadline,
  memory, and standard-library policy without enabling imported Lua.
- Record runtime evidence without treating compilation, a simulator, and a physical device as equivalent.
- Keep imported JavaScript and Lua disabled in the Store-Safe profile until written policy clearance and the required isolation evidence exist.

No 5-OS runtime support claim is valid until the [M-1 verification matrix](docs/m1/verification-matrix.md) contains the required evidence.

## Repository layout

```text
.
├── docs/m1/                    # M-1 gates, procedures, and evidence matrix
├── spikes/channel-stream/      # Disposable Channel vertical spike
├── spikes/keychain/            # Disposable five-OS credential-store spike
├── spikes/sqlite-fts/          # Disposable SQLite/FTS5 vertical spike
├── spikes/import-hardening/    # Disposable archive/PNG defense spike
├── spikes/lua-limits/          # Disposable Lua 5.4 limit-enforcement spike
├── .github/workflows/m1.yml    # Desktop and mobile compile verification
└── LorePia_기술계획서_v2.md    # Current technical plan
```

## Run and check a spike

Install the current [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
for your OS. The example below runs the Channel spike; use the same locked
check sequence from the other spike directory named below.

```sh
cd spikes/channel-stream
npm ci
npm test
npm audit --audit-level=moderate
npm run check
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
npm run tauri dev
```

The keychain spike uses the same check sequence from `spikes/keychain`. Its one
functional button runs the native lifecycle probe; it is not a password or API
key management UI. Platform behavior and evidence limits are in
[`docs/m1/keychain.md`](docs/m1/keychain.md).

The SQLite/FTS5 spike uses the same check sequence from `spikes/sqlite-fts`,
followed by `npm run tauri dev` for a local runtime attempt. It proves only the
bounded database/search contract in [`docs/m1/sqlite-fts.md`](docs/m1/sqlite-fts.md);
the full branching chat schema and product database API remain M1 work.

The import-hardening spike uses the same sequence from
`spikes/import-hardening`. Its one no-argument probe generates a self-authored
positive/negative corpus and returns bounded proof metadata. It does not expose
a product file picker or define Character Card conversion; see
[`docs/m1/import-hardening.md`](docs/m1/import-hardening.md).

The Lua limit spike uses the same sequence from `spikes/lua-limits`. Its one
no-argument diagnostic probe runs only a fixed self-authored corpus; it neither
accepts imported Lua nor defines the product scripting API. See
[`docs/m1/lua-limits.md`](docs/m1/lua-limits.md).

`rust-toolchain.toml`, `Cargo.lock`, and `package-lock.json` are application inputs and must be committed. CI uses the pinned Rust toolchain and lockfiles and must not silently refresh dependencies.

## M-1 evidence

Start with [`docs/m1/README.md`](docs/m1/README.md). A green desktop CI job proves source formatting, tests, type checking, and compilation on that runner only. An Android APK build or iOS simulator build is compile evidence only. Neither is physical-device smoke evidence.

## Data boundary

LorePia has no operating server that stores or collects user content. When a user sends a message, the assembled prompt—including conversation, lorebook, and card content—is sent to the LLM provider selected and configured by that user. Other application data is intended to remain on the user's device.

## License

Copyright 2026 TokenGourmet. Licensed under the [Apache License 2.0](LICENSE); attribution notices are in [`NOTICE`](NOTICE).
