# M-1 trusted-WebView audio playback candidate

This record defines the disposable audio spike. It is an architecture and
evidence contract for one candidate playback path, not a five-OS runtime
`PASS`, a product media API, or a product interaction design. Hosted tests,
compilation, an emitted fixture, a simulator, and an emulator cannot replace
physical output, lifecycle, and resource-release evidence in
[`verification-matrix.md`](verification-matrix.md).

The spike answers one narrow question: can the trusted main WebView load and
control one approved local fixture through the platform `HTMLAudioElement`, and
does that path obey LorePia's foreground-only mobile policy? It deliberately
tests the UI-side playback path selected by the v2 plan's core-media-trigger to
UI-playback split before that architecture is frozen.

## Decision at this checkpoint

**Status: `RETAIN AS CONDITIONAL CANDIDATE`.** The fixed-fixture contract,
fail-closed state machine, packaging path, and foreground-only lifecycle logic
are sufficient to keep trusted-WebView playback behind a narrow, replaceable
product boundary. They are not sufficient to freeze this backend or mark any
five-OS Audio runtime cell `PASS`.

This spike is frozen after its current contract, tests, fixture, and compile
lanes. Product work may reuse those assets but must reimplement the feature
without carrying over the diagnostic controls or temporary shell. Do not build
deep product audio behavior on this candidate until physical Android and iOS
runs prove audible output, background pause, no automatic resume, and native
resource release. Reopen the spike only when qualifying device evidence fails
or a new product requirement crosses the documented boundary; otherwise move
on to the product skeleton.

## Candidate architecture boundary

Playback belongs to the trusted main WebView and uses `HTMLAudioElement`. This
spike has no Tauri audio command, Rust decoder or output backend, `rodio`
dependency, mobile audio plugin, imported-media picker, remote URL, or untrusted
frame. Tauri packages the fixed frontend and local fixture; it does not expose
an IPC audio control surface.

This is a candidate, not a conclusion that system WebViews provide sufficient
audio lifecycle behavior on every platform. In particular, unit tests can
model media events but cannot prove that a platform WebView decodes the WAV,
opens the expected output route, delivers lifecycle events in time, or releases
native audio resources.

The candidate must remain replaceable. If a physical Android or iOS run misses
the required background signal, continues output, auto-resumes, or cannot prove
resource release, retain `FAIL` and evaluate a platform lifecycle hook or native
playback backend. Do not relabel a failed WebView path as passed because the
same source compiled.

## Fixed fixture and emitted-asset identity

The only approved input is the self-authored, uncompressed PCM WAV at
[`m1-audio-v1.wav`](../../spikes/audio-playback/static/fixtures/m1-audio-v1.wav).
Its generator and adjacent catalog pin the origin, `CC0-1.0` permission,
container/encoding, sample rate, channel count, frame count, duration, byte
length, and SHA-256. The source verifier regenerates the fixture and rejects
any byte or metadata drift:

```sh
npm run verify:fixture
```

After `npm run build`, the emitted-asset verifier requires the corresponding
WAV in the frontend output to have the same pinned identity and rejects
unexpected audio assets:

```sh
npm run verify:built-fixture
```

These checks establish fixture provenance and build inclusion only. They do
not decode the file through a system WebView or produce sound.

## Foreground functional sequence

One user-initiated diagnostic sequence must preserve the ordered observations
for all required actions:

1. **Load:** set only the approved local fixture, wait for usable media
   metadata, and confirm the pinned duration within the declared tolerance.
2. **Play:** call `play()` from the user gesture and observe monotonic media-time
   progress of at least 250 ms within 2500 ms of that call. The overall deadline
   starts before the play promise settles; a resolved promise without progress
   is not sufficient, and a sampled backward move greater than 5 ms fails.
3. **Pause:** require the element to report paused immediately and at the end of
   the bounded 500 ms observation window, with no sampled drift greater than
   100 ms during that window.
4. **Seek:** request the fixed in-range position, observe `seeked`, and prove
   actual media time is within 100 ms of the checkpoint.
5. **Resume:** resume from the seeked, paused position and again observe at
   least 250 ms progress within 2500 ms.
6. **Stop:** pause, reset media time to the beginning, and observe `seeked`
   without silently treating stop as release.
7. **Release:** pause, detach the source, force the media element to discard its
   current resource, unregister owned listeners, and drop the live element
   reference.

The functional controls exist only to run and inspect this contract. They do
not define LorePia's player layout, animation, shortcut, queue, volume, or
accessibility design.

The successful diagnostic receipt must be bounded and schema-strict. It records
a protocol version, backend identifier, fixture identity, ordered step results,
exact cumulative counts for every bounded action, media-time progress and
pause-window observations, media-event counts, and release state. Cumulative
counts must sum to the final sequence number plus one, equal the complete trace
histogram exactly, and bind persistent effects such as successful load
verification, play/pause/resume observations, and seek/end/error event counts.
Play and resume deltas must also agree with their trace positions within the
declared 100 ms observation-position tolerance. A non-playing receipt's live
position must agree with its last transition within that same tolerance; a
playing receipt may advance after its transition, but neither its live position
nor a position-preserving pause/end boundary may move backward by more than the
declared 5 ms monotonic tolerance. Stop and release transitions intentionally
reset the position to zero.
Failures expose one stable code through the diagnostic UI;
neither path may include browser/native error text, host or filesystem paths,
object dumps, or an arbitrary URL. The fixed same-origin public asset route is
part of the fixture identity. Frontend tests must reject missing, extra, or
wrong-typed fields and invalid or reordered transition traces. Every receipt
starts with the canonical `INITIALIZE` entry and retains the complete current
session through the named final entry; the spike never rolls over or silently
drops a prefix. Each foreground diagnostic action is one-shot. A new `Load`
starts a fresh bounded evidence session after release, while a repeated empty
lifecycle cycle may reset an already released session before recording its new
background transition.

## Foreground-only lifecycle policy

The M-1 product policy for this candidate is intentionally narrow:

- playback is foreground-only;
- when the document becomes hidden or receives `pagehide`, the spike aborts any
  in-flight fixture fetch, pauses playback, and releases the media element;
- if `play()` or resume has already unpaused the element but the 250 ms progress
  proof is still pending, backgrounding records that automatic pause before
  releasing while leaving the unproven playback transition uncommitted;
- becoming visible or receiving `pageshow` never auto-plays or reconstructs an
  active playback session; and
- the user must explicitly start a new session after returning to the app.

`visibilitychange`, `pagehide`, and `pageshow` are WebView candidate hooks, not
proven mobile lifecycle guarantees. Automated tests may prove that synthetic
events drive the expected state transition. Only a physical-device record can
show whether the named Android/iOS WebView delivered the relevant callback
before background output or resource retention occurred.

At the JavaScript boundary, release means that playback is paused, the source
is detached, the element is reloaded to discard its media resource, owned
listeners are removed, and the live reference is dropped. That state alone is
not proof that the OS audio focus, session, decoder, route, or buffers were
released. Qualifying mobile evidence must inspect the applicable platform-level
resource behavior separately.

## Qualifying runtime evidence

A physical-platform record must include the normal M-1 fields plus:

- exact source commit, toolchain and lockfile hashes, packaged app hash, and
  fixture catalog/source/emitted SHA-256 values;
- physical OS/build, hardware identity, system WebView or WebKit version, and
  the active audio output route;
- the complete bounded receipt with expected and actual load, progress, pause,
  seek, resume, stop, and JavaScript-release observations;
- direct evidence that the approved fixture produced audible or captured
  non-silent output through the named route; and
- raw logs, recording, or platform inspection artifacts linked from the matrix.

Android and iOS physical-device records must additionally run this scenario:

1. begin audible foreground playback by an explicit user action;
2. background or suspend the app during playback;
3. prove output pauses and the playback object is released;
4. inspect the relevant platform resource/focus/session state; and
5. foreground or resume the app and prove that playback did not auto-start.

If the WebView does not expose enough information to prove OS resource release,
the cell remains `NOT RUN`, `FAIL`, or `BLOCKED` as appropriate; JavaScript
state and silence alone must not be overstated. A desktop runtime pass does not
replace either mobile lifecycle record, and one mobile platform does not stand
in for the other.

## CI and test scope

Desktop CI runs frontend contract tests, Svelte/TypeScript checks, the frontend
build, emitted-fixture verification, and the standard Rust format/test/lint/
check sequence. Android CI cross-compiles a debug ARM64 APK; iOS CI compiles a
debug ARM64 simulator app without signing. Both mobile jobs also verify the
emitted fixture identity.

Fake media elements and synthetic lifecycle events are appropriate for exact
state-machine regression tests. Hosted desktop tests may run without an audio
device. These checks prove logic, schema, assets, and compilation only. None can
change an Audio runtime cell to `PASS`.

## Evidence this spike does not claim

- It does not prove background playback; continued background audio is outside
  this candidate and conflicts with its foreground-only policy.
- It does not prove remote streaming, imported-media playback, codecs beyond the
  fixed PCM WAV, playlist/queue behavior, mixing, volume policy, interruptions,
  Bluetooth routing, lock-screen controls, or media-session integration.
- It does not prove that WebView media state alone equals OS resource release.
- It does not define the Rust core's media-trigger schema or freeze the future
  product playback backend.
- It contains no product UI design or animation.
