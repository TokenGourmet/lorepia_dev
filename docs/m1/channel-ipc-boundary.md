# M-1 Tauri Channel transport boundary

This record captures the source-level decision made after the broker candidate.
It is not physical-device evidence and does not close a platform cell in the
verification matrix.

## Audited dependency behavior

The spike is locked to `tauri 2.11.5` and `tauri-runtime-wry 2.11.4`. In that
Tauri version:

- JSON Channel payloads smaller than 8192 bytes are evaluated directly in the
  destination WebView. Larger payloads are placed in a process-global queue.
- On non-macOS/iOS targets, JSON object/array command responses also use a
  Channel callback and therefore share the same large-response path.
- Queue identifiers come from a process-global incrementing `u32` counter.
- `plugin:__TAURI_CHANNEL__|fetch` removes an entry by that identifier without
  checking that the requesting WebView owns the entry.
- the fetch command is explicitly exempted from the normal ACL rejection path.

Upstream source references:

- [Channel threshold and global queue](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri/src/ipc/channel.rs#L34-L46)
- [large-payload enqueue and fetch](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri/src/ipc/channel.rs#L150-L180)
- [command responses routed through Channel](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri/src/ipc/protocol.rs#L340-L404)
- [fetch removes by numeric ID](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri/src/ipc/channel.rs#L318-L333)
- [fetch ACL exception](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri/src/webview/mod.rs#L1810-L1827)

Therefore a second Tauri-managed plugin WebView in the same process is not an
acceptable isolation boundary. Even with zero app-command capability, imported
code could guess or race a large trusted-host Channel response, remove it from
the shared queue, and observe or disrupt the stream. A capability-only split
would hide the application commands but would not bind queued data to its owner.

## Implemented mitigation

Candidate commit `f7a3270` keeps the current spike off that queue:

1. Every `StreamEvent` passes through one size-checking send helper.
2. LorePia's serialized JSON budget is 4096 bytes, leaving margin below the
   audited Tauri threshold.
3. Oversized mock-source batches are split at UTF-8 boundaries. Tests cover
   ASCII, Korean, emoji, JSON control-character expansion, and a full 1 MiB
   stream without byte loss.
4. `completed`, `cancelled`, and `failed` Channel events do not repeat the full
   accumulated text.
5. The final snapshot returns only `textBytes` and `textSha256`. The trusted
   frontend hashes its accepted deltas and rejects a mismatched receipt.
6. A broker result that would exceed the same 4096-byte response budget fails
   closed instead of entering Tauri's large command-response path.
7. Lifecycle commands accept only the exact fixed-width request IDs issued by
   the Rust registry, and error responses never echo a caller-supplied ID.
   Each request permits only one outstanding terminal waiter, so repeated
   direct invokes cannot accumulate unbounded watch receivers.
8. The app capability targets the exact trusted `main` WebView label, not the
   containing window, and the config explicitly enables only that capability.
9. Regression tests pin the audited Tauri/runtime-wry versions. A dependency
   update must re-audit these internal transport semantics before changing the
   lock.

This is a mitigation for the ten commands in the disposable spike, not a
general patch to Tauri. Any new command response or Channel payload must receive
an explicit bounded response-shape test.

The current lifecycle contract adds `wait_stream_terminal` and
`release_stream` without returning raw accumulated text. The waiter returns the
same bounded byte-length/SHA-256 receipt and may recover only a missed terminal
event at the next contiguous sequence, or a control-plane-only failure at the
last delivered sequence. A sequence gap or receipt mismatch fails closed. The
stream rejects empty chunks/deltas so a missing data event cannot masquerade as
a receipt-neutral terminal event. The
final snapshot is releasable only after the delivered terminal is cumulatively
ACKed; explicit release removes the exact retained request, while a five-minute
terminal TTL is the dead-WebView fallback. Immediate opportunistic eviction is
not used because it can race final validation and release.

Android URI grants in this spike are also limited to named app files, cache,
and app-specific external export directories. No root or broad external-storage
mapping remains.

## Product decision

- Do not add a same-process Tauri-managed plugin WebView as the production
  execution boundary.
- Do not fall back to an executable iframe in the privileged `main` WebView.
- Store-Safe mobile builds keep imported JavaScript and Lua off and must omit
  executable imported-code assets.
- Reopening executable plugins requires either an upstream/forked queue bound
  to `(webview_label, data_id)`, or a non-Tauri-managed/independently terminable
  execution boundary with its own bounded typed transport.

Tauri capability scoping is still defense in depth. Its reference explicitly
states that a window match grants its capability to all WebViews in that window
and recommends WebView labels for multi-WebView windows:
[Tauri capability reference](https://v2.tauri.app/reference/acl/capability/).
That scoping alone is not the missing queue-ownership check.

Commits `07ff9c9` and `3f511f2` implement the current Store-Safe packaging
boundary. Android and iOS builds disable Vite's wholesale public-directory copy,
emit only an explicit non-executable asset allowlist, and replace `/isolation`
with a status-only route. The chained build verifier rejects either fixture
asset and known plugin-runtime markers. Unknown non-empty
`TAURI_ENV_PLATFORM` values fail closed so a misspelled mobile target cannot
select the desktop research fixture.

## Remaining evidence

- The 4096-byte invariant currently has source/unit evidence only.
- The changed protocol still needs exact-commit packaged runtime execution on
  macOS and Android emulator, then Windows, Linux, and physical mobile devices.
- Stock Tauri still parses incoming command JSON before the Rust broker's
  admission and payload checks.
- The current research iframe shares the `main` WebView label. Its Android
  isolation failure remains preserved. Source tests and simulated Android/iOS
  frontend builds show the fixture absent from Store-Safe output, but packaged
  artifact inspection and runtime evidence are still required.
- A same-event-loop iframe watchdog still cannot preempt a synchronous busy
  loop.
