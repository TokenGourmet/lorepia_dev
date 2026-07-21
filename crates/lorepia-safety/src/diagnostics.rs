use std::collections::BTreeSet;

use serde::Serialize;

use crate::{Result, SAFETY_CONTRACT_VERSION, SafetyError};

pub const MAX_DIAGNOSTIC_CODES: usize = 64;
pub const MAX_DIAGNOSTIC_BUNDLE_BYTES: usize = 16 * 1024;
const MAX_APP_VERSION_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticCode {
    StorageUnavailable,
    StorageRecoveredInterruptedRequest,
    StorageWalMaintenanceDeferred,
    ProviderNetworkUnavailable,
    ProviderRateLimited,
    ProviderProtocolRejected,
    ProviderStreamCancelled,
    ProviderStreamAckTimeout,
    AssetCatalogNeedsReconciliation,
    BackupInterrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Android,
    Ios,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Architecture {
    X86_64,
    Aarch64,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDiagnostic {
    pub available: bool,
    pub schema_version: Option<u64>,
    pub recovered_interrupted_requests: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticInput<'a> {
    pub generated_at_ms: u64,
    pub app_version: &'a str,
    pub platform: Platform,
    pub architecture: Architecture,
    pub storage: StorageDiagnostic,
    pub recent_codes: &'a [DiagnosticCode],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticPrivacyProof {
    pub contains_api_credentials: bool,
    pub contains_prompt_or_lore_content: bool,
    pub contains_chat_or_persona_content: bool,
    pub contains_file_system_paths: bool,
    pub contains_raw_provider_errors: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticBundle {
    pub contract_version: u16,
    pub generated_at_ms: u64,
    pub app_version: String,
    pub platform: Platform,
    pub architecture: Architecture,
    pub storage: StorageDiagnostic,
    pub recent_codes: Vec<DiagnosticCode>,
    pub privacy: DiagnosticPrivacyProof,
}

pub fn build_diagnostic_bundle(input: DiagnosticInput<'_>) -> Result<DiagnosticBundle> {
    validate_app_version(input.app_version)?;
    if input.recent_codes.len() > MAX_DIAGNOSTIC_CODES {
        return Err(SafetyError::TooManyDiagnosticCodes);
    }
    let recent_codes = input
        .recent_codes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(DiagnosticBundle {
        contract_version: SAFETY_CONTRACT_VERSION,
        generated_at_ms: input.generated_at_ms,
        app_version: input.app_version.to_owned(),
        platform: input.platform,
        architecture: input.architecture,
        storage: input.storage,
        recent_codes,
        privacy: DiagnosticPrivacyProof {
            contains_api_credentials: false,
            contains_prompt_or_lore_content: false,
            contains_chat_or_persona_content: false,
            contains_file_system_paths: false,
            contains_raw_provider_errors: false,
        },
    })
}

pub fn serialize_diagnostic_bundle(bundle: &DiagnosticBundle) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(bundle).map_err(|_| SafetyError::SerializationFailed)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_DIAGNOSTIC_BUNDLE_BYTES {
        return Err(SafetyError::DiagnosticBundleTooLarge);
    }
    Ok(bytes)
}

fn validate_app_version(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_APP_VERSION_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err(SafetyError::InvalidField("app_version"));
    }
    Ok(())
}
