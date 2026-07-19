# ADR 0001: Imported executable content stays disabled

- Status: accepted
- Date: 2026-07-19
- Scope: the non-disposable LorePia product workspace

## Context

The M-1 unsafe baseline proved that a sandboxed iframe could invoke a raw
Tauri command on the Android emulator and change native state. The broker
candidate removed the demonstrated privileged wrappers, but did not establish
a production execution boundary:

- Tauri 2.11.5 can place large Channel data in an ACL-exempt, process-global
  fetch queue that is not bound to the destination WebView.
- Tauri decodes a command envelope before an application broker can enforce its
  payload limit, so the broker cannot prevent the first oversized allocation.
- A synchronous busy loop can block the host and a cooperative iframe watchdog
  when both share an event loop.

The exact historical observations remain in
[`../m1/isolation.md`](../m1/isolation.md), and the Channel source audit remains
in [`../m1/channel-ipc-boundary.md`](../m1/channel-ipc-boundary.md).

## Decision

The product must treat imported JavaScript and Lua as inert data. It must not
schedule, evaluate, render, or bundle an imported executable runtime under the
current bootstrap contract.

The M0 product enforces this decision with:

- bootstrap contract v2 value `DISABLED_BY_SECURITY_POLICY`; changing this
  value requires a new reviewed contract and cannot be inferred from one
  technical milestone;
- one trusted `main` WebView capability containing only
  `get_product_bootstrap`;
- no import, plugin, Lua, Channel, filesystem, shell, HTTP, or dialog command;
- CSP `frame-src`, `worker-src`, and `object-src` set to `'none'`; and
- source and build-output regression checks that reject known executable
  frame/runtime markers and unreviewed artifact types.

Those checks are tripwires, not a sandbox or proof that arbitrary code is
safe. The boundary depends on the imported-code executor being absent, the
single exact native command/capability, and the closed CSP together.

M-1 completion alone cannot enable imported execution. Enabling any language
requires a new product contract and an explicit review. The review may split
JavaScript and Lua into different policies, but absence of a new decision means
both remain disabled.

## Reopening requirements

Imported JavaScript requires all of the following before implementation:

1. an execution context the trusted host can terminate while remaining
   responsive under a synchronous busy loop;
2. a bounded typed transport that rejects oversized input before the full
   envelope is allocated or decoded;
3. ownership binding for every queued response and stream item;
4. unchanged native-side-effect evidence for raw IPC, forged broker messages,
   stale/replayed credentials, network bypass, and resource exhaustion; and
5. qualifying runtime evidence on every supported product profile, including
   physical Android and iOS devices where applicable.

Imported Lua may use a different native runtime design, but still requires its
own product contract, policy clearance, five-platform limit evidence, and a
negative test proving that imports or stale settings cannot enable it by
default.

## Consequences

- Card/archive import work may inspect, report, preserve, or quarantine script
  bytes, but cannot execute them.
- The iframe broker remains research code in `spikes/`; it is not product code.
- M0 and other non-executable product work may continue without claiming M-1
  exit or plugin API freeze.
- Creator scripting and executable plugin milestones remain blocked, not
  silently downgraded to an unsafe same-process WebView design.
