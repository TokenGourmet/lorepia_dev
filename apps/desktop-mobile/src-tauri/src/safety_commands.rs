use std::time::{SystemTime, UNIX_EPOCH};

use lorepia_safety::{
    AI_REPORT_MEDIA_TYPE, AiOutputReportInput, Architecture, DiagnosticCode, DiagnosticInput,
    Platform, ProductSafetyContract, SafetyError, StorageDiagnostic, build_ai_output_report_draft,
    build_diagnostic_bundle, product_safety_contract, serialize_ai_output_report_draft,
    serialize_diagnostic_bundle,
};
use serde::Serialize;
use tauri::State;

use crate::storage_commands::StorageState;

const DIAGNOSTIC_MEDIA_TYPE: &str = "application/vnd.lorepia.diagnostics+json";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafetyCommandError {
    code: &'static str,
    message: &'static str,
}

impl From<SafetyError> for SafetyCommandError {
    fn from(error: SafetyError) -> Self {
        Self {
            code: error.code(),
            message: error.public_message(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafetyArtifactResponse {
    file_name: &'static str,
    media_type: &'static str,
    byte_length: usize,
    json: String,
}

#[tauri::command]
pub(crate) fn get_product_safety_contract() -> ProductSafetyContract {
    product_safety_contract()
}

#[tauri::command]
pub(crate) fn prepare_ai_output_report(
    input: AiOutputReportInput,
) -> Result<SafetyArtifactResponse, SafetyCommandError> {
    let draft = build_ai_output_report_draft(input, unix_time_ms()?)?;
    let bytes = serialize_ai_output_report_draft(&draft)?;
    let byte_length = bytes.len();
    let json = String::from_utf8(bytes).map_err(|_| SafetyCommandError {
        code: "SAFETY_SERIALIZATION_FAILED",
        message: "safety artifact could not be serialized",
    })?;
    Ok(SafetyArtifactResponse {
        file_name: "lorepia-ai-output-report.json",
        media_type: AI_REPORT_MEDIA_TYPE,
        byte_length,
        json,
    })
}

#[tauri::command]
pub(crate) async fn export_redacted_diagnostics(
    storage: State<'_, StorageState>,
) -> Result<SafetyArtifactResponse, SafetyCommandError> {
    let startup = storage
        .run_read(|store| Ok(store.startup_report().clone()))
        .await;
    let (storage, codes) = match startup {
        Ok(report) => {
            let schema_version = u64::try_from(report.schema_version).ok();
            let mut codes = Vec::new();
            if report.recovered_request_count > 0 {
                codes.push(DiagnosticCode::StorageRecoveredInterruptedRequest);
            }
            (
                StorageDiagnostic {
                    available: true,
                    schema_version,
                    recovered_interrupted_requests: report.recovered_request_count,
                },
                codes,
            )
        }
        Err(_) => (
            StorageDiagnostic {
                available: false,
                schema_version: None,
                recovered_interrupted_requests: 0,
            },
            vec![DiagnosticCode::StorageUnavailable],
        ),
    };
    let bundle = build_diagnostic_bundle(DiagnosticInput {
        generated_at_ms: unix_time_ms()?,
        app_version: env!("CARGO_PKG_VERSION"),
        platform: current_platform(),
        architecture: current_architecture(),
        storage,
        recent_codes: &codes,
    })?;
    let bytes = serialize_diagnostic_bundle(&bundle)?;
    let byte_length = bytes.len();
    let json = String::from_utf8(bytes).map_err(|_| SafetyCommandError {
        code: "SAFETY_SERIALIZATION_FAILED",
        message: "safety artifact could not be serialized",
    })?;
    Ok(SafetyArtifactResponse {
        file_name: "lorepia-diagnostics.json",
        media_type: DIAGNOSTIC_MEDIA_TYPE,
        byte_length,
        json,
    })
}

fn unix_time_ms() -> Result<u64, SafetyCommandError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SafetyCommandError {
            code: "SYSTEM_CLOCK_INVALID",
            message: "system clock is unavailable",
        })?;
    u64::try_from(elapsed.as_millis()).map_err(|_| SafetyCommandError {
        code: "SYSTEM_CLOCK_INVALID",
        message: "system clock is unavailable",
    })
}

const fn current_platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "android") {
        Platform::Android
    } else if cfg!(target_os = "ios") {
        Platform::Ios
    } else {
        Platform::Unknown
    }
}

const fn current_architecture() -> Architecture {
    if cfg!(target_arch = "x86_64") {
        Architecture::X86_64
    } else if cfg!(target_arch = "aarch64") {
        Architecture::Aarch64
    } else {
        Architecture::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_command_rejects_content_without_explicit_consent() {
        let error = prepare_ai_output_report(AiOutputReportInput {
            message_id: "message_123".to_owned(),
            provider: lorepia_safety::ProviderKind::OpenAi,
            category: lorepia_safety::ReportCategory::Other,
            user_comment: None,
            selected_output_excerpt: Some("selected".to_owned()),
            include_selected_output: false,
        })
        .unwrap_err();
        assert_eq!(error.code, "AI_REPORT_CONTENT_CONSENT_REQUIRED");
    }

    #[test]
    fn platform_and_architecture_are_closed_enums() {
        let _ = current_platform();
        let _ = current_architecture();
    }
}
