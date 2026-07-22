# Audio-playback M-1 fixture

`m1-audio-v1.wav` is a self-authored, fixed local-media fixture for the
LorePia M-1 playback probe. The fixture is offered under `CC0-1.0`; it contains
no copied recording, speech, user content, or upstream implementation.

The canonical asset is `../static/fixtures/m1-audio-v1.wav`. It is a 12-second,
mono, signed 16-bit little-endian PCM WAV at 48 kHz. Four three-second tonal
segments use 440, 660, 880, and 1,100 Hz respectively. Every segment has a
20 ms fade-in and fade-out, including both sides of each frequency boundary.
The fixed seek checkpoint is 6,000 ms.

| Field | Pinned value |
|---|---|
| SHA-256 | `8559ab4de943a983094b3e27af499ee5fbff80d48263a36fa3c0d1e1339ead25` |
| Bytes | `1,152,044` |
| Format | `wav-pcm-s16le` |
| Duration | `12,000 ms` |
| Sample rate | `48,000 Hz` |
| Channels | `1` |
| Bits per sample | `16` |
| Seek checkpoint | `6,000 ms` |
| Fixture license | `CC0-1.0` |

The generator uses integer phase steps and an integer rational sine
approximation instead of `Math.sin`. This keeps the bytes independent of a
platform's math library. [`catalog.json`](catalog.json) pins the public path,
format, timing, byte size, and SHA-256 used by runtime receipts and tests.

From `spikes/audio-playback`:

```sh
node scripts/generate-audio-fixture.mjs
node scripts/generate-audio-fixture.mjs --check
node scripts/verify-built-fixture.mjs --check
```

The last command checks `build/fixtures/m1-audio-v1.wav` by default. It accepts
an alternate build directory as its final argument. Verification requires the
exact pinned WAV and rejects every other recognized audio file in the build.

The generated tone makes output easy to identify, but a successful hash or
frontend build is not physical playback evidence. Audible output, lifecycle
behavior, and resource release still require the target-specific M-1 run.
