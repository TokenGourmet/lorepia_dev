use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::{
    AnthropicOptions, AnthropicThinking, Capability, CapabilitySupport, ChatMessage,
    DeepSeekOptions, GenerationOptions, GoogleOptions, GoogleSafetySetting, GoogleThinking,
    MessageRole, OllamaCloudOptions, OllamaThinking, OpenAiOptions, ProviderConfigError,
    ProviderId, ProviderOptions, ProviderRequest, ResponseFormat, Result, RolePolicy,
    TokenizerOverride, VertexAiOptions, VertexRequestType, capability_support,
};

const MAX_MODEL_ID_BYTES: usize = 256;
const MAX_MESSAGE_COUNT: usize = 10_000;
const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_STOP_SEQUENCE_BYTES: usize = 256;
const MAX_ADDITIONAL_PARAMETERS: usize = 16;
const MAX_ADDITIONAL_PARAMETERS_BYTES: usize = 8 * 1024;
const MAX_ADDITIONAL_PARAMETER_DEPTH: usize = 4;
const MAX_JSON_SCHEMA_BYTES: usize = 16 * 1024;
const MAX_JSON_SCHEMA_DEPTH: usize = 16;
const MAX_TOKENIZER_ID_BYTES: usize = 128;
const MAX_CACHE_RESOURCE_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthScheme {
    AuthorizationBearer,
    AnthropicXApiKey,
    GoogleXGoogApiKey,
    GoogleOAuthBearer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamProtocol {
    Sse,
    Ndjson,
}

/// A provider request whose endpoint and wire shape have been validated.
///
/// This type deliberately contains no credential. The native transport must
/// resolve a credential reference and attach it according to `auth_scheme`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledProviderRequest {
    provider: ProviderId,
    origin: String,
    path_and_query: String,
    auth_scheme: AuthScheme,
    static_headers: BTreeMap<String, String>,
    stream_protocol: StreamProtocol,
    body: Value,
    /// Tokenizer overrides affect local estimates only and are never sent to a
    /// provider.
    tokenizer_override: Option<TokenizerOverride>,
}

impl CompiledProviderRequest {
    /// Provider selected by the validated product request.
    #[must_use]
    pub const fn provider(&self) -> ProviderId {
        self.provider
    }

    /// Product-owned HTTPS origin. Callers cannot construct this type or
    /// replace the origin after compilation.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    #[must_use]
    pub fn path_and_query(&self) -> &str {
        &self.path_and_query
    }

    #[must_use]
    pub const fn auth_scheme(&self) -> AuthScheme {
        self.auth_scheme
    }

    #[must_use]
    pub fn static_headers(&self) -> &BTreeMap<String, String> {
        &self.static_headers
    }

    #[must_use]
    pub const fn stream_protocol(&self) -> StreamProtocol {
        self.stream_protocol
    }

    #[must_use]
    pub fn body(&self) -> &Value {
        &self.body
    }

    #[must_use]
    pub fn tokenizer_override(&self) -> Option<&TokenizerOverride> {
        self.tokenizer_override.as_ref()
    }
}

#[derive(Debug)]
struct NormalizedConversation {
    system: Option<String>,
    messages: Vec<ChatMessage>,
}

pub fn compile_request(request: &ProviderRequest) -> Result<CompiledProviderRequest> {
    validate_request(request)?;
    let conversation = normalize_conversation(&request.messages, request.generation.role_policy);

    let mut compiled = match &request.provider_options {
        ProviderOptions::OpenAi(options) => compile_openai(request, options, &conversation),
        ProviderOptions::Anthropic(options) => compile_anthropic(request, options, &conversation),
        ProviderOptions::DeepSeek(options) => compile_deepseek(request, options, &conversation),
        ProviderOptions::OllamaCloud(options) => {
            compile_ollama_cloud(request, options, &conversation)
        }
        ProviderOptions::GoogleGemini(options) => {
            compile_google_gemini(request, options, &conversation)
        }
        ProviderOptions::GoogleVertexAi(options) => {
            compile_google_vertex(request, options, &conversation)
        }
    }?;

    merge_additional_parameters(&mut compiled.body, &request.additional_parameters)?;
    compiled
        .tokenizer_override
        .clone_from(&request.tokenizer_override);
    Ok(compiled)
}

fn validate_request(request: &ProviderRequest) -> Result<()> {
    let options_provider = request.provider_options.provider_id();
    if request.provider != options_provider {
        return Err(ProviderConfigError::ProviderOptionsMismatch {
            request_provider: request.provider,
            options_provider,
        });
    }

    validate_text_field("modelId", &request.model_id, MAX_MODEL_ID_BYTES)?;
    if matches!(
        request.provider,
        ProviderId::GoogleGemini | ProviderId::GoogleVertexAi
    ) {
        validate_path_segment("modelId", &request.model_id, false)?;
    }

    validate_messages(&request.messages)?;
    validate_generation(request.provider, &request.generation)?;
    validate_role_contract(request)?;
    validate_provider_options(request)?;
    validate_tokenizer_override(request.tokenizer_override.as_ref())?;
    validate_additional_parameters(&request.additional_parameters)?;
    Ok(())
}

fn validate_messages(messages: &[ChatMessage]) -> Result<()> {
    if messages.len() > MAX_MESSAGE_COUNT {
        return Err(ProviderConfigError::TooManyItems {
            field: "messages",
            max: MAX_MESSAGE_COUNT,
        });
    }

    let mut total_bytes = 0usize;
    let mut has_conversation = false;
    let mut conversation_started = false;
    for message in messages {
        if message.content.trim().is_empty() {
            return Err(ProviderConfigError::EmptyField("message.content"));
        }
        if message.content.chars().any(|character| character == '\0') {
            return Err(ProviderConfigError::InvalidField {
                field: "message.content",
                reason: "NUL characters are not allowed",
            });
        }
        total_bytes = total_bytes.saturating_add(message.content.len());
        if message.role == MessageRole::System && conversation_started {
            return Err(ProviderConfigError::InvalidField {
                field: "messages.role",
                reason: "system messages must precede the conversation",
            });
        }
        if message.role != MessageRole::System {
            conversation_started = true;
            has_conversation = true;
        }
    }

    if total_bytes > MAX_MESSAGE_BYTES {
        return Err(ProviderConfigError::PayloadTooLarge {
            field: "messages",
            max_bytes: MAX_MESSAGE_BYTES,
        });
    }
    if !has_conversation {
        return Err(ProviderConfigError::MissingConversation);
    }
    Ok(())
}

fn validate_generation(provider: ProviderId, generation: &GenerationOptions) -> Result<()> {
    if let Some(value) = generation.temperature {
        require_capability(provider, Capability::Temperature)?;
        if !value.is_finite() {
            return Err(invalid_number("temperature"));
        }
        let maximum = match provider {
            ProviderId::Anthropic => 1.0,
            ProviderId::OllamaCloud => 10.0,
            ProviderId::OpenAi
            | ProviderId::DeepSeek
            | ProviderId::GoogleGemini
            | ProviderId::GoogleVertexAi => 2.0,
        };
        validate_float_range("temperature", value, 0.0, maximum)?;
    }

    if let Some(maximum) = generation.max_output_tokens {
        require_capability(provider, Capability::MaxOutputTokens)?;
        if maximum == 0 && provider != ProviderId::Anthropic {
            return Err(ProviderConfigError::InvalidField {
                field: "maxOutputTokens",
                reason: "must be greater than zero",
            });
        }
    }

    if let Some(value) = generation.top_p {
        require_capability(provider, Capability::TopP)?;
        validate_float_range("topP", value, 0.0, 1.0)?;
    }
    if generation.top_k.is_some() {
        require_capability(provider, Capability::TopK)?;
    }
    if let Some(value) = generation.presence_penalty {
        require_capability(provider, Capability::PresencePenalty)?;
        validate_float_range("presencePenalty", value, -2.0, 2.0)?;
    }
    if let Some(value) = generation.frequency_penalty {
        require_capability(provider, Capability::FrequencyPenalty)?;
        validate_float_range("frequencyPenalty", value, -2.0, 2.0)?;
    }

    if !generation.stop_sequences.is_empty() {
        require_capability(provider, Capability::StopSequences)?;
        let maximum = match provider {
            ProviderId::GoogleGemini => 5,
            ProviderId::DeepSeek => 16,
            ProviderId::Anthropic | ProviderId::OllamaCloud | ProviderId::GoogleVertexAi => 16,
            ProviderId::OpenAi => unreachable!("capability check rejects OpenAI stop sequences"),
        };
        if generation.stop_sequences.len() > maximum {
            return Err(ProviderConfigError::TooManyItems {
                field: "stopSequences",
                max: maximum,
            });
        }
        for sequence in &generation.stop_sequences {
            validate_text_field("stopSequences", sequence, MAX_STOP_SEQUENCE_BYTES)?;
        }
    }

    if generation.seed.is_some() {
        require_capability(provider, Capability::Seed)?;
    }

    validate_response_format(provider, &generation.response_format)
}

fn validate_role_contract(request: &ProviderRequest) -> Result<()> {
    if !matches!(
        request.provider,
        ProviderId::GoogleGemini | ProviderId::GoogleVertexAi
    ) || request.generation.role_policy == RolePolicy::MergeConsecutive
    {
        return Ok(());
    }

    let mut previous = None;
    for message in request
        .messages
        .iter()
        .filter(|message| message.role != MessageRole::System)
    {
        if previous == Some(message.role) {
            return Err(ProviderConfigError::InvalidField {
                field: "messages.role",
                reason: "Gemini user and model roles must alternate; select merge_consecutive or normalize the history",
            });
        }
        previous = Some(message.role);
    }
    Ok(())
}

fn validate_response_format(provider: ProviderId, format: &ResponseFormat) -> Result<()> {
    match format {
        ResponseFormat::Text => Ok(()),
        ResponseFormat::JsonObject => {
            require_capability(provider, Capability::JsonResponse)?;
            if provider == ProviderId::Anthropic {
                return Err(ProviderConfigError::InvalidField {
                    field: "responseFormat",
                    reason: "Anthropic structured output requires a JSON schema",
                });
            }
            Ok(())
        }
        ResponseFormat::JsonSchema { schema } => {
            require_capability(provider, Capability::JsonResponse)?;
            if provider == ProviderId::DeepSeek {
                return Err(ProviderConfigError::InvalidField {
                    field: "responseFormat",
                    reason: "DeepSeek supports json_object but not json_schema",
                });
            }
            validate_json_schema(schema)
        }
    }
}

fn validate_json_schema(schema: &Value) -> Result<()> {
    if !schema.is_object() {
        return Err(ProviderConfigError::InvalidField {
            field: "responseFormat.schema",
            reason: "schema must be a JSON object",
        });
    }
    let bytes = serde_json::to_vec(schema).map_err(|_| ProviderConfigError::InvalidField {
        field: "responseFormat.schema",
        reason: "schema is not valid JSON",
    })?;
    if bytes.len() > MAX_JSON_SCHEMA_BYTES {
        return Err(ProviderConfigError::PayloadTooLarge {
            field: "responseFormat.schema",
            max_bytes: MAX_JSON_SCHEMA_BYTES,
        });
    }
    if value_depth(schema) > MAX_JSON_SCHEMA_DEPTH {
        return Err(ProviderConfigError::InvalidField {
            field: "responseFormat.schema",
            reason: "schema nesting exceeds the local safety limit",
        });
    }
    Ok(())
}

fn validate_provider_options(request: &ProviderRequest) -> Result<()> {
    match &request.provider_options {
        ProviderOptions::OpenAi(options) => validate_openai_options(options),
        ProviderOptions::Anthropic(options) => {
            validate_anthropic_options(options, request.generation.max_output_tokens)
        }
        ProviderOptions::DeepSeek(options) => {
            validate_deepseek_options(options, &request.generation)
        }
        ProviderOptions::OllamaCloud(_) => Ok(()),
        ProviderOptions::GoogleGemini(options) => validate_google_options(options),
        ProviderOptions::GoogleVertexAi(options) => validate_vertex_options(options),
    }
}

fn validate_openai_options(options: &OpenAiOptions) -> Result<()> {
    if let Some(cache) = &options.prompt_cache {
        validate_text_field("promptCache.key", &cache.key, 128)?;
    }
    Ok(())
}

fn validate_anthropic_options(
    options: &AnthropicOptions,
    max_output_tokens: Option<u32>,
) -> Result<()> {
    let maximum = max_output_tokens.ok_or(ProviderConfigError::MissingField(
        "generation.maxOutputTokens",
    ))?;
    if let Some(AnthropicThinking::Enabled { budget_tokens, .. }) = options.thinking {
        if budget_tokens < 1_024 {
            return Err(ProviderConfigError::InvalidField {
                field: "thinking.budgetTokens",
                reason: "Anthropic thinking budget must be at least 1024",
            });
        }
        if budget_tokens >= maximum {
            return Err(ProviderConfigError::InvalidField {
                field: "thinking.budgetTokens",
                reason: "Anthropic thinking budget must be lower than maxOutputTokens",
            });
        }
    }
    Ok(())
}

fn validate_deepseek_options(
    options: &DeepSeekOptions,
    generation: &GenerationOptions,
) -> Result<()> {
    if options.thinking_enabled != Some(false)
        && (generation.temperature.is_some() || generation.top_p.is_some())
    {
        return Err(ProviderConfigError::InvalidField {
            field: "thinkingEnabled",
            reason: "DeepSeek ignores temperature and topP in thinking mode",
        });
    }
    if options.thinking_enabled == Some(false) && options.reasoning_effort.is_some() {
        return Err(ProviderConfigError::InvalidField {
            field: "reasoningEffort",
            reason: "reasoning effort cannot be used when thinking is disabled",
        });
    }
    Ok(())
}

fn validate_google_options(options: &GoogleOptions) -> Result<()> {
    if let Some(thinking) = &options.thinking {
        validate_google_thinking(thinking)?;
    }
    validate_cache_resource("cachedContent", options.cached_content.as_deref())?;
    if let Some(resource) = options.cached_content.as_deref() {
        let segments = resource.split('/').collect::<Vec<_>>();
        if segments.len() != 2 || segments[0] != "cachedContents" || segments[1].is_empty() {
            return Err(ProviderConfigError::InvalidField {
                field: "cachedContent",
                reason: "Gemini cache resource must be exactly cachedContents/{id}",
            });
        }
        validate_path_segment("cachedContent.id", segments[1], false)?;
    }
    validate_safety_settings(&options.safety_settings, false)
}

fn validate_vertex_options(options: &VertexAiOptions) -> Result<()> {
    validate_path_segment("projectId", &options.project_id, true)?;
    validate_path_segment("location", &options.location, true)?;
    if let Some(thinking) = &options.thinking {
        validate_google_thinking(thinking)?;
    }
    validate_cache_resource("cachedContent", options.cached_content.as_deref())?;
    if let Some(resource) = options.cached_content.as_deref() {
        let segments = resource.split('/').collect::<Vec<_>>();
        if segments.len() != 6
            || segments[0] != "projects"
            || segments[1] != options.project_id
            || segments[2] != "locations"
            || segments[3] != options.location
            || segments[4] != "cachedContents"
            || segments[5].is_empty()
        {
            return Err(ProviderConfigError::InvalidField {
                field: "cachedContent",
                reason: "Vertex cache resource must exactly match the selected project and location",
            });
        }
        validate_path_segment("cachedContent.id", segments[5], false)?;
    }
    validate_safety_settings(&options.safety_settings, true)
}

fn validate_google_thinking(thinking: &GoogleThinking) -> Result<()> {
    if thinking.level.is_some() && thinking.budget_tokens.is_some() {
        return Err(ProviderConfigError::InvalidField {
            field: "thinking",
            reason: "thinking level and token budget are mutually exclusive",
        });
    }
    if thinking.budget_tokens.is_some_and(|budget| budget < -1) {
        return Err(ProviderConfigError::InvalidField {
            field: "thinking.budgetTokens",
            reason: "must be -1, 0, or a positive model-supported budget",
        });
    }
    if thinking.level.is_none() && thinking.budget_tokens.is_none() && !thinking.include_thoughts {
        return Err(ProviderConfigError::InvalidField {
            field: "thinking",
            reason: "empty thinking configuration has no effect",
        });
    }
    Ok(())
}

fn validate_safety_settings(settings: &[GoogleSafetySetting], vertex: bool) -> Result<()> {
    let maximum = if vertex { 6 } else { 5 };
    if settings.len() > maximum {
        return Err(ProviderConfigError::TooManyItems {
            field: "safetySettings",
            max: maximum,
        });
    }
    let mut categories = BTreeSet::new();
    for setting in settings {
        if !categories.insert(setting.category.as_str()) {
            return Err(ProviderConfigError::InvalidField {
                field: "safetySettings",
                reason: "each harm category may appear only once",
            });
        }
        if !vertex && setting.method.is_some() {
            return Err(ProviderConfigError::InvalidField {
                field: "safetySettings.method",
                reason: "safety method is a Vertex AI option",
            });
        }
        if !vertex && setting.category == crate::GoogleSafetyCategory::Jailbreak {
            return Err(ProviderConfigError::InvalidField {
                field: "safetySettings.category",
                reason: "jailbreak is a Vertex AI safety category",
            });
        }
    }
    Ok(())
}

fn validate_tokenizer_override(override_config: Option<&TokenizerOverride>) -> Result<()> {
    let Some(override_config) = override_config else {
        return Ok(());
    };
    validate_text_field(
        "tokenizerOverride.tokenizerId",
        &override_config.tokenizer_id,
        MAX_TOKENIZER_ID_BYTES,
    )?;
    if !override_config.tokenizer_id.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':' | '/')
    }) {
        return Err(ProviderConfigError::InvalidField {
            field: "tokenizerOverride.tokenizerId",
            reason: "contains unsupported characters",
        });
    }
    Ok(())
}

fn validate_additional_parameters(parameters: &BTreeMap<String, Value>) -> Result<()> {
    if parameters.len() > MAX_ADDITIONAL_PARAMETERS {
        return Err(ProviderConfigError::TooManyItems {
            field: "additionalParameters",
            max: MAX_ADDITIONAL_PARAMETERS,
        });
    }
    let bytes = serde_json::to_vec(parameters).map_err(|_| ProviderConfigError::InvalidField {
        field: "additionalParameters",
        reason: "parameters are not valid JSON",
    })?;
    if bytes.len() > MAX_ADDITIONAL_PARAMETERS_BYTES {
        return Err(ProviderConfigError::PayloadTooLarge {
            field: "additionalParameters",
            max_bytes: MAX_ADDITIONAL_PARAMETERS_BYTES,
        });
    }

    for (key, value) in parameters {
        if key.is_empty()
            || key.len() > 64
            || !key
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Err(ProviderConfigError::UnsafeExtraParameter(key.clone()));
        }
        if is_reserved_extra_parameter(key) {
            return Err(ProviderConfigError::UnsafeExtraParameter(key.clone()));
        }
        if let Some(nested_key) = find_sensitive_nested_key(value) {
            return Err(ProviderConfigError::UnsafeExtraParameter(format!(
                "{key}.{nested_key}"
            )));
        }
        if value_depth(value) > MAX_ADDITIONAL_PARAMETER_DEPTH {
            return Err(ProviderConfigError::ExtraParameterTooDeep {
                key: key.clone(),
                max_depth: MAX_ADDITIONAL_PARAMETER_DEPTH,
            });
        }
    }
    Ok(())
}

fn is_reserved_extra_parameter(key: &str) -> bool {
    let canonical = canonical_extra_parameter(key);
    const RESERVED: &[&str] = &[
        "model",
        "input",
        "messages",
        "contents",
        "system",
        "instructions",
        "systeminstruction",
        "stream",
        "streamoptions",
        "store",
        "background",
        "conversation",
        "previousresponseid",
        "safetyidentifier",
        "user",
        "tools",
        "toolchoice",
        "paralleltoolcalls",
        "mcp",
        "temperature",
        "maxoutputtokens",
        "maxtokens",
        "numpredict",
        "topp",
        "topk",
        "presencepenalty",
        "frequencypenalty",
        "stop",
        "stopsequences",
        "seed",
        "responseformat",
        "responsemimetype",
        "responsejsonschema",
        "text",
        "reasoning",
        "reasoningeffort",
        "thinking",
        "safetysettings",
        "servicetier",
        "cachedcontent",
        "cachecontrol",
        "promptcachekey",
        "promptcacheretention",
        "generationconfig",
        "options",
        "format",
        "project",
        "projectid",
        "location",
    ];
    RESERVED.contains(&canonical.as_str()) || is_sensitive_extra_parameter(key)
}

fn canonical_extra_parameter(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_sensitive_extra_parameter(key: &str) -> bool {
    let canonical = canonical_extra_parameter(key);
    const SENSITIVE_PARTS: &[&str] = &[
        "endpoint",
        "baseurl",
        "url",
        "host",
        "proxy",
        "header",
        "authorization",
        "apikey",
        "token",
        "secret",
        "password",
        "cookie",
        "credential",
        "serviceaccount",
        "tool",
        "mcp",
    ];
    SENSITIVE_PARTS.iter().any(|part| canonical.contains(part))
}

fn find_sensitive_nested_key(value: &Value) -> Option<&str> {
    match value {
        Value::Object(entries) => entries.iter().find_map(|(key, value)| {
            is_sensitive_extra_parameter(key)
                .then_some(key.as_str())
                .or_else(|| find_sensitive_nested_key(value))
        }),
        Value::Array(values) => values.iter().find_map(find_sensitive_nested_key),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn compile_openai(
    request: &ProviderRequest,
    options: &OpenAiOptions,
    conversation: &NormalizedConversation,
) -> Result<CompiledProviderRequest> {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model_id));
    body.insert(
        "input".into(),
        Value::Array(chat_messages(&conversation.messages, "assistant")),
    );
    body.insert("stream".into(), Value::Bool(true));
    body.insert("store".into(), Value::Bool(false));
    if let Some(system) = &conversation.system {
        body.insert("instructions".into(), json!(system));
    }
    insert_generation_scalar_fields(&mut body, &request.generation, GenerationDialect::OpenAi);

    match &request.generation.response_format {
        ResponseFormat::Text => {}
        ResponseFormat::JsonObject => {
            body.insert("text".into(), json!({"format": {"type": "json_object"}}));
        }
        ResponseFormat::JsonSchema { schema } => {
            body.insert(
                "text".into(),
                json!({
                    "format": {
                        "type": "json_schema",
                        "name": "lorepia_response",
                        "schema": schema,
                        "strict": true
                    }
                }),
            );
        }
    }
    if let Some(reasoning) = options.reasoning {
        let mut value = Map::new();
        value.insert("effort".into(), json!(reasoning.effort.as_str()));
        if let Some(summary) = reasoning.summary {
            value.insert("summary".into(), json!(summary.as_str()));
        }
        body.insert("reasoning".into(), Value::Object(value));
    }
    if let Some(cache) = &options.prompt_cache {
        body.insert("prompt_cache_key".into(), json!(cache.key));
        body.insert(
            "prompt_cache_retention".into(),
            json!(cache.retention.as_str()),
        );
    }
    if let Some(tier) = options.service_tier {
        body.insert("service_tier".into(), json!(tier.as_str()));
    }

    Ok(base_compiled(
        ProviderId::OpenAi,
        "https://api.openai.com",
        "/v1/responses",
        AuthScheme::AuthorizationBearer,
        StreamProtocol::Sse,
        body,
    ))
}

fn compile_anthropic(
    request: &ProviderRequest,
    options: &AnthropicOptions,
    conversation: &NormalizedConversation,
) -> Result<CompiledProviderRequest> {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model_id));
    body.insert(
        "max_tokens".into(),
        json!(
            request
                .generation
                .max_output_tokens
                .ok_or(ProviderConfigError::MissingField(
                    "generation.maxOutputTokens"
                ))?
        ),
    );
    body.insert(
        "messages".into(),
        Value::Array(chat_messages(&conversation.messages, "assistant")),
    );
    body.insert("stream".into(), Value::Bool(true));
    if let Some(system) = &conversation.system {
        body.insert("system".into(), json!(system));
    }
    insert_generation_scalar_fields(&mut body, &request.generation, GenerationDialect::Anthropic);

    if let Some(thinking) = &options.thinking {
        let value = match thinking {
            AnthropicThinking::Disabled => json!({"type": "disabled"}),
            AnthropicThinking::Adaptive { display } => {
                json!({"type": "adaptive", "display": display.as_str()})
            }
            AnthropicThinking::Enabled {
                budget_tokens,
                display,
            } => json!({
                "type": "enabled",
                "budget_tokens": budget_tokens,
                "display": display.as_str()
            }),
        };
        body.insert("thinking".into(), value);
    }

    let mut output_config = Map::new();
    if let Some(effort) = options.reasoning_effort {
        output_config.insert("effort".into(), json!(effort.as_str()));
    }
    if let ResponseFormat::JsonSchema { schema } = &request.generation.response_format {
        output_config.insert(
            "format".into(),
            json!({"type": "json_schema", "schema": schema}),
        );
    }
    if !output_config.is_empty() {
        body.insert("output_config".into(), Value::Object(output_config));
    }
    if let Some(ttl) = options.cache_ttl {
        body.insert(
            "cache_control".into(),
            json!({"type": "ephemeral", "ttl": ttl.as_str()}),
        );
    }
    if let Some(tier) = options.service_tier {
        body.insert("service_tier".into(), json!(tier.as_str()));
    }

    let mut compiled = base_compiled(
        ProviderId::Anthropic,
        "https://api.anthropic.com",
        "/v1/messages",
        AuthScheme::AnthropicXApiKey,
        StreamProtocol::Sse,
        body,
    );
    compiled
        .static_headers
        .insert("anthropic-version".into(), "2023-06-01".into());
    Ok(compiled)
}

fn compile_deepseek(
    request: &ProviderRequest,
    options: &DeepSeekOptions,
    conversation: &NormalizedConversation,
) -> Result<CompiledProviderRequest> {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model_id));
    body.insert(
        "messages".into(),
        Value::Array(messages_with_system(conversation, "assistant")),
    );
    body.insert("stream".into(), Value::Bool(true));
    body.insert("stream_options".into(), json!({"include_usage": true}));
    insert_generation_scalar_fields(&mut body, &request.generation, GenerationDialect::DeepSeek);
    if let ResponseFormat::JsonObject = request.generation.response_format {
        body.insert("response_format".into(), json!({"type": "json_object"}));
    }
    if let Some(enabled) = options.thinking_enabled {
        body.insert(
            "thinking".into(),
            json!({"type": if enabled { "enabled" } else { "disabled" }}),
        );
    }
    if let Some(effort) = options.reasoning_effort {
        body.insert("reasoning_effort".into(), json!(effort.as_str()));
    }

    Ok(base_compiled(
        ProviderId::DeepSeek,
        "https://api.deepseek.com",
        "/chat/completions",
        AuthScheme::AuthorizationBearer,
        StreamProtocol::Sse,
        body,
    ))
}

fn compile_ollama_cloud(
    request: &ProviderRequest,
    options: &OllamaCloudOptions,
    conversation: &NormalizedConversation,
) -> Result<CompiledProviderRequest> {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model_id));
    body.insert(
        "messages".into(),
        Value::Array(messages_with_system(conversation, "assistant")),
    );
    body.insert("stream".into(), Value::Bool(true));

    let mut generation_options = Map::new();
    insert_generation_scalar_fields(
        &mut generation_options,
        &request.generation,
        GenerationDialect::Ollama,
    );
    if !generation_options.is_empty() {
        body.insert("options".into(), Value::Object(generation_options));
    }
    if let Some(thinking) = options.thinking {
        let value = match thinking {
            OllamaThinking::Disabled => Value::Bool(false),
            OllamaThinking::Enabled => Value::Bool(true),
            OllamaThinking::Low => json!("low"),
            OllamaThinking::Medium => json!("medium"),
            OllamaThinking::High => json!("high"),
            OllamaThinking::Max => json!("max"),
        };
        body.insert("think".into(), value);
    }

    Ok(base_compiled(
        ProviderId::OllamaCloud,
        "https://ollama.com",
        "/api/chat",
        AuthScheme::AuthorizationBearer,
        StreamProtocol::Ndjson,
        body,
    ))
}

fn compile_google_gemini(
    request: &ProviderRequest,
    options: &GoogleOptions,
    conversation: &NormalizedConversation,
) -> Result<CompiledProviderRequest> {
    let body = compile_google_body(
        request,
        conversation,
        &options.thinking,
        &options.safety_settings,
        options.cached_content.as_deref(),
        options.service_tier.map(|tier| tier.as_str()),
        false,
    );
    Ok(base_compiled(
        ProviderId::GoogleGemini,
        "https://generativelanguage.googleapis.com",
        &format!(
            "/v1beta/models/{}:streamGenerateContent?alt=sse",
            request.model_id
        ),
        AuthScheme::GoogleXGoogApiKey,
        StreamProtocol::Sse,
        body,
    ))
}

fn compile_google_vertex(
    request: &ProviderRequest,
    options: &VertexAiOptions,
    conversation: &NormalizedConversation,
) -> Result<CompiledProviderRequest> {
    let origin = if options.location == "global" {
        "https://aiplatform.googleapis.com".to_owned()
    } else {
        format!("https://{}-aiplatform.googleapis.com", options.location)
    };
    let path = format!(
        "/v1/projects/{}/locations/{}/publishers/google/models/{}:streamGenerateContent?alt=sse",
        options.project_id, options.location, request.model_id
    );
    let body = compile_google_body(
        request,
        conversation,
        &options.thinking,
        &options.safety_settings,
        options.cached_content.as_deref(),
        None,
        true,
    );
    let mut compiled = base_compiled(
        ProviderId::GoogleVertexAi,
        &origin,
        &path,
        AuthScheme::GoogleOAuthBearer,
        StreamProtocol::Sse,
        body,
    );
    let request_type = match options.request_type {
        VertexRequestType::Automatic => None,
        VertexRequestType::Shared => Some("shared"),
        VertexRequestType::Dedicated => Some("dedicated"),
    };
    if let Some(request_type) = request_type {
        compiled
            .static_headers
            .insert("X-Vertex-AI-LLM-Request-Type".into(), request_type.into());
    }
    Ok(compiled)
}

#[allow(clippy::too_many_arguments)]
fn compile_google_body(
    request: &ProviderRequest,
    conversation: &NormalizedConversation,
    thinking: &Option<GoogleThinking>,
    safety_settings: &[GoogleSafetySetting],
    cached_content: Option<&str>,
    service_tier: Option<&str>,
    vertex: bool,
) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert(
        "contents".into(),
        Value::Array(google_messages(&conversation.messages)),
    );
    if let Some(system) = &conversation.system {
        body.insert(
            "systemInstruction".into(),
            json!({"parts": [{"text": system}]}),
        );
    }

    let mut generation_config = Map::new();
    insert_generation_scalar_fields(
        &mut generation_config,
        &request.generation,
        GenerationDialect::Google,
    );
    match &request.generation.response_format {
        ResponseFormat::Text => {
            generation_config.insert("responseMimeType".into(), json!("text/plain"));
        }
        ResponseFormat::JsonObject => {
            generation_config.insert("responseMimeType".into(), json!("application/json"));
        }
        ResponseFormat::JsonSchema { schema } => {
            generation_config.insert("responseMimeType".into(), json!("application/json"));
            generation_config.insert("responseJsonSchema".into(), schema.clone());
        }
    }
    if let Some(thinking) = thinking {
        let mut thinking_config = Map::new();
        if let Some(level) = thinking.level {
            thinking_config.insert("thinkingLevel".into(), json!(level.as_str()));
        }
        if let Some(budget) = thinking.budget_tokens {
            thinking_config.insert("thinkingBudget".into(), json!(budget));
        }
        thinking_config.insert(
            "includeThoughts".into(),
            Value::Bool(thinking.include_thoughts),
        );
        generation_config.insert("thinkingConfig".into(), Value::Object(thinking_config));
    }
    body.insert("generationConfig".into(), Value::Object(generation_config));

    if let Some(cached_content) = cached_content {
        body.insert("cachedContent".into(), json!(cached_content));
    }
    if !safety_settings.is_empty() {
        body.insert(
            "safetySettings".into(),
            Value::Array(
                safety_settings
                    .iter()
                    .map(|setting| {
                        let mut value = Map::new();
                        value.insert("category".into(), json!(setting.category.as_str()));
                        value.insert("threshold".into(), json!(setting.threshold.as_str()));
                        if vertex && let Some(method) = setting.method {
                            value.insert("method".into(), json!(method.as_str()));
                        }
                        Value::Object(value)
                    })
                    .collect(),
            ),
        );
    }
    if let Some(service_tier) = service_tier {
        body.insert("serviceTier".into(), json!(service_tier));
    }
    if !vertex {
        body.insert("store".into(), Value::Bool(false));
    }
    body
}

#[derive(Clone, Copy)]
enum GenerationDialect {
    OpenAi,
    Anthropic,
    DeepSeek,
    Ollama,
    Google,
}

fn insert_generation_scalar_fields(
    body: &mut Map<String, Value>,
    generation: &GenerationOptions,
    dialect: GenerationDialect,
) {
    let names = match dialect {
        GenerationDialect::OpenAi => GenerationFieldNames {
            temperature: "temperature",
            max_output_tokens: Some("max_output_tokens"),
            top_p: "top_p",
            top_k: None,
            presence_penalty: None,
            frequency_penalty: None,
            stop_sequences: None,
            seed: None,
        },
        GenerationDialect::Anthropic => GenerationFieldNames {
            temperature: "temperature",
            max_output_tokens: None,
            top_p: "top_p",
            top_k: Some("top_k"),
            presence_penalty: None,
            frequency_penalty: None,
            stop_sequences: Some("stop_sequences"),
            seed: None,
        },
        GenerationDialect::DeepSeek => GenerationFieldNames {
            temperature: "temperature",
            max_output_tokens: Some("max_tokens"),
            top_p: "top_p",
            top_k: None,
            presence_penalty: None,
            frequency_penalty: None,
            stop_sequences: Some("stop"),
            seed: None,
        },
        GenerationDialect::Ollama => GenerationFieldNames {
            temperature: "temperature",
            max_output_tokens: Some("num_predict"),
            top_p: "top_p",
            top_k: Some("top_k"),
            presence_penalty: None,
            frequency_penalty: None,
            stop_sequences: Some("stop"),
            seed: Some("seed"),
        },
        GenerationDialect::Google => GenerationFieldNames {
            temperature: "temperature",
            max_output_tokens: Some("maxOutputTokens"),
            top_p: "topP",
            top_k: Some("topK"),
            presence_penalty: Some("presencePenalty"),
            frequency_penalty: Some("frequencyPenalty"),
            stop_sequences: Some("stopSequences"),
            seed: Some("seed"),
        },
    };

    if let Some(value) = generation.temperature {
        body.insert(names.temperature.into(), json!(value));
    }
    if let (Some(name), Some(value)) = (names.max_output_tokens, generation.max_output_tokens) {
        body.insert(name.into(), json!(value));
    }
    if let Some(value) = generation.top_p {
        body.insert(names.top_p.into(), json!(value));
    }
    if let (Some(name), Some(value)) = (names.top_k, generation.top_k) {
        body.insert(name.into(), json!(value));
    }
    if let (Some(name), Some(value)) = (names.presence_penalty, generation.presence_penalty) {
        body.insert(name.into(), json!(value));
    }
    if let (Some(name), Some(value)) = (names.frequency_penalty, generation.frequency_penalty) {
        body.insert(name.into(), json!(value));
    }
    if let Some(name) = names.stop_sequences
        && !generation.stop_sequences.is_empty()
    {
        body.insert(name.into(), json!(generation.stop_sequences));
    }
    if let (Some(name), Some(value)) = (names.seed, generation.seed) {
        body.insert(name.into(), json!(value));
    }
}

struct GenerationFieldNames {
    temperature: &'static str,
    max_output_tokens: Option<&'static str>,
    top_p: &'static str,
    top_k: Option<&'static str>,
    presence_penalty: Option<&'static str>,
    frequency_penalty: Option<&'static str>,
    stop_sequences: Option<&'static str>,
    seed: Option<&'static str>,
}

fn base_compiled(
    provider: ProviderId,
    origin: &str,
    path_and_query: &str,
    auth_scheme: AuthScheme,
    stream_protocol: StreamProtocol,
    body: Map<String, Value>,
) -> CompiledProviderRequest {
    let mut static_headers = BTreeMap::new();
    static_headers.insert("content-type".into(), "application/json".into());
    CompiledProviderRequest {
        provider,
        origin: origin.to_owned(),
        path_and_query: path_and_query.to_owned(),
        auth_scheme,
        static_headers,
        stream_protocol,
        body: Value::Object(body),
        tokenizer_override: None,
    }
}

fn chat_messages(messages: &[ChatMessage], assistant_role: &str) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            json!({
                "role": match message.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => assistant_role,
                    MessageRole::System => unreachable!("system messages are extracted"),
                },
                "content": message.content
            })
        })
        .collect()
}

fn messages_with_system(conversation: &NormalizedConversation, assistant_role: &str) -> Vec<Value> {
    let mut messages = Vec::with_capacity(conversation.messages.len() + 1);
    if let Some(system) = &conversation.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.extend(chat_messages(&conversation.messages, assistant_role));
    messages
}

fn google_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            json!({
                "role": match message.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "model",
                    MessageRole::System => unreachable!("system messages are extracted"),
                },
                "parts": [{"text": message.content}]
            })
        })
        .collect()
}

fn normalize_conversation(messages: &[ChatMessage], policy: RolePolicy) -> NormalizedConversation {
    let system = messages
        .iter()
        .filter(|message| message.role == MessageRole::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut normalized = Vec::<ChatMessage>::new();
    for message in messages
        .iter()
        .filter(|message| message.role != MessageRole::System)
    {
        if policy == RolePolicy::MergeConsecutive
            && let Some(previous) = normalized.last_mut()
            && previous.role == message.role
        {
            previous.content.push_str("\n\n");
            previous.content.push_str(&message.content);
            continue;
        }
        normalized.push(message.clone());
    }
    NormalizedConversation {
        system: (!system.is_empty()).then_some(system),
        messages: normalized,
    }
}

fn merge_additional_parameters(
    body: &mut Value,
    parameters: &BTreeMap<String, Value>,
) -> Result<()> {
    let object = body
        .as_object_mut()
        .expect("provider compilers always create object bodies");
    for (key, value) in parameters {
        if object.contains_key(key) {
            return Err(ProviderConfigError::ExtraParameterCollision(key.clone()));
        }
        object.insert(key.clone(), value.clone());
    }
    Ok(())
}

fn require_capability(provider: ProviderId, capability: Capability) -> Result<()> {
    match capability_support(provider, capability) {
        CapabilitySupport::Native
        | CapabilitySupport::CompatibilityOnly
        | CapabilitySupport::LocalTransform
        | CapabilitySupport::EstimationOnly
        | CapabilitySupport::Restricted => Ok(()),
        CapabilitySupport::Automatic
        | CapabilitySupport::SeparateSecurityBoundary
        | CapabilitySupport::Forbidden
        | CapabilitySupport::Unsupported => Err(ProviderConfigError::UnsupportedOption {
            provider,
            capability,
        }),
    }
}

fn validate_text_field(field: &'static str, value: &str, max_bytes: usize) -> Result<()> {
    if value.trim().is_empty() {
        return Err(ProviderConfigError::EmptyField(field));
    }
    if value.len() > max_bytes {
        return Err(ProviderConfigError::FieldTooLong { field, max_bytes });
    }
    if value.chars().any(char::is_control) {
        return Err(ProviderConfigError::InvalidField {
            field,
            reason: "control characters are not allowed",
        });
    }
    Ok(())
}

fn validate_path_segment(field: &'static str, value: &str, lowercase_only: bool) -> Result<()> {
    validate_text_field(field, value, MAX_MODEL_ID_BYTES)?;
    let safe = value.chars().all(|character| {
        (if lowercase_only {
            character.is_ascii_lowercase() || character.is_ascii_digit()
        } else {
            character.is_ascii_alphanumeric()
        }) || matches!(character, '-' | '_' | '.')
    });
    if !safe || value == "." || value == ".." {
        return Err(ProviderConfigError::InvalidField {
            field,
            reason: "must be a single safe URL path segment",
        });
    }
    Ok(())
}

fn validate_cache_resource(field: &'static str, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        validate_text_field(field, value, MAX_CACHE_RESOURCE_BYTES)?;
        if value.starts_with('/')
            || value.contains("//")
            || value.contains('?')
            || value.contains('#')
            || value.split('/').any(|segment| segment == "..")
        {
            return Err(ProviderConfigError::InvalidField {
                field,
                reason: "cache resource must be a relative provider resource name",
            });
        }
    }
    Ok(())
}

fn validate_float_range(field: &'static str, value: f64, min: f64, max: f64) -> Result<()> {
    if !value.is_finite() {
        return Err(invalid_number(field));
    }
    if !(min..=max).contains(&value) {
        return Err(ProviderConfigError::OutOfRange { field, min, max });
    }
    Ok(())
}

fn invalid_number(field: &'static str) -> ProviderConfigError {
    ProviderConfigError::InvalidField {
        field,
        reason: "must be a finite number",
    }
}

fn value_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(value_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(value_depth).max().unwrap_or(0),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::{AuthScheme, StreamProtocol, compile_request};
    use crate::{
        AnthropicOptions, AnthropicThinking, AnthropicThinkingDisplay, Capability, ChatMessage,
        DeepSeekOptions, GenerationOptions, GoogleOptions, GoogleSafetyCategory,
        GoogleSafetyMethod, GoogleSafetySetting, GoogleSafetyThreshold, GoogleServiceTier,
        GoogleThinking, GoogleThinkingLevel, MessageRole, OllamaCloudOptions, OllamaThinking,
        OpenAiOptions, OpenAiPromptCache, OpenAiPromptCacheRetention, OpenAiReasoning,
        OpenAiReasoningEffort, OpenAiReasoningSummary, OpenAiServiceTier, ProviderConfigError,
        ProviderId, ProviderOptions, ProviderRequest, ResponseFormat, RolePolicy,
        TokenizerOverride, VertexAiOptions, VertexRequestType,
    };

    fn messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage::new(MessageRole::System, "Stay in character."),
            ChatMessage::new(MessageRole::User, "Hello"),
            ChatMessage::new(MessageRole::Assistant, "Hi"),
        ]
    }

    fn request(provider: ProviderId, options: ProviderOptions) -> ProviderRequest {
        ProviderRequest {
            provider,
            model_id: "test-model".into(),
            messages: messages(),
            generation: GenerationOptions {
                max_output_tokens: Some(2048),
                ..GenerationOptions::default()
            },
            provider_options: options,
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
        }
    }

    #[test]
    fn openai_responses_request_has_fixed_transport_and_wire_shape() {
        let mut request = request(
            ProviderId::OpenAi,
            ProviderOptions::OpenAi(OpenAiOptions {
                reasoning: Some(OpenAiReasoning {
                    effort: OpenAiReasoningEffort::High,
                    summary: Some(OpenAiReasoningSummary::Concise),
                }),
                prompt_cache: Some(OpenAiPromptCache {
                    key: "character-42".into(),
                    retention: OpenAiPromptCacheRetention::TwentyFourHours,
                }),
                service_tier: Some(OpenAiServiceTier::Priority),
            }),
        );
        request.generation.temperature = Some(0.7);
        request.generation.top_p = Some(0.9);
        request.generation.response_format = ResponseFormat::JsonObject;

        let compiled = compile_request(&request).expect("OpenAI request must compile");
        assert_eq!(compiled.origin, "https://api.openai.com");
        assert_eq!(compiled.path_and_query, "/v1/responses");
        assert_eq!(compiled.auth_scheme, AuthScheme::AuthorizationBearer);
        assert_eq!(compiled.stream_protocol, StreamProtocol::Sse);
        assert_eq!(compiled.body["store"], false);
        assert_eq!(compiled.body["instructions"], "Stay in character.");
        assert_eq!(compiled.body["reasoning"]["effort"], "high");
        assert_eq!(compiled.body["text"]["format"]["type"], "json_object");
        assert_eq!(compiled.body["prompt_cache_retention"], "24h");
        assert_eq!(compiled.body["service_tier"], "priority");
    }

    #[test]
    fn anthropic_compiles_thinking_schema_cache_and_version_header() {
        let mut request = request(
            ProviderId::Anthropic,
            ProviderOptions::Anthropic(AnthropicOptions {
                thinking: Some(AnthropicThinking::Enabled {
                    budget_tokens: 1024,
                    display: AnthropicThinkingDisplay::Summarized,
                }),
                reasoning_effort: Some(crate::AnthropicReasoningEffort::High),
                cache_ttl: Some(crate::AnthropicCacheTtl::OneHour),
                service_tier: Some(crate::AnthropicServiceTier::StandardOnly),
            }),
        );
        request.generation.max_output_tokens = Some(4096);
        request.generation.response_format = ResponseFormat::JsonSchema {
            schema: json!({"type": "object", "properties": {"answer": {"type": "string"}}}),
        };

        let compiled = compile_request(&request).expect("Anthropic request must compile");
        assert_eq!(compiled.origin, "https://api.anthropic.com");
        assert_eq!(compiled.path_and_query, "/v1/messages");
        assert_eq!(compiled.auth_scheme, AuthScheme::AnthropicXApiKey);
        assert_eq!(compiled.static_headers["anthropic-version"], "2023-06-01");
        assert_eq!(compiled.body["thinking"]["budget_tokens"], 1024);
        assert_eq!(
            compiled.body["output_config"]["format"]["type"],
            "json_schema"
        );
        assert_eq!(compiled.body["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn deepseek_compiles_json_object_and_thinking_contract() {
        let mut request = request(
            ProviderId::DeepSeek,
            ProviderOptions::DeepSeek(DeepSeekOptions {
                thinking_enabled: Some(true),
                reasoning_effort: Some(crate::DeepSeekReasoningEffort::Max),
            }),
        );
        request.generation.response_format = ResponseFormat::JsonObject;
        request.generation.stop_sequences = vec!["END".into()];

        let compiled = compile_request(&request).expect("DeepSeek request must compile");
        assert_eq!(compiled.origin, "https://api.deepseek.com");
        assert_eq!(compiled.path_and_query, "/chat/completions");
        assert_eq!(compiled.body["thinking"]["type"], "enabled");
        assert_eq!(compiled.body["reasoning_effort"], "max");
        assert_eq!(compiled.body["response_format"]["type"], "json_object");
        assert_eq!(compiled.body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn ollama_cloud_uses_native_cloud_api_and_ndjson() {
        let mut request = request(
            ProviderId::OllamaCloud,
            ProviderOptions::OllamaCloud(OllamaCloudOptions {
                thinking: Some(OllamaThinking::Medium),
            }),
        );
        request.model_id = "qwen3:latest".into();
        request.generation.temperature = Some(0.4);
        request.generation.top_k = Some(40);
        request.generation.seed = Some(7);

        let compiled = compile_request(&request).expect("Ollama Cloud request must compile");
        assert_eq!(compiled.origin, "https://ollama.com");
        assert_eq!(compiled.path_and_query, "/api/chat");
        assert_eq!(compiled.stream_protocol, StreamProtocol::Ndjson);
        assert_eq!(compiled.body["options"]["temperature"], 0.4);
        assert_eq!(compiled.body["options"]["top_k"], 40);
        assert_eq!(compiled.body["think"], "medium");
    }

    #[test]
    fn gemini_compiles_lower_camel_generation_and_safety_fields() {
        let mut request = request(
            ProviderId::GoogleGemini,
            ProviderOptions::GoogleGemini(GoogleOptions {
                thinking: Some(GoogleThinking {
                    level: Some(GoogleThinkingLevel::High),
                    budget_tokens: None,
                    include_thoughts: true,
                }),
                cached_content: Some("cachedContents/example".into()),
                safety_settings: vec![GoogleSafetySetting {
                    category: GoogleSafetyCategory::Harassment,
                    threshold: GoogleSafetyThreshold::BlockOnlyHigh,
                    method: None,
                }],
                service_tier: Some(GoogleServiceTier::Flex),
            }),
        );
        request.model_id = "gemini-test".into();
        request.generation.top_k = Some(32);
        request.generation.presence_penalty = Some(0.2);
        request.generation.response_format = ResponseFormat::JsonObject;

        let compiled = compile_request(&request).expect("Gemini request must compile");
        assert_eq!(compiled.origin, "https://generativelanguage.googleapis.com");
        assert_eq!(
            compiled.path_and_query,
            "/v1beta/models/gemini-test:streamGenerateContent?alt=sse"
        );
        assert_eq!(compiled.auth_scheme, AuthScheme::GoogleXGoogApiKey);
        assert_eq!(compiled.body["contents"][1]["role"], "model");
        assert_eq!(compiled.body["generationConfig"]["topK"], 32);
        assert_eq!(
            compiled.body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "HIGH"
        );
        assert_eq!(compiled.body["serviceTier"], "flex");
        assert_eq!(
            compiled.body["safetySettings"][0]["category"],
            "HARM_CATEGORY_HARASSMENT"
        );
    }

    #[test]
    fn vertex_compiles_scoped_url_oauth_and_request_type_header() {
        let mut request = request(
            ProviderId::GoogleVertexAi,
            ProviderOptions::GoogleVertexAi(VertexAiOptions {
                project_id: "lorepia-prod".into(),
                location: "us-central1".into(),
                thinking: None,
                cached_content: Some(
                    "projects/lorepia-prod/locations/us-central1/cachedContents/cache-1".into(),
                ),
                safety_settings: vec![GoogleSafetySetting {
                    category: GoogleSafetyCategory::DangerousContent,
                    threshold: GoogleSafetyThreshold::BlockMediumAndAbove,
                    method: Some(GoogleSafetyMethod::Severity),
                }],
                request_type: VertexRequestType::Dedicated,
            }),
        );
        request.model_id = "gemini-test".into();

        let compiled = compile_request(&request).expect("Vertex request must compile");
        assert_eq!(
            compiled.origin,
            "https://us-central1-aiplatform.googleapis.com"
        );
        assert_eq!(
            compiled.path_and_query,
            "/v1/projects/lorepia-prod/locations/us-central1/publishers/google/models/gemini-test:streamGenerateContent?alt=sse"
        );
        assert_eq!(compiled.auth_scheme, AuthScheme::GoogleOAuthBearer);
        assert_eq!(
            compiled.static_headers["X-Vertex-AI-LLM-Request-Type"],
            "dedicated"
        );
        assert_eq!(compiled.body["safetySettings"][0]["method"], "SEVERITY");
        assert!(compiled.body.get("serviceTier").is_none());
    }

    #[test]
    fn role_policy_merges_consecutive_non_system_messages() {
        let mut request = request(
            ProviderId::OpenAi,
            ProviderOptions::OpenAi(OpenAiOptions::default()),
        );
        request.messages = vec![
            ChatMessage::new(MessageRole::User, "one"),
            ChatMessage::new(MessageRole::User, "two"),
            ChatMessage::new(MessageRole::Assistant, "three"),
        ];
        request.generation.role_policy = RolePolicy::MergeConsecutive;

        let compiled = compile_request(&request).expect("merged request must compile");
        assert_eq!(compiled.body["input"].as_array().unwrap().len(), 2);
        assert_eq!(compiled.body["input"][0]["content"], "one\n\ntwo");
    }

    #[test]
    fn google_requires_alternating_roles_unless_merge_is_selected() {
        let mut request = request(
            ProviderId::GoogleGemini,
            ProviderOptions::GoogleGemini(GoogleOptions::default()),
        );
        request.model_id = "gemini-test".into();
        request.messages = vec![
            ChatMessage::new(MessageRole::User, "one"),
            ChatMessage::new(MessageRole::User, "two"),
        ];

        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::InvalidField {
                field: "messages.role",
                ..
            })
        ));

        request.generation.role_policy = RolePolicy::MergeConsecutive;
        let compiled = compile_request(&request).expect("Google roles must merge");
        assert_eq!(compiled.body["contents"].as_array().unwrap().len(), 1);
        assert_eq!(
            compiled.body["contents"][0]["parts"][0]["text"],
            "one\n\ntwo"
        );
    }

    #[test]
    fn system_messages_cannot_be_silently_reordered_from_mid_conversation() {
        let mut request = request(
            ProviderId::OpenAi,
            ProviderOptions::OpenAi(OpenAiOptions::default()),
        );
        request.messages = vec![
            ChatMessage::new(MessageRole::User, "one"),
            ChatMessage::new(MessageRole::System, "late instruction"),
            ChatMessage::new(MessageRole::Assistant, "two"),
        ];

        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::InvalidField {
                field: "messages.role",
                ..
            })
        ));
    }

    #[test]
    fn tokenizer_override_never_enters_provider_body() {
        let mut request = request(
            ProviderId::OpenAi,
            ProviderOptions::OpenAi(OpenAiOptions::default()),
        );
        request.tokenizer_override = Some(TokenizerOverride {
            tokenizer_id: "o200k_base".into(),
        });

        let compiled = compile_request(&request).expect("override metadata must compile");
        assert_eq!(compiled.tokenizer_override, request.tokenizer_override);
        let body = serde_json::to_string(&compiled.body).unwrap();
        assert!(!body.contains("tokenizer"));
        assert!(!body.contains("o200k_base"));
    }

    #[test]
    fn restricted_additional_parameter_is_merged_but_sensitive_keys_are_rejected() {
        let mut request = request(
            ProviderId::OpenAi,
            ProviderOptions::OpenAi(OpenAiOptions::default()),
        );
        request
            .additional_parameters
            .insert("metadata".into(), json!({"surface": "chat"}));
        let compiled = compile_request(&request).expect("safe metadata must compile");
        assert_eq!(compiled.body["metadata"]["surface"], "chat");

        for key in ["tools", "endpoint_url", "authorization", "api_key"] {
            let mut rejected = request.clone();
            rejected.additional_parameters.clear();
            rejected
                .additional_parameters
                .insert(key.into(), json!("bad"));
            assert!(matches!(
                compile_request(&rejected),
                Err(ProviderConfigError::UnsafeExtraParameter(_))
            ));
        }

        let mut nested = request.clone();
        nested.additional_parameters.clear();
        nested.additional_parameters.insert(
            "metadata".into(),
            json!({"connection": {"mcp_server": "https://example.invalid"}}),
        );
        assert!(matches!(
            compile_request(&nested),
            Err(ProviderConfigError::UnsafeExtraParameter(_))
        ));
    }

    #[test]
    fn provider_option_mismatch_is_rejected() {
        let request = request(
            ProviderId::OpenAi,
            ProviderOptions::DeepSeek(DeepSeekOptions::default()),
        );
        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::ProviderOptionsMismatch { .. })
        ));
    }

    #[test]
    fn unsupported_options_fail_instead_of_being_silently_dropped() {
        let mut openai = request(
            ProviderId::OpenAi,
            ProviderOptions::OpenAi(OpenAiOptions::default()),
        );
        openai.generation.stop_sequences.push("STOP".into());
        assert_eq!(
            compile_request(&openai),
            Err(ProviderConfigError::UnsupportedOption {
                provider: ProviderId::OpenAi,
                capability: Capability::StopSequences,
            })
        );

        let mut ollama = request(
            ProviderId::OllamaCloud,
            ProviderOptions::OllamaCloud(OllamaCloudOptions::default()),
        );
        ollama.generation.response_format = ResponseFormat::JsonObject;
        assert_eq!(
            compile_request(&ollama),
            Err(ProviderConfigError::UnsupportedOption {
                provider: ProviderId::OllamaCloud,
                capability: Capability::JsonResponse,
            })
        );
    }

    #[test]
    fn deepseek_rejects_controls_that_thinking_mode_would_ignore() {
        let mut request = request(
            ProviderId::DeepSeek,
            ProviderOptions::DeepSeek(DeepSeekOptions {
                thinking_enabled: Some(true),
                reasoning_effort: None,
            }),
        );
        request.generation.temperature = Some(0.5);
        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::InvalidField {
                field: "thinkingEnabled",
                ..
            })
        ));
    }

    #[test]
    fn deepseek_default_thinking_also_rejects_ignored_sampling_controls() {
        let mut request = request(
            ProviderId::DeepSeek,
            ProviderOptions::DeepSeek(DeepSeekOptions::default()),
        );
        request.generation.top_p = Some(0.9);
        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::InvalidField {
                field: "thinkingEnabled",
                ..
            })
        ));
    }

    #[test]
    fn anthropic_requires_output_budget_and_schema_for_json() {
        let mut request = request(
            ProviderId::Anthropic,
            ProviderOptions::Anthropic(AnthropicOptions::default()),
        );
        request.generation.max_output_tokens = None;
        assert_eq!(
            compile_request(&request),
            Err(ProviderConfigError::MissingField(
                "generation.maxOutputTokens"
            ))
        );

        request.generation.max_output_tokens = Some(2048);
        request.generation.response_format = ResponseFormat::JsonObject;
        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::InvalidField {
                field: "responseFormat",
                ..
            })
        ));
    }

    #[test]
    fn anthropic_allows_zero_output_for_cache_warming() {
        let mut request = request(
            ProviderId::Anthropic,
            ProviderOptions::Anthropic(AnthropicOptions {
                cache_ttl: Some(crate::AnthropicCacheTtl::FiveMinutes),
                ..AnthropicOptions::default()
            }),
        );
        request.generation.max_output_tokens = Some(0);

        let compiled = compile_request(&request).expect("cache warm request must compile");
        assert_eq!(compiled.body["max_tokens"], 0);
        assert_eq!(compiled.body["cache_control"]["ttl"], "5m");
    }

    #[test]
    fn google_rejects_ambiguous_thinking_and_duplicate_safety_categories() {
        let options = GoogleOptions {
            thinking: Some(GoogleThinking {
                level: Some(GoogleThinkingLevel::Low),
                budget_tokens: Some(1024),
                include_thoughts: false,
            }),
            cached_content: None,
            safety_settings: vec![],
            service_tier: None,
        };
        let ambiguous = request(
            ProviderId::GoogleGemini,
            ProviderOptions::GoogleGemini(options),
        );
        assert!(matches!(
            compile_request(&ambiguous),
            Err(ProviderConfigError::InvalidField {
                field: "thinking",
                ..
            })
        ));

        let duplicate = GoogleSafetySetting {
            category: GoogleSafetyCategory::HateSpeech,
            threshold: GoogleSafetyThreshold::BlockNone,
            method: None,
        };
        let request = request(
            ProviderId::GoogleGemini,
            ProviderOptions::GoogleGemini(GoogleOptions {
                safety_settings: vec![duplicate, duplicate],
                ..GoogleOptions::default()
            }),
        );
        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::InvalidField {
                field: "safetySettings",
                ..
            })
        ));
    }

    #[test]
    fn gemini_preserves_disable_and_dynamic_thinking_budgets() {
        for budget in [0, -1] {
            let mut request = request(
                ProviderId::GoogleGemini,
                ProviderOptions::GoogleGemini(GoogleOptions {
                    thinking: Some(GoogleThinking {
                        level: None,
                        budget_tokens: Some(budget),
                        include_thoughts: false,
                    }),
                    ..GoogleOptions::default()
                }),
            );
            request.model_id = "gemini-test".into();
            let compiled = compile_request(&request).expect("thinking budget must compile");
            assert_eq!(
                compiled.body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
                budget
            );
        }
    }

    #[test]
    fn cache_resources_require_exact_provider_scopes() {
        let mut gemini = request(
            ProviderId::GoogleGemini,
            ProviderOptions::GoogleGemini(GoogleOptions {
                cached_content: Some("cachedContents/id/extra".into()),
                ..GoogleOptions::default()
            }),
        );
        gemini.model_id = "gemini-test".into();
        assert!(matches!(
            compile_request(&gemini),
            Err(ProviderConfigError::InvalidField {
                field: "cachedContent",
                ..
            })
        ));

        let mut vertex = request(
            ProviderId::GoogleVertexAi,
            ProviderOptions::GoogleVertexAi(VertexAiOptions {
                project_id: "lorepia-prod".into(),
                location: "global".into(),
                thinking: None,
                cached_content: Some(
                    "projects/lorepia-prod/locations/global/cachedContents/".into(),
                ),
                safety_settings: vec![],
                request_type: VertexRequestType::Automatic,
            }),
        );
        vertex.model_id = "gemini-test".into();
        assert!(matches!(
            compile_request(&vertex),
            Err(ProviderConfigError::InvalidField { .. })
        ));
    }

    #[test]
    fn jailbreak_safety_category_is_vertex_only() {
        let setting = GoogleSafetySetting {
            category: GoogleSafetyCategory::Jailbreak,
            threshold: GoogleSafetyThreshold::BlockOnlyHigh,
            method: None,
        };
        let mut gemini = request(
            ProviderId::GoogleGemini,
            ProviderOptions::GoogleGemini(GoogleOptions {
                safety_settings: vec![setting],
                ..GoogleOptions::default()
            }),
        );
        gemini.model_id = "gemini-test".into();
        assert!(matches!(
            compile_request(&gemini),
            Err(ProviderConfigError::InvalidField {
                field: "safetySettings.category",
                ..
            })
        ));

        let mut vertex = request(
            ProviderId::GoogleVertexAi,
            ProviderOptions::GoogleVertexAi(VertexAiOptions {
                project_id: "lorepia-prod".into(),
                location: "global".into(),
                thinking: None,
                cached_content: None,
                safety_settings: vec![setting],
                request_type: VertexRequestType::Automatic,
            }),
        );
        vertex.model_id = "gemini-test".into();
        let compiled = compile_request(&vertex).expect("Vertex jailbreak category must compile");
        assert_eq!(
            compiled.body["safetySettings"][0]["category"],
            "HARM_CATEGORY_JAILBREAK"
        );
    }

    #[test]
    fn vertex_url_segments_and_cache_scope_cannot_escape() {
        let request = request(
            ProviderId::GoogleVertexAi,
            ProviderOptions::GoogleVertexAi(VertexAiOptions {
                project_id: "../other".into(),
                location: "global".into(),
                thinking: None,
                cached_content: None,
                safety_settings: vec![],
                request_type: VertexRequestType::Automatic,
            }),
        );
        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::InvalidField {
                field: "projectId",
                ..
            })
        ));
    }

    #[test]
    fn compiled_transport_metadata_contains_no_secret_value() {
        for (provider, options) in [
            (
                ProviderId::OpenAi,
                ProviderOptions::OpenAi(OpenAiOptions::default()),
            ),
            (
                ProviderId::Anthropic,
                ProviderOptions::Anthropic(AnthropicOptions::default()),
            ),
            (
                ProviderId::DeepSeek,
                ProviderOptions::DeepSeek(DeepSeekOptions::default()),
            ),
            (
                ProviderId::OllamaCloud,
                ProviderOptions::OllamaCloud(OllamaCloudOptions::default()),
            ),
            (
                ProviderId::GoogleGemini,
                ProviderOptions::GoogleGemini(GoogleOptions::default()),
            ),
        ] {
            let compiled = compile_request(&request(provider, options)).unwrap();
            let representation = format!("{compiled:?}").to_ascii_lowercase();
            assert!(!representation.contains("bearer sk-"));
            assert!(!representation.contains("api_key\":"));
            assert!(!representation.contains("authorization\":"));
        }
    }

    #[test]
    fn nested_additional_parameters_have_a_hard_depth_limit() {
        let mut request = request(
            ProviderId::OpenAi,
            ProviderOptions::OpenAi(OpenAiOptions::default()),
        );
        request.additional_parameters.insert(
            "metadata".into(),
            json!({"one": {"two": {"three": {"four": {"five": true}}}}}),
        );
        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::ExtraParameterTooDeep { .. })
        ));
    }

    #[test]
    fn response_schema_must_be_a_bounded_object() {
        let mut request = request(
            ProviderId::OpenAi,
            ProviderOptions::OpenAi(OpenAiOptions::default()),
        );
        request.generation.response_format = ResponseFormat::JsonSchema {
            schema: Value::String("not a schema".into()),
        };
        assert!(matches!(
            compile_request(&request),
            Err(ProviderConfigError::InvalidField {
                field: "responseFormat.schema",
                ..
            })
        ));
    }
}
