# LorePia M-1 audio playback spike

Disposable candidate for one fixed, self-authored local WAV played by the
trusted main WebView. It exercises load, play, pause, fixed seek, resume, stop,
release, and a foreground-only lifecycle policy. It is not a product player,
does not accept a path or URL, and does not expose a Tauri command.

Run the full local checks:

```sh
npm ci
npm run verify:fixture
npm test
npm audit --audit-level=moderate
npm run check
npm run build
npm run verify:built-fixture
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
```

Launch the plain diagnostic shell with `npm run tauri dev`. A successful unit
test or compile is not audio-output, lifecycle, or OS-resource-release evidence.
