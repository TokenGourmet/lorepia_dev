# Provider/network chaos evidence

The automated boundary is a raw loopback HTTP/TLS peer in
`src/network_fault_tests.rs`; it never contacts a provider or the public
Internet. The same vocabulary is available to black-box app runs through
`tools/lorepia-chaos`.

| ID | Automated evidence | Remaining platform evidence |
|---|---|---|
| NET-001 | LF SSE through real loopback socket | None |
| NET-002 | CRLF, comments, blank lines, multiline `data` | None |
| NET-003 | one-byte writes plus every byte boundary unit corpus | TCP may coalesce writes, so the framer corpus is the exact split proof |
| NET-004 | malformed JSON fails immediately; later frames are not recovered | None |
| NET-005 | missing and incorrect content type rejected | None |
| NET-006 | 100 MiB hostile source is streamed; runtime closes at bounded frame limit | RSS evidence is covered by process-level soak, not this unit |
| NET-007 | header count and byte ceilings | HTTP/1 count/bytes check is post-parse; see allocation boundary below |
| NET-008 | gzip, deflate, brotli and zstd rejected before body read | None |
| NET-009 | 301/302/307/308 not followed; destination receives zero requests | None |
| NET-010 | credential sentinel absent from URL, debug, errors and receipts | OS-native logging inspection remains a release-device check |
| NET-011 | injected pending resolver proves independent DNS deadline | OS resolver integration remains in the five-OS run |
| NET-012 | stalled connector/TLS handshake obeys connector deadline | a true TCP black-hole is a network-lab check |
| NET-013 | self-signed TLS server is rejected | OS trust-store variants remain in the five-OS run |
| NET-014 | IPv6-only loopback request (conditional when IPv6 unavailable) | carrier/NAT64 evidence remains device-only |
| NET-015 | production client hard-disables ambient proxy discovery with `no_proxy` | PAC/VPN products require OS/network-lab evidence |
| NET-016 | reset plus explicit new-request behavior exercises route loss | real Wi-Fi/cellular handoff remains device-only |
| NET-017 | closed port then same-address healthy server; recovery requires a new request | real radio offline/online remains device-only |
| NET-018 | 429 delta-seconds becomes bounded typed `RetryAfter` | HTTP-date falls back to typed exponential policy |
| NET-019 | 500/502/503 become typed exponential decisions; hit count stays one | caller scheduling/jitter is a product integration test |
| NET-020 | header stall and idle body have distinct deadlines | None |
| NET-021 | mid-body reset fails with stable transport error and no replay | None |
| NET-022 | EOF without terminal marker fails closed | None |
| NET-023 | usage-only terminal is valid and emits no fake text | None |
| NET-024 | reasoning/text/refusal ordering remains typed | Tool events remain disabled and fail closed in decoder tests |
| NET-025 | provider error text is discarded; control text remains JSON-safe | UI escaping belongs to renderer security tests |

## Retry contract

`RetryDecision` is only a hint for a separately authorized new request. The
reqwest implicit retry policy is disabled, redirects are disabled, and the
provider runtime never replays a streaming POST. A partially delivered stream
must therefore terminate and remain visible as partial/failed state at the
product layer.

## Allocation boundary

- SSE/NDJSON framers check the partial line/event length before extending their
  retained buffer and before JSON parsing.
- Compressed responses are rejected from headers before body consumption, so
  no decompressor can allocate ahead of the frame ceiling.
- HTTP/2 header-list size is supplied to the HTTP stack before parsing.
- reqwest 0.13 does not expose hyper's HTTP/1 pre-allocation count/buffer knobs.
  LorePia checks HTTP/1 headers immediately after parsing (64 headers and 16 KiB
  by default), while hyper's own parser remains the earlier upstream boundary.
  This distinction is intentional and must not be reported as a pre-allocation
  proof.
