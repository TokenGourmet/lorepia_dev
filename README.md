# LorePia

LorePia is a local-first, cross-platform AI character chat client in the M-1 risk-removal phase. The current repository contains disposable vertical spikes used to prove or reject the architecture in [`LorePia_기술계획서_v2.md`](LorePia_기술계획서_v2.md); it is not yet the product application.

The first spike exercises mock SSE-to-Tauri-Channel streaming. It is intentionally functional and minimal. Product UI, visual design, and animation are outside the implementation scope here and remain owner-authored work.

## Current scope

- Prove the Tauri 2 + Rust + Svelte toolchain on Windows, macOS, Linux, Android, and iOS.
- Verify Channel sequencing, batching, cancellation, backpressure, and partial-result behavior.
- Record runtime evidence without treating compilation, a simulator, and a physical device as equivalent.
- Keep imported JavaScript and Lua disabled in the Store-Safe profile until written policy clearance and the required isolation evidence exist.

No 5-OS runtime support claim is valid until the [M-1 verification matrix](docs/m1/verification-matrix.md) contains the required evidence.

## Repository layout

```text
.
├── docs/m1/                    # M-1 gates, procedures, and evidence matrix
├── spikes/channel-stream/      # Disposable Channel vertical spike
├── .github/workflows/m1.yml    # Desktop and mobile compile verification
└── LorePia_기술계획서_v2.md    # Current technical plan
```

## Run the Channel spike

Install the current [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS, then:

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

`rust-toolchain.toml`, `Cargo.lock`, and `package-lock.json` are application inputs and must be committed. CI uses the pinned Rust toolchain and lockfiles and must not silently refresh dependencies.

## M-1 evidence

Start with [`docs/m1/README.md`](docs/m1/README.md). A green desktop CI job proves source formatting, tests, type checking, and compilation on that runner only. An Android APK build or iOS simulator build is compile evidence only. Neither is physical-device smoke evidence.

## Data boundary

LorePia has no operating server that stores or collects user content. When a user sends a message, the assembled prompt—including conversation, lorebook, and card content—is sent to the LLM provider selected and configured by that user. Other application data is intended to remain on the user's device.

## License

Copyright 2026 TokenGourmet. Licensed under the [Apache License 2.0](LICENSE); attribution notices are in [`NOTICE`](NOTICE).
