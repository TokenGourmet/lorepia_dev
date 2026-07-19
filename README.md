# LorePia

LorePia is a local-first, cross-platform AI character chat client. The
repository now contains the first non-disposable M0 product scaffold alongside
the M-1 vertical spikes used to prove or reject risky architecture choices in
[`LorePia_기술계획서_v2.md`](LorePia_기술계획서_v2.md).

The product scaffold is not an M0 completion claim. It establishes one typed
`lorepia-core -> Tauri -> Svelte` startup path and an owner-authored first UI
slice while M-1 exit evidence, the plugin API freeze, physical mobile smoke,
and benchmark baselines remain open. See [`docs/m0/README.md`](docs/m0/README.md).

The current spikes exercise mock SSE-to-Tauri-Channel streaming, an independent
five-OS credential-store lifecycle, a file-backed SQLite/FTS5 lifecycle, a
bounded archive/PNG import-hardening lifecycle, and a constrained Lua 5.4
runtime. A sixth spike exercises a fixed local PCM WAV through the trusted
main WebView's `HTMLAudioElement` path. A seventh tests an independently
terminable QuickJS-WASM Worker boundary without sending source through Tauri
IPC. They are intentionally functional and minimal and remain separate from the
product UI and its owner-authored design system.

## Current scope

- Maintain a real root Rust workspace and a Tauri 2 + Svelte 5 product shell.
- Keep every product command capability explicit, bounded, and limited to the
  trusted main WebView.
- Keep product UI on the owner-authored tokens and screen components without
  weakening executable-content or native-command boundaries.
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
- Verify fixed-fixture load, play, pause, seek, resume, stop, release, and the
  foreground-only lifecycle policy through the trusted WebView audio path.
- Verify that a fresh QuickJS-WASM Worker can enforce bounded script execution,
  survive hostile fixtures, and be terminated externally without a Tauri
  command, Channel, or source-transport path.
- Record runtime evidence without treating compilation, a simulator, and a physical device as equivalent.
- Keep imported JavaScript and Lua disabled in every current product profile;
  reopening requires a new reviewed contract, technical boundary evidence, and
  any applicable store-policy clearance.

The current product decision is stricter than an evidence-pending toggle:
imported executable content remains disabled until a new reviewed contract
satisfies the independently terminable runtime and bounded-transport gates in
[`ADR 0001`](docs/decisions/0001-imported-code-execution.md).

No 5-OS runtime support claim is valid until the [M-1 verification matrix](docs/m1/verification-matrix.md) contains the required evidence.

## Repository layout

```text
.
├── Cargo.toml                  # Product Rust workspace
├── crates/lorepia-core/        # Platform-independent product core
├── crates/lorepia-providers/   # Provider options and wire compiler
├── crates/lorepia-credential-vault/ # Five-OS native credential storage
├── crates/lorepia-provider-runtime/ # Native HTTPS and stream decoding
├── crates/lorepia-tool-runtime/ # Deny-by-default tool/MCP policy contracts
├── apps/desktop-mobile/        # Tauri 2 + Svelte 5 product shell
├── docs/m0/                    # Product scaffold scope and open gates
├── docs/m1/                    # M-1 gates, procedures, and evidence matrix
├── docs/decisions/             # Product architecture decisions
├── spikes/channel-stream/      # Disposable Channel vertical spike
├── spikes/keychain/            # Disposable five-OS credential-store spike
├── spikes/sqlite-fts/          # Disposable SQLite/FTS5 vertical spike
├── spikes/import-hardening/    # Disposable archive/PNG defense spike
├── spikes/lua-limits/          # Disposable Lua 5.4 limit-enforcement spike
├── spikes/audio-playback/      # Disposable trusted-WebView audio spike
├── spikes/script-runner/       # Disposable terminable JavaScript runner spike
├── .github/workflows/product.yml # Product 5OS compile gates
├── .github/workflows/m1.yml    # Spike desktop and mobile compile verification
└── LorePia_기술계획서_v2.md    # Current technical plan
```

## Run the product scaffold

Install the current [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/),
then run:

```sh
cd apps/desktop-mobile
npm ci
npm test
npm run check
npm run tauri dev
```

The current owner-authored screen set is not yet wired to the headless provider
commands. The native implementation boundary is documented in
[`docs/m0/provider-runtime.md`](docs/m0/provider-runtime.md).

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

The audio spike uses the same sequence from `spikes/audio-playback`, with
`npm run verify:fixture` and `npm run verify:built-fixture` pinning the source
and emitted fixed WAV. Its controls and receipt exercise a trusted
`HTMLAudioElement`; they do not define product media UI or establish a runtime
pass without physical output, lifecycle, and resource-release evidence. See
[`docs/m1/audio-playback.md`](docs/m1/audio-playback.md).

The script-runner spike uses the same locked sequence from
`spikes/script-runner`. Its fixed 15-case probe runs each case in a fresh
QuickJS-WASM Worker and proves both engine interruption and host-side Worker
termination. It has an empty Tauri command/capability surface and does not send
source through Tauri IPC. This candidate does not enable imported JavaScript in
the product; architecture, current runtime observations, and remaining gates
are in [`docs/m1/script-runner.md`](docs/m1/script-runner.md).

`rust-toolchain.toml`, `Cargo.lock`, and `package-lock.json` are application inputs and must be committed. CI uses the pinned Rust toolchain and lockfiles and must not silently refresh dependencies.

## M-1 evidence

Start with [`docs/m1/README.md`](docs/m1/README.md). A green desktop CI job proves source formatting, tests, type checking, and compilation on that runner only. An Android APK build or iOS simulator build is compile evidence only. Neither is physical-device smoke evidence.

The latest indexed implementation subject is `d56388e`: its
[Product workflow](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504268)
passed 6/6 jobs and its
[M-1 workflow](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249)
passed 30/30 jobs. Exact job links and runtime limitations are in the
[verification matrix](docs/m1/verification-matrix.md).

## Data boundary

LorePia has no operating server that stores or collects user content. When a user sends a message, the assembled prompt—including conversation, lorebook, and card content—is sent to the LLM provider selected and configured by that user. Other application data is intended to remain on the user's device.

## License

Copyright 2026 TokenGourmet. Licensed under the [Apache License 2.0](LICENSE); attribution notices are in [`NOTICE`](NOTICE).
