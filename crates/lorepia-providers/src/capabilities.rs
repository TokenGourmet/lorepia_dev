use serde::{Deserialize, Serialize};

use crate::config::ProviderId;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Temperature,
    MaxOutputTokens,
    TopP,
    TopK,
    PresencePenalty,
    FrequencyPenalty,
    StopSequences,
    Seed,
    Reasoning,
    IncludeThoughts,
    JsonResponse,
    ContextCaching,
    SafetySettings,
    ServiceTier,
    ThroughputRequestType,
    ProjectAndLocation,
    RoleAlternation,
    TokenizerOverride,
    AdditionalParameters,
    McpTools,
    EndpointOverride,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CapabilitySupport {
    Native,
    CompatibilityOnly,
    Automatic,
    LocalTransform,
    EstimationOnly,
    Restricted,
    SeparateSecurityBoundary,
    Forbidden,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderCapabilities {
    provider: ProviderId,
}

impl ProviderCapabilities {
    #[must_use]
    pub const fn new(provider: ProviderId) -> Self {
        Self { provider }
    }

    #[must_use]
    pub const fn provider(self) -> ProviderId {
        self.provider
    }

    #[must_use]
    pub const fn support(self, capability: Capability) -> CapabilitySupport {
        capability_support(self.provider, capability)
    }
}

#[must_use]
pub const fn capability_support(provider: ProviderId, capability: Capability) -> CapabilitySupport {
    use Capability as C;
    use CapabilitySupport as S;
    use ProviderId as P;

    match capability {
        C::Temperature | C::TopP => match provider {
            P::Anthropic => S::CompatibilityOnly,
            P::OpenAi | P::DeepSeek | P::OllamaCloud | P::GoogleGemini | P::GoogleVertexAi => {
                S::Native
            }
        },
        C::MaxOutputTokens => S::Native,
        C::TopK => match provider {
            P::Anthropic => S::CompatibilityOnly,
            P::OllamaCloud | P::GoogleGemini | P::GoogleVertexAi => S::Native,
            P::OpenAi | P::DeepSeek => S::Unsupported,
        },
        C::PresencePenalty | C::FrequencyPenalty => match provider {
            P::GoogleGemini | P::GoogleVertexAi => S::Native,
            P::OpenAi | P::Anthropic | P::DeepSeek | P::OllamaCloud => S::Unsupported,
        },
        C::StopSequences => match provider {
            P::Anthropic | P::DeepSeek | P::OllamaCloud | P::GoogleGemini | P::GoogleVertexAi => {
                S::Native
            }
            P::OpenAi => S::Unsupported,
        },
        C::Seed => match provider {
            P::OllamaCloud | P::GoogleGemini | P::GoogleVertexAi => S::Native,
            P::OpenAi | P::Anthropic | P::DeepSeek => S::Unsupported,
        },
        C::Reasoning => S::Native,
        C::IncludeThoughts => match provider {
            P::Anthropic | P::GoogleGemini | P::GoogleVertexAi => S::Native,
            P::DeepSeek | P::OllamaCloud => S::Automatic,
            P::OpenAi => S::Unsupported,
        },
        C::JsonResponse => match provider {
            P::OpenAi | P::Anthropic | P::DeepSeek | P::GoogleGemini | P::GoogleVertexAi => {
                S::Native
            }
            P::OllamaCloud => S::Unsupported,
        },
        C::ContextCaching => match provider {
            P::OpenAi | P::Anthropic | P::GoogleGemini | P::GoogleVertexAi => S::Native,
            P::DeepSeek => S::Automatic,
            P::OllamaCloud => S::Unsupported,
        },
        C::SafetySettings => match provider {
            P::GoogleGemini | P::GoogleVertexAi => S::Native,
            P::OpenAi | P::Anthropic | P::DeepSeek | P::OllamaCloud => S::Unsupported,
        },
        C::ServiceTier => match provider {
            P::OpenAi | P::Anthropic | P::GoogleGemini => S::Native,
            P::DeepSeek | P::OllamaCloud | P::GoogleVertexAi => S::Unsupported,
        },
        C::ThroughputRequestType => match provider {
            P::GoogleVertexAi => S::Native,
            P::OpenAi | P::Anthropic | P::DeepSeek | P::OllamaCloud | P::GoogleGemini => {
                S::Unsupported
            }
        },
        C::ProjectAndLocation => match provider {
            P::GoogleVertexAi => S::Native,
            P::OpenAi | P::Anthropic | P::DeepSeek | P::OllamaCloud | P::GoogleGemini => {
                S::Unsupported
            }
        },
        C::RoleAlternation => S::LocalTransform,
        C::TokenizerOverride => S::EstimationOnly,
        C::AdditionalParameters => S::Restricted,
        C::McpTools => S::SeparateSecurityBoundary,
        C::EndpointOverride => S::Forbidden,
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, CapabilitySupport, capability_support};
    use crate::ProviderId;

    #[test]
    fn dangerous_surfaces_are_never_native_provider_options() {
        for provider in ProviderId::ALL {
            assert_eq!(
                capability_support(provider, Capability::McpTools),
                CapabilitySupport::SeparateSecurityBoundary
            );
            assert_eq!(
                capability_support(provider, Capability::EndpointOverride),
                CapabilitySupport::Forbidden
            );
            assert_eq!(
                capability_support(provider, Capability::AdditionalParameters),
                CapabilitySupport::Restricted
            );
        }
    }

    #[test]
    fn provider_specific_controls_do_not_leak_across_the_matrix() {
        assert_eq!(
            capability_support(ProviderId::GoogleVertexAi, Capability::ProjectAndLocation),
            CapabilitySupport::Native
        );
        assert_eq!(
            capability_support(ProviderId::GoogleGemini, Capability::ProjectAndLocation),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            capability_support(ProviderId::DeepSeek, Capability::PresencePenalty),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            capability_support(ProviderId::GoogleVertexAi, Capability::ServiceTier),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            capability_support(
                ProviderId::GoogleVertexAi,
                Capability::ThroughputRequestType
            ),
            CapabilitySupport::Native
        );
        assert_eq!(
            capability_support(ProviderId::OpenAi, Capability::StopSequences),
            CapabilitySupport::Unsupported
        );
    }
}
