# M0 LLM provider catalog

This record defines the product-owned LLM provider catalog and its UI-independent
request-compilation contract. Five API-key providers can be authenticated and
called through the native runtime. The settings and first-chat path are wired
for those five providers. Vertex remains blocked until the native Google OAuth
flow is implemented.

## Included providers

| Product ID | Display name | Authentication contract | Network target |
|---|---|---|---|
| `openai` | OpenAI | API key | `api.openai.com` |
| `anthropic` | Anthropic | API key | `api.anthropic.com` |
| `deepseek` | DeepSeek | API key | `api.deepseek.com` |
| `ollama-cloud` | Ollama Cloud | API key | `ollama.com` |
| `google-gemini` | Google Gemini | Gemini Developer API key | `generativelanguage.googleapis.com` |
| `google-vertex-ai` | Vertex AI Gemini | Google Cloud OAuth, project, and location | `aiplatform.googleapis.com` service domain |

Gemini Developer API and Vertex AI are separate providers because their
authentication, billing, project, location, and endpoint contracts differ.
Ollama Cloud is also separate from a local Ollama daemon. This catalog does not
open a localhost, LAN, or user-supplied base URL.

The catalog and request compiler follow the providers' current official
documentation:

- [OpenAI Responses API](https://developers.openai.com/api/reference/resources/responses/methods/create)
- [Anthropic Messages API](https://platform.claude.com/docs/en/api/messages/create)
- [DeepSeek chat completions](https://api-docs.deepseek.com/api/create-chat-completion/)
- [Ollama Cloud](https://docs.ollama.com/cloud) and [chat API](https://docs.ollama.com/api/chat)
- [Gemini Developer generateContent](https://ai.google.dev/api/generate-content)
- [Vertex AI generateContent](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/projects.locations.publishers.models/generateContent)

## Headless request compiler

The `lorepia-providers` Rust crate owns provider IDs, capability discovery,
bounded configuration validation, role normalization, and wire-request
compilation. It has no UI dependency, does no I/O, accepts no credential, and
never chooses a provider or model on the user's behalf.

Its common generation contract covers temperature, maximum output tokens,
top-p, top-k, presence and frequency penalties, stop sequences, seed, text or
JSON output, and optional merging of consecutive equal roles. Gemini and Vertex
requests with consecutive equal roles are rejected unless that merge policy is
selected. Each option is accepted only where the selected provider supports
it. A provider-specific contract then covers:

- OpenAI reasoning, reasoning summary, prompt-cache key and retention, and
  service tier;
- Anthropic thinking display, thinking budget, reasoning effort, ephemeral
  cache TTL, service tier, and JSON Schema output;
- DeepSeek thinking and reasoning effort, including rejection of temperature
  or top-p when thinking mode would silently ignore them;
- Ollama Cloud thinking levels and its native `options` envelope;
- Gemini thinking levels or budgets (including model-dependent `-1` dynamic and
  `0` disabled modes), thought-summary inclusion, exactly scoped cached content,
  safety settings, and service tier;
- Vertex AI project, location, thinking, scoped cached content, safety method,
  Vertex-only jailbreak category, and shared/dedicated throughput request
  header. That throughput selector is intentionally distinct from a service
  tier.

The compiler fixes the official origin and path for every provider, declares
the required authentication scheme without carrying a secret, emits SSE or
NDJSON stream metadata, and keeps a tokenizer override as local estimation
metadata rather than sending it upstream. Model-specific limits can still be
narrower than these provider-level contracts and must later be checked against
fresh model metadata.

`additionalParameters` is a deliberately restricted top-level escape hatch. It
is bounded by item count, encoded size, key syntax, and nesting depth. Typed
generation fields, authentication material, endpoint or header controls,
project/location selectors, tools, MCP keys, server-side state controls, and
logging controls are denied, including in nested objects. OpenAI and Gemini
requests explicitly compile with `store: false`. Built-in providers have no
endpoint-override field.

## Current product boundary

The settings screen can select one of the six catalog entries, save an API key
through the write-only native vault, and manually enter a model ID. The
selection is volatile and is not persisted. Its profile draft contains only:

- provider ID and model ID for API-key providers;
- provider ID, model ID, Google Cloud project ID, and location for Vertex AI.

The draft has no API key, access token, secret, service-account document,
credential reference, custom endpoint, or hard-coded default model. Model names
and limits change independently of an app release, so a later native connection
must query the provider and retain a manual model-ID fallback instead of
shipping a supposedly permanent model list.

The production WebView CSP still limits `connect-src` to `'self'`; provider
traffic uses the native Rust HTTP client. The product now exposes write-only
credential status/save/delete commands and an authenticated start/ACK/cancel/
snapshot stream command family. The stream runtime internally compiles the
typed request, loads the provider key without returning it to the WebView, and
normalizes the five wire protocols described in
[`provider-runtime.md`](provider-runtime.md).

The first-chat path activates only after native credential status is positive
and a bounded model ID is present. The WebView sends only that non-secret
profile and the current user message. The narrow native command constructs the
fixed system message, provider defaults, official endpoint, and 512-token output
limit; callers cannot submit a raw provider request or endpoint. It does not
bind prompt presets, personas, memory, tools, endpoint overrides, tokenizer
overrides, or additional parameters. Channel deltas are applied and then
acknowledged in sequence. Terminal results are published only after their ACK
and authenticated snapshot agree, and a bounded snapshot poll recovers a lost
terminal callback. The native control token remains inside the stream adapter.

There is still no provider profile database or model-list cache, so a restart
requires selecting the profile again. Vertex compilation does not imply a
working Vertex login; the stream command rejects it until a native OAuth flow
exists.

## Implemented connection gates and remaining evidence

The native slice now implements the source-level parts of the earlier gate:

1. five-OS credential backends, native-only reads, zeroizing buffers, and
   redacted error codes;
2. provider-specific native HTTPS request and SSE/NDJSON decoding;
3. official target validation plus an explicit HTTPS override with public-DNS
   validation, address pinning, no proxy, and no redirects;
4. bounded request/frame/error sizes, timeouts, cancellation, and no automatic
   generation retry;
5. 4096-byte Tauri events with request-control ownership, sequence, cumulative
   ACK, four-event backpressure, and a reserved terminal slot.

Still open are verified model metadata/cache persistence, the native Vertex
OAuth flow, live requests against user-owned test accounts, physical
Android/iOS evidence, signed iOS Keychain evidence, and packaged Windows/Linux
runtime evidence.

MCP remains a separate security boundary. A deny-by-default tool policy,
call-bound one-use approval, bounded schema/argument validation, executor
registry, and safe remote-server configuration contract now exist, but there is
no MCP network transport and no provider tool-call loop. Endpoint override is a
native transport selection and never enters the provider body or
`additionalParameters`.

Vertex AI must not accept a long-lived service-account JSON file in the desktop
or mobile product. Local Ollama, if added later, needs its own network and policy
review because mobile localhost is the mobile device itself and LAN access adds
local-network permissions and SSRF exposure.

## Verification

`src/lib/providers/catalog.test.ts` fixes the exact six-entry catalog, the
Gemini/Vertex split, the Ollama Cloud HTTPS target, the absence of custom
endpoints and hard-coded models, and the non-secret draft shape.

`src/routes/settings/provider-surface.test.ts` fixes the settings surface's
catalog use and proves that secrets flow only through the write-only native
vault, never browser persistence or `fetch`.

`src/lib/providers/first-chat-request.test.ts` fixes the five-provider narrow
command shape and absence of raw prompt/transport controls. Native
`provider_stream` tests fix the actual system message and request defaults.
`stream.test.ts` covers events arriving before start resolves,
apply-before-ACK ordering, monotonically serialized ACKs, exact cancellation
ownership, forged identity rejection, fixed public errors, lost-terminal
recovery, and terminal snapshot cleanup.

`crates/lorepia-providers` unit tests fix all six official targets and request
envelopes, role merging, provider-option mismatches, JSON-format differences,
thinking-mode conflicts, cache scoping, safe extra parameters, tokenizer
separation, and the absence of embedded credential values.
