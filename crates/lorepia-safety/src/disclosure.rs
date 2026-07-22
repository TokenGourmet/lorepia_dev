use serde::{Deserialize, Serialize};

use crate::SAFETY_CONTRACT_VERSION;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    DeepSeek,
    OllamaCloud,
    GoogleGemini,
    GoogleVertexAi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DataDestination {
    UserSelectedLlmProviderOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DataCategory {
    CurrentConversationContext,
    ActiveCharacterAndPersonaContext,
    ActivePromptAndLoreContext,
    RequestedMediaWhenProviderSupportsIt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialHandling {
    NativeVaultOnly,
    RequestAuthorizationHeaderOnly,
    NeverInDiagnosticExport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticsPolicy {
    LocalUserInitiatedExportOnly,
    AllowlistedMetadataWithoutUserContent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutableImportPolicy {
    DisabledBySecurityPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReleaseProfile {
    StoreSafe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportReadiness {
    pub privacy_policy_url_configured: bool,
    pub support_url_configured: bool,
    pub remote_report_submission_configured: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductSafetyContract {
    pub contract_version: u16,
    pub release_profile: ReleaseProfile,
    pub request_destination: DataDestination,
    /// Destinations covered by this disclosure. Presence here does not claim
    /// that credentials, OAuth, or a product adapter are configured.
    pub provider_destinations: &'static [ProviderKind],
    pub request_data: &'static [DataCategory],
    pub credentials: &'static [CredentialHandling],
    pub diagnostics: &'static [DiagnosticsPolicy],
    pub imported_javascript: ExecutableImportPolicy,
    pub imported_lua: ExecutableImportPolicy,
    pub support: SupportReadiness,
}

const PROVIDERS: &[ProviderKind] = &[
    ProviderKind::OpenAi,
    ProviderKind::Anthropic,
    ProviderKind::DeepSeek,
    ProviderKind::OllamaCloud,
    ProviderKind::GoogleGemini,
    ProviderKind::GoogleVertexAi,
];

const REQUEST_DATA: &[DataCategory] = &[
    DataCategory::CurrentConversationContext,
    DataCategory::ActiveCharacterAndPersonaContext,
    DataCategory::ActivePromptAndLoreContext,
    DataCategory::RequestedMediaWhenProviderSupportsIt,
];

const CREDENTIALS: &[CredentialHandling] = &[
    CredentialHandling::NativeVaultOnly,
    CredentialHandling::RequestAuthorizationHeaderOnly,
    CredentialHandling::NeverInDiagnosticExport,
];

const DIAGNOSTICS: &[DiagnosticsPolicy] = &[
    DiagnosticsPolicy::LocalUserInitiatedExportOnly,
    DiagnosticsPolicy::AllowlistedMetadataWithoutUserContent,
];

#[must_use]
pub const fn product_safety_contract() -> ProductSafetyContract {
    ProductSafetyContract {
        contract_version: SAFETY_CONTRACT_VERSION,
        release_profile: ReleaseProfile::StoreSafe,
        request_destination: DataDestination::UserSelectedLlmProviderOnly,
        provider_destinations: PROVIDERS,
        request_data: REQUEST_DATA,
        credentials: CREDENTIALS,
        diagnostics: DIAGNOSTICS,
        imported_javascript: ExecutableImportPolicy::DisabledBySecurityPolicy,
        imported_lua: ExecutableImportPolicy::DisabledBySecurityPolicy,
        support: SupportReadiness {
            privacy_policy_url_configured: false,
            support_url_configured: false,
            remote_report_submission_configured: false,
        },
    }
}
