# Imported executable quarantine contract

## Status

**Implemented metadata boundary; execution remains prohibited.**

This product-owned contract records that an imported JavaScript payload was
observed without creating a source-to-runner path. It is a narrow prerequisite
for future importer work, not the reviewed hook contract or executor extraction
required to reopen imported execution under
[`ADR 0001`](../decisions/0001-imported-code-execution.md).

## Exact boundary

The product accepts only a serialized metadata object containing:

- metadata contract version `1`;
- language `JAVASCRIPT`;
- a non-negative safe-integer content byte length; and
- a lowercase SHA-256 content identity.

The serialized metadata is limited to 4096 UTF-8 bytes before JSON decoding.
Its key set is exact. Source, code, URL, runtime, policy, disposition, and
activation fields are rejected rather than preserved. A valid record receives
one immutable product-authored result:

```text
disposition = INERT_QUARANTINED
executable  = false
policy      = DISABLED_BY_SECURITY_POLICY
```

Manifest values, import settings, migrations, and stale legacy settings are
untrusted policy inputs. The product policy resolver does not inspect them and
cannot derive a different result from them. The same policy constant is used
by the frontend parser for the native bootstrap response, preventing the
quarantine contract and startup contract from drifting to different values.

## Deliberately absent

This change adds no imported source field, execution method, hook name,
compatibility semantics, persistence schema, runtime dependency, route,
native command, capability, network permission, or CSP exception. In
particular, it does not copy the QuickJS candidate into the non-disposable
product workspace and does not admit arbitrary JavaScript.

The existing product boundary remains:

- one native command and one capability permission;
- `worker-src`, `frame-src`, and `object-src` set to `'none'`;
- production network access limited to same-origin resources;
- emitted runtime and executable artifact tripwires; and
- imported executable content disabled in every current profile.

## Evidence and claim limits

Unit tests prove exact metadata parsing, a pre-decode metadata-size check,
immutable quarantine output, rejection of executable-looking extra fields, and
non-enablement by hostile manifest/import/legacy objects. Product source,
type-check, build-output, CSP, and native-surface checks cover the unchanged
shell boundary.

These tests do not prove arbitrary-source admission, source-size enforcement,
guest network denial, host resource ceilings, queue ownership, lifecycle,
store acceptance, or runtime behavior on any OS. Those remain reopening gates.
Public card hook names also remain intentionally undefined because the v2 plan
classifies the earlier hook list as compatibility research rather than a
product API.
