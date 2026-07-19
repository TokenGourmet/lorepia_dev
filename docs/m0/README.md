# M0 product scaffold

This directory records the first non-disposable LorePia product slice. It is a
start of M0, not an M0 completion claim.

## Implemented slice

```text
Svelte 5 static product screen
  -> get_product_bootstrap
  -> trusted main-WebView Tauri capability
  -> lorepia-core
```

The internal bootstrap response is intentionally exact and small:

- contract version `1`
- product name and core version
- device-local data policy, except user-selected LLM requests
- imported executable content disabled pending M-1 evidence

The application exposes no filesystem, shell, dialog, HTTP, plugin, audio,
import, database, provider, keychain, Lua, or Channel command. The production
CSP blocks frames, media, objects, workers, forms, and remote network
connections. The main WebView can invoke only `get_product_bootstrap`.

This bootstrap contract is an internal startup seam. It does not freeze the
blocked public plugin API or any M-1 spike contract.

## UI ownership boundary

The current page is functional plain HTML. It has no CSS, design tokens,
animation, transition, iframe, audio element, or component library. Product
visual design and animation remain owner-authored. Therefore the v2 plan's M0
design-token condition is intentionally still open.

The existing LorePia icon is reused unchanged only because Tauri requires a
compile-time application icon. No new branding or visual asset was designed in
this slice.

## Product source policy

- The product Rust workspace uses the root `Cargo.lock`.
- The product JavaScript application uses
  `apps/desktop-mobile/package-lock.json`.
- Exact dependency versions are used in manifests.
- Existing M-1 spikes remain excluded from the root Rust workspace and retain
  their independent lockfiles.
- `src-tauri/gen/android` and `src-tauri/gen/apple` are committed product
  source. CI must fail if either wrapper is absent; it must not initialize a
  disposable wrapper.
- `src-tauri/gen/schemas`, native build directories, Gradle state, Xcode user
  state, and emitted binaries are generated output and remain ignored.
- Wrapper regeneration is a reviewed change tied to a Tauri CLI update.
- Android CI validates committed wrapper JARs and the Gradle distribution URL
  is paired with its official SHA-256 checksum.

The provisional application identifier is `dev.lorepia.client`. It must be
confirmed against the actual Apple and Android publisher accounts before a
signed store release.

The Apple wrapper was generated locally by the locked Tauri CLI. The Android
wrapper was seeded from the repository's same-version generated Tauri wrapper
because this Mac has no Android SDK; all spike identifiers and product text
were replaced. Android CI is the first independent compile gate for that
wrapper.

## Local verification

From the repository root:

```sh
cd apps/desktop-mobile
npm ci
npm test
npm audit --audit-level=moderate
npm run check
npm run build
cd ../..
cargo fmt --all -- --check
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo check --locked --workspace --all-targets
cd apps/desktop-mobile
npm run tauri build -- --debug --no-bundle --ci
```

Run `npm run tauri dev` for the local macOS product shell. A desktop compile,
Android APK compile, or iOS simulator compile is compile evidence only.

## Local evidence on 2026-07-19

The reproducible receipt is in
[`evidence/2026-07-19-local.md`](evidence/2026-07-19-local.md).

On the development Mac, the following passed with the committed source and
lockfiles:

- 15 frontend contract/boundary tests
- Svelte and TypeScript checking with zero errors and warnings
- static frontend production build
- npm audit at the moderate threshold; three known low-severity findings remain
  in the SvelteKit development dependency chain
- Rust formatting, three workspace tests, Clippy with warnings denied, and
  workspace check
- debug macOS executable and `LorePia.app` bundle build
- packaged macOS WebView runtime round trip showing the connected core,
  bootstrap contract v1, core version 0.1.0, local-first data boundary, and
  disabled imported executable content
- debug ARM64 iOS simulator bundle build at `LorePia.app`

The iOS bundle was compiled but not launched. Android was not compiled locally
because the Mac has no Android SDK. Those facts do not establish physical
mobile or five-OS runtime support.

## Open M0 gates

- GitHub CI has not established green Linux, macOS, Windows, Android, and iOS
  results until the workflow runs on the branch or PR.
- No physical Android or iOS smoke evidence exists from this product shell.
- The v2 benchmark regression gate has no honest workload or baseline yet; no
  arbitrary threshold is invented here.
- Design tokens and responsive visual design are deferred to the owner.
- M-1 exit review and plugin API freeze remain blocked by the M-1 evidence
  matrix. Imported JavaScript and Lua therefore remain disabled.

Do not label this slice “M0 complete,” “5OS runtime passed,” or “plugin API
frozen.”
