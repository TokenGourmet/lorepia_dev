# M-1 Lua 5.4 limit-enforcement boundary

This record defines the disposable Lua limit spike. It is an implementation and
evidence contract for a diagnostic runtime, not approval of a LorePia scripting
API and not a runtime `PASS`. Hosted tests, compilation, a simulator, and an
emulator do not replace the five physical-platform cells in
[`verification-matrix.md`](verification-matrix.md).

The spike answers one narrow question: can a fixed self-authored Lua 5.4 corpus
run inside explicit instruction, deadline, allocator, and standard-library
bounds while the host survives every hostile case? It does not accept imported
Lua and does not change the Store-Safe decision that imported JavaScript and
Lua remain off.

## Exact runtime and IPC boundary

The native spike pins `mlua =0.12.0` with default features disabled and only
the `lua54` and `vendored` features enabled. LuaJIT, Luau, dynamic modules,
async execution, and system Lua linkage are outside this vertical.

`cargo tree -e features` also shows the upstream `mlua-sys` vendoring helper
feature names `lua-src` and `luajit-src`. That is how `mlua-sys` makes its
vendored source builders available; it is not selection of the `mlua/luajit`
runtime feature. The selected VM feature is `mlua/lua54`, `mlua/luajit` is
disabled, and the runtime contract separately requires `_VERSION == "Lua 5.4"`.

Tauri's iOS shell links the Rust product as `libapp.a`. The upstream vendored
Lua archive is marked `-bundle`, so it is not automatically folded into that
static library and an unmodified iOS link leaves the `lua*` symbols unresolved.
For iOS targets only, this spike declares the matching pinned `mlua-sys`
dependency and its build script copies the already-built `liblua5.4.a` under a
private name and links it with Rust's `+bundle` modifier. This keeps the final
iOS app self-contained without enabling dynamic modules or changing the
selected Lua 5.4 runtime.

Exactly one no-argument Tauri command is exposed:
`run_lua_limits_m1_probe`. The WebView cannot provide source, bytecode, a file
or module path, globals, a limit, a fixture, or an expected result. Rust loads
only the six fixed self-authored sources pinned by
[`fixtures/catalog.json`](../../spikes/lua-limits/fixtures/catalog.json),
whose current SHA-256 is
`9ea567d6901ec39412e73f439ee9ea7d47538baea4d1a92cd409c9f3e9b97db5`.
Rust embeds that catalog and all six sources at compile time, reports the
catalog hash, and uses native tests to pin every source length and SHA-256. It
creates a new Lua state for every fixture and recovery case. No global,
registry value, hook, or allocator state is reused between cases.

The async command moves the complete native probe into Tauri blocking work so
Lua does not execute on the WebView event-loop thread. A process-wide
non-blocking lock covers catalog validation and every fixture; a concurrent
invocation returns `PROBE_BUSY` instead of starting another VM. This is
in-process serialization, not multi-process exclusion.

Success and error IPC are fixed proof receipts capped at 4096 serialized bytes.
They contain stable case identifiers, booleans, counts, timings, memory
measurements, dependency/policy metadata, and the fixture-catalog hash only.
Raw Lua source, bytecode, native or Lua error text, stack traces, and filesystem
paths must not cross IPC.

## Fixed resource policy

These are diagnostic probe values. They are not product defaults and must not
be copied into the M4 scripting API by implication.

| Resource | Exact policy |
|---|---:|
| Wall-clock deadline per fixture | 50 ms |
| Lua VM instruction cap per fixture | 100,000 |
| Instruction-hook cadence | every 1,000 VM instructions |
| Absolute Lua allocator ceiling | 8,388,608 bytes (8 MiB) |
| Serialized success or error IPC | 4,096 bytes |

At each 1,000-instruction callback, the hook checks the monotonic deadline and
then the cumulative instruction estimate, aborting when either configured
boundary has been reached. If both are true at one callback, the stable result
is the deadline code because it is checked first. The receipt records the
observed stop code, instruction estimate, elapsed time, configured values, and
whether each host recovery sentinel completed.

The 8 MiB limit is absolute memory tracked by the Lua allocator for that Lua
state, including its initialized baseline; it is not 8 MiB in addition to the
baseline. The initial `used_memory` value seeds the observed peak, while the
receipt records final used bytes, maximum observed bytes, the configured
ceiling, and whether an allocator rejection occurred. It must never describe
this as a limit on Rust, Tauri, WebView, native stack, or total process memory.

## Library and chunk allowlist

Each state starts from an explicit allowlist and then reduces the global table
to exactly four library tables: `math`, `string`, `table`, and `utf8`. It does
not retain base-library globals and does not rely on an `ALL_SAFE` convenience
set. Before any fixture loads, the native harness proves all of these globals
are absent:

```text
os io package debug require dofile loadfile load collectgarbage
pcall xpcall coroutine print warn
```

Removing `pcall`, `xpcall`, and `coroutine` is part of this diagnostic boundary:
the probe must not assume that an ordinary Lua error raised by a cooperative
hook is uncatchable. Removing `load`, file loaders, module loading, debug access,
and `print` also prevents fixture-selected code loading, hook mutation, host I/O,
or unbounded diagnostic output.

Every chunk is explicitly text-only. Binary/precompiled chunks are rejected
before execution. No Rust callback, userdata method, filesystem API, network
API, clock API, process API, or native module is exposed to the fixture.

## Required fixed cases

The self-authored catalog pins source order, byte lengths, SHA-256 values,
origin, and license. The native and frontend contracts separately pin this
exact receipt order:

```text
allowed-baseline
infinite-loop
recovery-after-infinite-loop
recursive-pressure
recovery-after-recursive-pressure
allocator-pressure
recovery-after-allocator-pressure
forbidden-globals-absent
recovery-after-forbidden-globals
bypass-surfaces-absent
recovery-after-bypass-surfaces
```

A successful diagnostic receipt proves all of the following without returning
the sources:

1. **Allowed sentinel:** a deterministic bounded fixture returns its exact
   golden value and stays below every configured resource ceiling.
2. **Infinite loop:** a pure-Lua non-terminating loop stops at the instruction
   cap or the 50 ms deadline, whichever the hook observes first. The host
   process and command remain alive.
3. **Recursion pressure:** hostile recursive Lua stops with the stable stack,
   instruction, or deadline limit code without aborting the Rust process or
   leaving a reusable poisoned state.
4. **Allocator pressure:** deterministic allocation growth is rejected by the
   8 MiB Lua allocator ceiling, and the recorded Lua memory never exceeds that
   ceiling.
5. **Forbidden globals:** every name in the denylist is absent. Attempts to use
   file, OS, dynamic-load, debug, protected-call, coroutine, collection-control,
   and output paths produce only stable diagnostic outcomes.
6. **Recovery sentinel:** after every hostile case, a new clean Lua state runs
   the allowed sentinel successfully. Final lock release is proved by a later
   invocation or an equivalent native regression test.

Expected hostile rejections are assertions inside a successful proof receipt.
An unexpected harness, catalog, serialization, or recovery failure returns a
stable top-level error code; it must not be converted into a successful case.

Native tests must also prove that binary/precompiled input is rejected by the
text-only loader. They must pin the exact hook and allocator constants and
cover allowed-case headroom, hostile interruption, allocator rejection,
forbidden-global completeness, fresh-state recovery, `PROBE_BUSY`, response
size, and raw-error/source-field rejection.

## Cooperative-hook limitation

This probe is intentionally not a hard wall-clock execution boundary. A Lua
5.4 instruction hook runs only when the VM reaches the next configured hook
callback. OS descheduling can make measured return time exceed 50 ms, and the
hook cannot preempt a long-running native/C function between VM instructions.
The fixed runtime exposes no such callback, so this vertical can test the
restricted pure-Lua case only.

Source tests that pin the policy constants do not turn the cooperative
mechanism into a real-time kill. Qualifying runtime evidence must retain actual
elapsed measurements and any overshoot rather than rounding them into a pass.
If a future product API exposes native callbacks, protected error recovery,
coroutines, or dynamically loaded code, this result no longer establishes
termination and an independently terminable boundary must be evaluated.

## Qualifying evidence

A physical-platform record must include the normal M-1 evidence fields plus:

- exact source commit, Rust toolchain, `Cargo.lock`, and fixture catalog hashes;
- physical OS/build and hardware identity;
- Lua `_VERSION`, exact `mlua` feature/version selection, and denylist result;
- per-case expected/actual stable result, stop code, instruction estimate,
  actual elapsed time, final/peak observed Lua memory, and 8 MiB ceiling;
- recovery-sentinel and process-lock results; and
- bounded raw logs or artifact links that contain no fixture source or raw Lua
  errors.

Desktop hosted jobs can establish source tests and native compilation on their
named runners. Android CI builds an ARM64 debug APK and iOS CI builds an ARM64
simulator app without launching it. Those are compile-only. A simulator or
emulator runtime is still simulated evidence. None changes a physical Lua
capability cell or the physical `Lua budget and stdlib removal` negative cell.

## Evidence this spike does not claim

- It does not define or freeze LorePia's product Lua hooks, globals, values,
  permissions, error model, compatibility surface, or plugin API.
- It does not execute imported packages or justify enabling imported Lua in a
  Store-Safe build. Existing mobile JS/Lua-off policy remains unchanged.
- It does not prove full process-memory containment, native stack containment,
  hard real-time termination, filesystem/network isolation for a future API,
  or safe execution of Rust/C callbacks.
- It does not replace Windows, Linux, Android-device, or iOS-device runtime
  evidence with a macOS run or a cross-compile.
- It contains no product UI design, animation, or interaction contract.

## Upstream references

- [`mlua 0.12.0` feature selection](https://docs.rs/crate/mlua/0.12.0)
- [`Lua::set_hook`, `set_memory_limit`, and `used_memory`](https://docs.rs/mlua/0.12.0/mlua/struct.Lua.html)
- [`HookTriggers` instruction cadence](https://docs.rs/mlua/0.12.0/mlua/struct.HookTriggers.html)
