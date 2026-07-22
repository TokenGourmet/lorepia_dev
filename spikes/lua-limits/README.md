# LorePia M-1 Lua limits spike

Disposable diagnostic spike for one fixed, self-authored Lua 5.4 fixture set.
It tests instruction/deadline interruption, allocator pressure, stack pressure,
dangerous-standard-library removal, protected-call/coroutine bypass removal,
and host recovery. It is not a product scripting API and accepts no external
script, path, or limit input.

Run the full local checks:

```sh
npm ci
npm test
npm run check
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
```

Launch the plain diagnostic shell with `npm run tauri dev`.
