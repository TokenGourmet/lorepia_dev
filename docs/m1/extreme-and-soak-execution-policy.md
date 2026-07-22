# Extreme and soak execution policy

This document defines how LorePia schedules, isolates, measures, and records
extreme and soak runs. It does not define checklist IDs, replace the reviewed
2026-07-20 extreme plan or launch audit, or assert that a test has run. The
canonical 365-ID catalog and its exact source hashes remain in
[`tools/extreme-test-tracker/manifest.json`](../../tools/extreme-test-tracker/manifest.json).
Current platform evidence remains in
[`verification-matrix.md`](verification-matrix.md).

Every run must select an existing canonical ID before it starts. A useful new
scenario that has no existing ID is a proposal to revise the canonical source,
not an ad hoc ID and not a second checklist in this file. Only a reviewed
manifest override may change an extreme-check status.

Every asynchronous, stress, and soak command also has an external wall-clock
watchdog. A lost terminal signal, deadlock, or hung child is a failed run with
the last available task/process diagnostics; it is never allowed to hold a CI
runner indefinitely. Numeric limits are exercised at the accepted boundary and
on both sides where meaningful (`limit - 1`, `limit`, and `limit + 1`). A
failure artifact is retained alongside the later successful replay instead of
being replaced by it.

## Execution tiers

The tiers describe intended scheduling and evidence strength. Their presence
here does not prove that a corresponding workflow or device lab exists.

| Tier | Intended work | Evidence boundary |
|---|---|---|
| Pull request | Deterministic unit and contract tests, fixed fixtures, boundary values, format/lint/build checks | May prove source behavior or compilation only; it cannot promote a packaged runtime or physical-device cell |
| Nightly | Release-mode stress, seeded model-based runs, repeated race cases, maximum admitted payload/count checks, medium database fixtures, and a 30-minute short soak | Must retain the seed and raw metrics; hosted execution remains hosted evidence |
| Standard runtime | A two-hour integrated run on the named reference desktop or physical device, including relevant lifecycle transitions | Applies only to the exact executable, OS, hardware, and scenario recorded |
| Release-candidate component | An eight-hour run of the affected product profile or capability with raw resource telemetry | Does not satisfy the launch soak by itself and cannot be assembled from shorter runs |
| Launch candidate | One continuous 72-hour integrated-product soak for the launch condition tracked as `GO-016` | A spike, an eight-hour component run, or the sum of several shorter runs cannot close this gate |
| Destructive or large-fixture | Disk-full, corruption, process-kill, migration-crash, multi-GiB database, and large asset/backup work on dedicated disposable infrastructure | Never part of an ordinary developer checkout or shared-volume PR run |

A two-hour device run and an eight-hour component run are useful intermediate
evidence, but neither changes a physical-device or packaged-runtime result
unless the exact canonical acceptance scenario was executed. Any matrix cell
that is currently `NOT RUN` remains `NOT RUN` until such a receipt is reviewed.

## Model-based and randomized runs

Model-based tests use the production contract as the oracle and must preserve
the same invariants as deterministic tests. Each run records:

- the existing canonical test ID, commit, generator version, configuration,
  action depth/count, and original seed;
- the complete failure action sequence and the first violated invariant;
- the shrinker version and the smallest reproducing sequence it found;
- commands that replay both the original seed and the minimized sequence; and
- after a fix, a deterministic regression fixture plus successful replay of
  both reproductions.

An unrepeatable randomized failure remains a failure with its raw artifacts; it
is not discarded as noise. Protocol timing logic should use a controllable
clock where possible. Real-clock scheduler and lifecycle races belong in the
stress or soak lanes and must record their actual timing distribution.

## Raw telemetry and percentile evidence

Soak and performance runs retain timestamped raw samples rather than only a
dashboard screenshot or an average. Record the sampling interval and, where
the subject exposes the metric:

- RSS or private bytes, CPU, threads, async tasks, request/registry counts, and
  WebView or Worker counts;
- file descriptors on Unix or handles on Windows;
- database, WAL, SHM, freelist, staging, backup, asset, and free-space bytes;
- throughput, operation/event latency, p50, p95, p99, and stable error counts;
  and
- device lifecycle state, battery/charging mode, thermal state, and WebView or
  WebKit version for mobile measurements.

Also retain the exact commit, artifact hash, build profile, OS/build, hardware
or runner, toolchain and SQLite versions, fixture hash, warm-up, sample count,
expected threshold, raw artifact location, and the command that derives each
reported percentile. Percentiles are computed from raw samples; percentile
values from separate runs are not averaged.

Unavailable instrumentation is recorded as unavailable, never as zero. For
example, `reserved bytes` is evidence only when the exact product or spike under
test exports that counter. A missing required metric leaves the relevant check
`not_run`, `in_progress`, or `blocked` according to the tracker vocabulary; it
cannot produce a `pass`.

## Destructive-test containment

Disk exhaustion, WAL/SHM removal, permission changes, database-page corruption,
archive bombs, process termination, and migration crash injection must satisfy
all of these controls:

1. Run only against generated fixtures or verified copies under a newly
   created disposable root on a quota-limited volume, sparse disk image,
   container, emulator, or equivalent isolated filesystem.
2. Resolve and validate the exact root before mutation. Reject a home
   directory, workspace/repository root, installed app-data directory, backup
   source, shared runner volume, unresolved environment variable, symlink, or
   path outside the disposable root.
3. Place a run-specific guard marker in the disposable root and require both
   the marker and expected fixture identity before every destructive phase.
4. Preflight quota, host free space, expected peak temporary space, maximum
   bytes/files, and an external wall-clock timeout. Stop before the host or
   shared volume approaches exhaustion.
5. Launch the subject as a child process and terminate only the recorded child
   process group. Never inject a crash into the interactive Codex shell, CI
   supervisor, database owned by another process, or a user's installed app.
6. Exercise live-WAL behavior through the product's snapshot/backup contract.
   Direct WAL/SHM deletion or page mutation is allowed only on a closed fixture
   copy made for that case.
7. Keep cleanup bounded to the validated disposable root. Preserve the failed
   fixture, seed, logs, and hashes as an artifact before cleanup when policy and
   available space permit.

Real multi-GiB and high-file-count cases are explicit dedicated load gates.
Injected accounting or sparse fixtures may test admission logic, but they are
not runtime proof of the corresponding real-size case. Fault servers must bind
to a controlled test interface, use synthetic non-secret payloads, and enforce
request, byte, duration, and cost limits.

## Product and spike evidence are separate

A spike receipt proves only the named spike harness at its exact commit. A
product receipt must exercise the built LorePia product through its product
command, persistence, and lifecycle path and identify the installed executable,
APK/AAB, or app bundle by hash. Do not promote:

- a spike pass to a product pass;
- a Rust or frontend test to Tauri/WebView runtime evidence;
- a desktop compile to a packaged desktop launch;
- an APK/AAB or simulator build to Android/iOS runtime evidence;
- emulator or simulator behavior to a physical-device cell; or
- one OS, WebView, device, or build profile to another.

Where the same canonical ID is exercised by both a spike and the product, keep
separate receipts and name the subject in each evidence note. Product success
does not erase a preserved spike failure, and a product regression cannot be
hidden by a green spike.

## Recording a result

Use the status and evidence rules in the tracker README. At minimum, a result
records the canonical ID, status, full commit, UTC window, exact subject and
artifact hash, environment, fixture/seed hash, command or scenario, expected
and actual outcome, raw logs/metrics, known limitations, and follow-up issue.

Compilation, simulator/emulator, physical-device, and packaged-runtime evidence
remain distinct. A policy document, planned tier, or successful shorter run is
never evidence by itself.
