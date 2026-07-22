# LorePia localhost chaos fixture

This dependency-free tool emits deterministic provider/network fault profiles
and a machine-readable receipt. It binds only to `127.0.0.1`. Receipts retain
request shape (method, byte counts, header names and credential-presence
booleans), never URL text, header values, secrets, or request bodies.

```sh
node tools/lorepia-chaos/chaos.mjs plan --seed 42 --profile fragmented-sse
node tools/lorepia-chaos/chaos.mjs serve --seed 42 --profile http-429 \
  --receipt /tmp/lorepia-chaos-receipt.json --lifetime-ms 5000
node --test tools/lorepia-chaos/chaos.test.mjs
```

The Rust provider-runtime tests use their own in-process loopback peer so CI
does not need an external process or network. This CLI exists for repeatable
manual/black-box app runs with the same fault vocabulary.

## Allocation boundary

- SSE and NDJSON frame limits are enforced while bytes are accumulated, before
  a complete hostile frame is retained or parsed as JSON.
- `Content-Encoding` other than absent/`identity` is rejected before body
  consumption. This keeps decompression outside the trusted allocation path.
- HTTP/2 header-list bytes are bounded in the HTTP client before delivery.
  HTTP/1 headers are additionally checked immediately after reqwest/hyper has
  parsed them. reqwest 0.13 does not expose hyper's HTTP/1 pre-allocation
  header limits, so this second check is explicitly post-allocation; the
  upstream parser remains the first boundary.
- The `oversized-frame` fixture describes 100 MiB but streams bounded blocks;
  neither the fixture nor the runtime constructs a 100 MiB contiguous buffer.

The runtime disables reqwest's implicit retry policy. `RetryDecision` describes
what a caller may do as a separately authorized new request; it never replays a
partially observed streaming POST.
