# M0 product scaffold

This directory records the first non-disposable LorePia product slice. It is a
start of M0, not an M0 completion claim.

## Implemented slice

```text
Svelte 5 static product screen
  -> trusted main-WebView Tauri capability
     -> get_product_bootstrap -> lorepia-core
     -> write-only credential commands -> five-OS OS credential store
     -> provider stream commands -> native HTTPS/SSE/NDJSON runtime
```

The internal bootstrap response is intentionally exact and small:

- contract version `2`
- product name and core version
- device-local data policy, except user-selected LLM requests
- imported executable content `DISABLED_BY_SECURITY_POLICY`

The product also has a metadata-only
[`imported executable quarantine contract`](imported-executable-quarantine.md).
It records a JavaScript payload's byte length and SHA-256 identity as immutable
`INERT_QUARANTINED` data. It accepts no source, hook, runtime, or activation
field, and imported or stale settings cannot change the fixed disabled policy.

The application still exposes no filesystem, shell, dialog, plugin, audio,
import, database, Lua, or imported-code command. The production CSP blocks
remote browser networking. The trusted main WebView now has a narrow native
credential and provider-stream command surface; provider traffic leaves through
the Rust HTTP client, not browser `fetch`. See the
[native provider runtime contract](provider-runtime.md).

The product-owned [LLM provider catalog](llm-provider-catalog.md) records six
providers and a UI-independent request compiler. Five API-key providers now
connect through the native vault and bounded stream runtime. Vertex request
compilation exists, but invocation remains fail-closed until the native Google
OAuth flow is implemented. The settings screen now writes API keys directly to
the native vault, keeps only a volatile provider/model profile in the WebView,
and the chat screen consumes the authenticated stream with ordered ACK,
cancellation, fixed public errors, terminal snapshot recovery/cleanup, and a
native-owned fixed first-chat prompt.

This bootstrap contract is an internal startup seam. It does not freeze the
blocked public plugin API or any M-1 spike contract.

The product-level decision and reopening requirements are recorded in
[`ADR 0001`](../decisions/0001-imported-code-execution.md). The M-1 iframe and
broker code remains disposable research and is not copied into this product.
An independently terminable runtime and pre-decode bounded transport are
necessary reopening conditions, but satisfying one condition does not
automatically change the policy; a new reviewed contract is required.
The disposable [QuickJS-WASM Worker candidate](../m1/script-runner.md) now
demonstrates termination on three local WebView runtimes, but it is not copied
into this product and does not change the bootstrap policy.

## UI ownership boundary

The first owner-authored UI slice now includes shared design tokens, app-shell
screens, chat composition, keyboard inset handling, and bounded touch gestures.
It does not add an iframe, audio element, dynamic-code surface, or browser
network path. This is a product slice, not an M0-completion claim.

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
- `npm run build` scans the emitted frontend for the v2 disabled-policy marker,
  known iframe/dynamic-code/Worker/WebAssembly surfaces, executable
  Lua/WASM extensions, stale policy values, and unreviewed artifact types.
  This is a regression tripwire, not a general-purpose code sandbox.

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

## Baseline local evidence on 2026-07-19

The historical reproducible receipt for feature commit `3434dbe` is in
[`evidence/2026-07-19-local.md`](evidence/2026-07-19-local.md).

On that exact feature commit, the following passed with its committed source
and lockfiles:

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

That immutable receipt describes bootstrap v1 and 15 frontend tests. It is not
silently rewritten to claim the current v2 security-policy contract; current
source and hosted-CI evidence receives its own exact-commit record.

## Current hardening evidence

Implementation subject `d56388e` has a separate reproducible
[`local and hosted receipt`](evidence/2026-07-19-hardening-d56388e.md).
Its [Product run](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504268)
passed all six quality/desktop/mobile-compile jobs, and its
[M-1 run](https://github.com/TokenGourmet/lorepia_dev/actions/runs/29671504249)
passed all 30 spike jobs. This closes the hosted compile/check failures for that
exact subject only. It does not establish a physical-device or packaged
Windows/Linux runtime pass.

## Open M0 gates

- No physical Android or iOS smoke evidence exists from this product shell.
- No packaged Windows or Linux runtime smoke evidence exists from this product
  shell; hosted native compilation is recorded separately.
- The v2 benchmark regression gate has no honest workload or baseline yet; no
  arbitrary threshold is invented here.
- The first design-token and responsive screen slice exists; accessibility,
  physical-device polish, and broader product flows remain open.
- M-1 exit review and plugin API freeze remain blocked by the M-1 evidence
  matrix. Imported JavaScript and Lua remain disabled by product policy; M-1
  completion alone cannot enable them without an independently terminable
  runtime, bounded transport, and a new reviewed bootstrap contract.

Do not label this slice “M0 complete,” “5OS runtime passed,” or “plugin API
frozen.”
