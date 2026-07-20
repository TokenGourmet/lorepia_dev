# Native LLM provider runtime

This record defines the UI-independent provider connection slice extracted into
the product. It is an implementation record, not physical-device evidence and
not a claim that every authentication flow is complete.

## Product command boundary

Only the trusted `main` WebView capability can invoke the native provider
commands. No provider request uses browser `fetch`, and the production CSP does
not need a remote `connect-src` exception.

The credential surface is deliberately write-only from the WebView's point of
view:

- `get_provider_credential_status`
- `save_provider_api_key`
- `delete_provider_credential`

There is no command that returns a stored secret. The native stream command
loads it directly from the OS store and drops zeroizing buffers after use.
OpenAI, Anthropic, DeepSeek, Ollama Cloud, and Gemini Developer API keys use
fixed product-owned accounts. Vertex AI rejects API-key storage because it
requires a native Google OAuth access-token and refresh flow; until that flow
exists, `start_provider_stream` returns `VERTEX_OAUTH_NOT_CONFIGURED`.

The stream surface is:

- `start_provider_stream`
- `ack_provider_stream`
- `cancel_provider_stream`
- `get_provider_stream_snapshot`

Start returns a native-generated request ID and a separate random control token.
ACK, cancellation, and snapshot requests require both. This binds control
messages to the caller that started the request instead of treating a guessed
request ID as authority.

For the first-chat slice, start accepts only a provider/model profile, one user
message, and the event channel. Native code constructs the fixed system prompt,
provider-default options, 512-token output cap, and official endpoint. The
WebView cannot pass a raw `ProviderRequest` or endpoint selection through this
product command. Prompt-preset sessions remain closed until native-owned preset
IDs and bindings can use the exact-token prompt runtime.

## Native transport

`lorepia-provider-runtime` accepts a typed `ProviderRequest`, compiles it
internally, attaches a native-only credential, and makes one non-retried HTTPS
POST. It normalizes OpenAI, Anthropic, DeepSeek, Gemini/Vertex SSE and Ollama
Cloud NDJSON into text, reasoning, refusal, usage, and terminal events.

The runtime fixes these resource and network boundaries:

- 5 MiB maximum serialized request body;
- 256 KiB maximum SSE event or NDJSON line;
- 64 KiB maximum HTTP error body, which is discarded and never reflected;
- bounded DNS, connect, response-header, idle, and overall timeouts;
- system proxy disabled and redirects denied;
- provider-controlled errors and response identifiers bounded before IPC;
- no automatic retry of a generation POST.

The HTTP event channel has capacity one, so a stalled product adapter stops
reading the response body and propagates backpressure toward the socket.

## Endpoint override

An override is an exact compatible HTTPS endpoint URL, not a generation
parameter and not a replacement for provider wire-format validation. Selecting
one explicitly scopes the credential to its exact DNS host.

This remains a runtime-library contract and is not exposed by the first-chat
product command, which always selects the official endpoint.

The runtime rejects HTTP, userinfo, fragments, non-443 ports, IP literals,
localhost and local-name suffixes, and query keys that look like embedded
credentials. Before sending a credential, it resolves the host, rejects the
whole request if any DNS answer is non-public, and pins the accepted addresses
into a no-proxy, no-redirect client. Imported settings do not silently turn an
official endpoint into an override; the caller must select the override for
that request.

## Tauri Channel boundary

Every product event is serialized and checked against a 4096-byte budget,
below Tauri 2.11's large-payload fetch-queue path. Provider text is split at
UTF-8 boundaries into at most 512 raw bytes per event, including a worst-case
JSON-escaping test.

At most four ordinary events may be unacknowledged. A terminal event has one
reserved slot so cancel/failure can finish even when the ordinary window is
full. ACK is cumulative and monotonic. Once cancellation is accepted, the same
state lock prevents any later delta/completed/failed event; exactly one
`cancelled` terminal is emitted. Terminal state remains available for a bounded
snapshot/ACK window and is then evicted.

## Tool and MCP boundary

`lorepia-tool-runtime` now owns the non-network security contract for tools:

- deny-all by default;
- explicit app-owned allowlist;
- closed, bounded JSON-schema subset for definitions and arguments;
- one-use HMAC approval bound to the exact call ID, tool name, arguments, and
  short expiry;
- app-owned audited executor registry with bounded results;
- remote MCP configuration limited to HTTPS public-domain endpoints and an
  opaque credential reference;
- no shell, process, arbitrary native command, inline secret, localhost/LAN,
  IP-literal, or stdio executor in the five-OS common contract.

This is not yet an MCP network client. The product request compiler still sends
no tool definitions, and every unexpected provider tool event fails closed as
`UNEXPECTED_TOOL_EVENT`. Enabling remote MCP requires a reviewed Streamable
HTTP transport, native credential lifecycle, server discovery/call fixtures,
provider-specific tool-call round trips, and explicit user-approval UI. Those
requirements are intentionally not represented as a provider checkbox.

## Platform integration and evidence limits

The product vault selects macOS Keychain, iOS Protected Data, Windows Credential
Manager, Linux Secret Service, and Android Keystore-backed encrypted
preferences. Linux has no plaintext fallback. Android initializes `ndk-context`
from the product `MainActivity` and excludes its device-bound encrypted
preference file from cloud backup and device transfer. The iOS wrapper declares
the product keychain access group, but only a development-team-signed physical
device run can establish that entitlement works.

Current local verification covers Rust unit/contract tests, Clippy, the macOS
host build, and an `aarch64-linux-android` Rust compile. No live provider request
was made because no user credential was supplied. No physical Android/iOS,
packaged Windows/Linux, signed iOS Keychain, or real MCP runtime pass is claimed.
The five API-key settings and first-chat surfaces are now wired to this runtime,
but that source-level connection is not live-provider or physical-device
evidence. Vertex OAuth remains unavailable.
