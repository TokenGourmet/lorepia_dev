# LorePia extreme-test tracker

This directory turns the 2026-07-20 extreme test plan and launch audit into a
versioned, machine-readable source of truth. It deliberately has no workspace
or third-party dependencies: Node.js 20 or newer is sufficient, and neither
`Cargo.toml` nor `Cargo.lock` is involved.

## What is tracked

- 365 extreme checklist IDs generated from 13 explicit inclusive ranges
- 7 P0 findings (`P0-1` through `P0-7`)
- 11 P1 findings (`P1-1` through `P1-11`)
- 17 launch conditions, assigned stable IDs `GO-001` through `GO-017` in the
  order of section 10 of the launch audit
- status, evidence, evidence commit, reproducible commands, and remaining work

`manifest.json` records the repository assessment at
`agent/m1-product-script-contract@b8243f386a44e495229183051a0519c6843e34f8`.
This is a conservative audit snapshot, not a claim that an item is complete.

## Commands

Run the self-contained checks:

```sh
node --test tools/extreme-test-tracker/test/tracker.test.mjs
node tools/extreme-test-tracker/tracker.mjs validate
node tools/extreme-test-tracker/tracker.mjs summary
```

Validate the exact source documents as well as the manifest:

```sh
node tools/extreme-test-tracker/tracker.mjs validate \
  --extreme-doc /Users/codexer/Downloads/LorePia_Extreme_Test_Plan_2026-07-20.md \
  --launch-doc /Users/codexer/Downloads/LorePia_Launch_Audit_2026-07-20.md
```

That command fails on:

- a missing, duplicate, or unknown extreme-test ID;
- a missing, duplicate, or unknown P0/P1 finding;
- a changed P0/P1 title;
- a missing, added, reordered, or changed GO condition;
- a source-document hash that differs from the reviewed input.

Use `--skip-source-hash` only while deliberately editing a new document
revision. Before accepting the revision, review the diff, update the ranges or
launch records, and update the corresponding SHA-256 digest.

List all expanded extreme-test IDs or records:

```sh
node tools/extreme-test-tracker/tracker.mjs expand
node tools/extreme-test-tracker/tracker.mjs expand --json
```

## Updating one extreme check

The ranges are the complete catalog and their default is `not_run`. Record an
individual result in `extremeChecks.overrides` without duplicating the other
364 records:

```json
{
  "CHAT-001": {
    "status": "pass",
    "evidence": [
      {
        "kind": "artifact",
        "value": "artifacts/extreme/CHAT-001/result.json",
        "note": "seeded empty-database first-run evidence"
      }
    ],
    "commit": "0123456789abcdef0123456789abcdef01234567",
    "commands": [
      "node tools/extreme-test-tracker/tracker.mjs validate"
    ]
  }
}
```

Allowed check/GO statuses are `not_run`, `in_progress`, `pass`, `fail`,
`blocked`, and `unsupported`. A `pass` is rejected unless it contains evidence,
a full 40-character commit, and at least one reproduction command.

Finding statuses are `open`, `in_progress`, `fixed`, `verified`, and
`accepted`. `fixed` is not the same as `verified`: launch closure should use
`verified` only after the required platform and artifact evidence exists.

## Evidence rules

Evidence must identify what was inspected, not merely state an opinion. Useful
kinds are repository paths, commits, commands, artifacts, physical-device
records, and measurements. A broad claim needs broad evidence: a unit test does
not prove a five-OS runtime condition, and a compile check does not prove a
physical-device or signed-release condition.

When HEAD changes, do not mechanically replace `asOf.commit`. Re-evaluate every
affected P0/P1 and GO record, keep unresolved work explicit, then move the
snapshot commit. Extreme check results should stay pinned to the exact commit
and artifact that produced them.

## Files

- `manifest.json`: compact catalog plus the current audit state
- `schema.json`: dependency-free JSON Schema for editors and external tooling
- `tracker.mjs`: semantic validator, source-document parser, expander, summary
- `test/tracker.test.mjs`: missing/duplicate/order/hash/evidence regression tests

The semantic validator intentionally enforces invariants beyond JSON Schema,
including exact counts, generated-ID uniqueness, known override IDs, complete
P0/P1 ranges, complete GO ranges, and evidence requirements for positive
statuses.
