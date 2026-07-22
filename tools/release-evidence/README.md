# LorePia release evidence tools

These dependency-free Node tools generate deterministic, reviewable inputs for
the release pipeline. They do not sign or publish an application.

- `cargo-metadata-to-cyclonedx.mjs` converts `cargo metadata --locked` output
  into a CycloneDX 1.5 component and dependency graph.
- `generate-hash-manifest.mjs` recursively hashes explicit files or directories,
  rejects symlinks and paths outside its base, and binds the result to a full
  source commit. The release workflow feeds it the NUL-delimited Git index so
  every tracked source file is covered, including paths containing spaces.
- `verify-dependency-policy.mjs` rejects dependencies with missing or
  previously unreviewed license expressions. The checked-in allowlist can only
  change through an explicit code review.

Run the contract tests with:

```sh
node --test tools/release-evidence/release-evidence.test.mjs
```

These outputs are unsigned evidence. Store signing, notarization, mobile
archive export, update signatures, and rollback remain separate release gates.
The scheduled RustSec workflow is deliberately separate because its advisory
database changes over time, while a release evidence bundle is immutable.
