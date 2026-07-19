use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum ProviderId {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "deepseek")]
    DeepSeek,
    #[serde(rename = "ollama-cloud")]
    OllamaCloud,
    #[serde(rename = "google-gemini")]
    GoogleGemini,
    #[serde(rename = "google-vertex-ai")]
    GoogleVertexAi,
}

impl ProviderId {
    pub const ALL: [Self; 6] = [
        Self::OpenAi,
        Self::Anthropic,
        Self::DeepSeek,
        Self::OllamaCloud,
        Self::GoogleGemini,
        Self::GoogleVertexAi,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::DeepSeek => "deepseek",
            Self::OllamaCloud => "ollama-cloud",
            Self::GoogleGemini => "google-gemini",
            Self::GoogleVertexAi => "google-vertex-ai",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

impl ChatMessage {
    #[must_use]
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolePolicy {
    #[default]
    Preserve,
    MergeConsecutive,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    #[default]
    Text,
    JsonObject,
    JsonSchema {
        schema: Value,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationOptions {
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<u32>,
    pub top_p: Option<f64>,
    pub top_k: Option<u32>,
    pub presence_penalty: Option<f64>,
    pub frequency_penalty: Option<f64>,
    #[serde(default)]
    pub stop_sequences: Vec<String>,
    pub seed: Option<i64>,
    #[serde(default)]
    pub response_format: ResponseFormat,
    #[serde(default)]
    pub role_policy: RolePolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl OpenAiReasoningEffort {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiReasoningSummary {
    Auto,
    Concise,
    Detailed,
}

impl OpenAiReasoningSummary {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Concise => "concise",
            Self::Detailed => "detailed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiServiceTier {
    Auto,
    Default,
    Flex,
    Scale,
    Priority,
}

impl OpenAiServiceTier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Default => "default",
            Self::Flex => "flex",
            Self::Scale => "scale",
            Self::Priority => "priority",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenAiPromptCacheRetention {
    InMemory,
    #[serde(rename = "24h")]
    TwentyFourHours,
}

impl OpenAiPromptCacheRetention {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InMemory => "in-memory",
            Self::TwentyFourHours => "24h",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiPromptCache {
    pub key: String,
    pub retention: OpenAiPromptCacheRetention,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiReasoning {
    pub effort: OpenAiReasoningEffort,
    pub summary: Option<OpenAiReasoningSummary>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiOptions {
    pub reasoning: Option<OpenAiReasoning>,
    pub prompt_cache: Option<OpenAiPromptCache>,
    pub service_tier: Option<OpenAiServiceTier>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnthropicThinkingDisplay {
    Summarized,
    Omitted,
}

impl AnthropicThinkingDisplay {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Summarized => "summarized",
            Self::Omitted => "omitted",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AnthropicThinking {
    Disabled,
    Adaptive {
        display: AnthropicThinkingDisplay,
    },
    Enabled {
        budget_tokens: u32,
        display: AnthropicThinkingDisplay,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnthropicReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl AnthropicReasoningEffort {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AnthropicCacheTtl {
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
}

impl AnthropicCacheTtl {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FiveMinutes => "5m",
            Self::OneHour => "1h",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnthropicServiceTier {
    Auto,
    StandardOnly,
}

impl AnthropicServiceTier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::StandardOnly => "standard_only",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnthropicOptions {
    pub thinking: Option<AnthropicThinking>,
    pub reasoning_effort: Option<AnthropicReasoningEffort>,
    pub cache_ttl: Option<AnthropicCacheTtl>,
    pub service_tier: Option<AnthropicServiceTier>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeepSeekReasoningEffort {
    High,
    Max,
}

impl DeepSeekReasoningEffort {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekOptions {
    pub thinking_enabled: Option<bool>,
    pub reasoning_effort: Option<DeepSeekReasoningEffort>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OllamaThinking {
    Disabled,
    Enabled,
    Low,
    Medium,
    High,
    Max,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaCloudOptions {
    pub thinking: Option<OllamaThinking>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoogleThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
}

impl GoogleThinkingLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "MINIMAL",
            Self::Low => "LOW",
            Self::Medium => "MEDIUM",
            Self::High => "HIGH",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleThinking {
    pub level: Option<GoogleThinkingLevel>,
    /// `-1` requests a dynamic budget and `0` disables thinking on models
    /// which support those Gemini 2.5 controls. Other values are model-bound.
    pub budget_tokens: Option<i32>,
    pub include_thoughts: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoogleSafetyCategory {
    HateSpeech,
    DangerousContent,
    Harassment,
    SexuallyExplicit,
    CivicIntegrity,
    Jailbreak,
}

impl GoogleSafetyCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HateSpeech => "HARM_CATEGORY_HATE_SPEECH",
            Self::DangerousContent => "HARM_CATEGORY_DANGEROUS_CONTENT",
            Self::Harassment => "HARM_CATEGORY_HARASSMENT",
            Self::SexuallyExplicit => "HARM_CATEGORY_SEXUALLY_EXPLICIT",
            Self::CivicIntegrity => "HARM_CATEGORY_CIVIC_INTEGRITY",
            Self::Jailbreak => "HARM_CATEGORY_JAILBREAK",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoogleSafetyThreshold {
    BlockLowAndAbove,
    BlockMediumAndAbove,
    BlockOnlyHigh,
    BlockNone,
    Off,
}

impl GoogleSafetyThreshold {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BlockLowAndAbove => "BLOCK_LOW_AND_ABOVE",
            Self::BlockMediumAndAbove => "BLOCK_MEDIUM_AND_ABOVE",
            Self::BlockOnlyHigh => "BLOCK_ONLY_HIGH",
            Self::BlockNone => "BLOCK_NONE",
            Self::Off => "OFF",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoogleSafetyMethod {
    Severity,
    Probability,
}

impl GoogleSafetyMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Severity => "SEVERITY",
            Self::Probability => "PROBABILITY",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleSafetySetting {
    pub category: GoogleSafetyCategory,
    pub threshold: GoogleSafetyThreshold,
    pub method: Option<GoogleSafetyMethod>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoogleServiceTier {
    Standard,
    Flex,
    Priority,
}

impl GoogleServiceTier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Flex => "flex",
            Self::Priority => "priority",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleOptions {
    pub thinking: Option<GoogleThinking>,
    pub cached_content: Option<String>,
    #[serde(default)]
    pub safety_settings: Vec<GoogleSafetySetting>,
    pub service_tier: Option<GoogleServiceTier>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VertexRequestType {
    #[default]
    Automatic,
    Shared,
    Dedicated,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VertexAiOptions {
    pub project_id: String,
    pub location: String,
    pub thinking: Option<GoogleThinking>,
    pub cached_content: Option<String>,
    #[serde(default)]
    pub safety_settings: Vec<GoogleSafetySetting>,
    #[serde(default)]
    pub request_type: VertexRequestType,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "provider", content = "config")]
pub enum ProviderOptions {
    #[serde(rename = "openai")]
    OpenAi(OpenAiOptions),
    #[serde(rename = "anthropic")]
    Anthropic(AnthropicOptions),
    #[serde(rename = "deepseek")]
    DeepSeek(DeepSeekOptions),
    #[serde(rename = "ollama-cloud")]
    OllamaCloud(OllamaCloudOptions),
    #[serde(rename = "google-gemini")]
    GoogleGemini(GoogleOptions),
    #[serde(rename = "google-vertex-ai")]
    GoogleVertexAi(VertexAiOptions),
}

impl ProviderOptions {
    #[must_use]
    pub const fn provider_id(&self) -> ProviderId {
        match self {
            Self::OpenAi(_) => ProviderId::OpenAi,
            Self::Anthropic(_) => ProviderId::Anthropic,
            Self::DeepSeek(_) => ProviderId::DeepSeek,
            Self::OllamaCloud(_) => ProviderId::OllamaCloud,
            Self::GoogleGemini(_) => ProviderId::GoogleGemini,
            Self::GoogleVertexAi(_) => ProviderId::GoogleVertexAi,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenizerOverride {
    pub tokenizer_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRequest {
    pub provider: ProviderId,
    pub model_id: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub generation: GenerationOptions,
    pub provider_options: ProviderOptions,
    pub tokenizer_override: Option<TokenizerOverride>,
    #[serde(default)]
    pub additional_parameters: BTreeMap<String, Value>,
}
