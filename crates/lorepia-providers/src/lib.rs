#![forbid(unsafe_code)]

mod capabilities;
mod compile;
mod config;
mod error;

pub use capabilities::{Capability, CapabilitySupport, ProviderCapabilities, capability_support};
pub use compile::{AuthScheme, CompiledProviderRequest, StreamProtocol, compile_request};
pub use config::{
    AnthropicCacheTtl, AnthropicOptions, AnthropicReasoningEffort, AnthropicServiceTier,
    AnthropicThinking, AnthropicThinkingDisplay, ChatMessage, DeepSeekOptions,
    DeepSeekReasoningEffort, GenerationOptions, GoogleOptions, GoogleSafetyCategory,
    GoogleSafetyMethod, GoogleSafetySetting, GoogleSafetyThreshold, GoogleServiceTier,
    GoogleThinking, GoogleThinkingLevel, MessageRole, OllamaCloudOptions, OllamaThinking,
    OpenAiOptions, OpenAiPromptCache, OpenAiPromptCacheRetention, OpenAiReasoning,
    OpenAiReasoningEffort, OpenAiReasoningSummary, OpenAiServiceTier, ProviderId, ProviderOptions,
    ProviderRequest, ResponseFormat, RolePolicy, TokenizerOverride, VertexAiOptions,
    VertexRequestType,
};
pub use error::{ProviderConfigError, Result};
