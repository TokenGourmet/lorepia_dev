# LorePia provider request compiler

`lorepia-providers` is the UI-independent boundary between LorePia's common LLM
configuration and provider-specific HTTP request data.

```rust
use lorepia_providers::{compile_request, ProviderRequest};

fn prepare(request: &ProviderRequest) {
    let compiled = compile_request(request).expect("validated provider request");
    // A reviewed native transport later supplies the credential and sends
    // `compiled`; this crate performs no network or credential I/O.
    drop(compiled);
}
```

The compiler owns:

- the exact OpenAI, Anthropic, DeepSeek, Ollama Cloud, Gemini Developer API, and
  Vertex AI origins and request paths;
- provider-level capability discovery and rejection of unsupported controls;
- bounded message, schema, cache-resource, and additional-parameter validation;
- provider-specific JSON envelopes and streaming protocol metadata;
- optional merging of consecutive equal conversation roles, with Gemini and
  Vertex alternation enforced before compilation.

It deliberately does not own:

- UI, settings persistence, model discovery, or default model selection;
- API keys, OAuth tokens, keychain access, or HTTP execution;
- SSE/NDJSON decoding, cancellation, retry, or backpressure;
- MCP discovery or execution, arbitrary endpoints, local Ollama, or LAN access;
- authoritative token counting. A tokenizer override is local-estimation
  metadata and is never included in the provider body.

Provider-level acceptance does not prove that every model accepts the option.
The later transport/model-metadata layer must narrow model-specific reasoning
levels, token limits, top-k support, and service tiers without silently dropping
the user's setting.
