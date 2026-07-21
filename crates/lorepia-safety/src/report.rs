use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ProviderKind, Result, SAFETY_CONTRACT_VERSION, SafetyError};

pub const AI_REPORT_MEDIA_TYPE: &str = "application/vnd.lorepia.ai-output-report+json";
pub const MAX_REPORT_COMMENT_BYTES: usize = 4 * 1024;
pub const MAX_REPORT_EXCERPT_BYTES: usize = 16 * 1024;
const MAX_MESSAGE_ID_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReportCategory {
    SafetyConcern,
    HarassmentOrHate,
    SexualContent,
    SelfHarm,
    IllegalOrDangerous,
    PrivacyConcern,
    CopyrightConcern,
    IncorrectOrLowQuality,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiOutputReportInput {
    pub message_id: String,
    pub provider: ProviderKind,
    pub category: ReportCategory,
    pub user_comment: Option<String>,
    pub selected_output_excerpt: Option<String>,
    pub include_selected_output: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOutputReportDraft {
    pub contract_version: u16,
    pub report_id: String,
    pub created_at_ms: u64,
    pub message_id: String,
    pub provider: ProviderKind,
    pub category: ReportCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_output_excerpt: Option<String>,
    pub contains_user_selected_content: bool,
    pub ready_for_user_review: bool,
    pub submitted: bool,
    pub network_request_created: bool,
}

pub fn build_ai_output_report_draft(
    input: AiOutputReportInput,
    created_at_ms: u64,
) -> Result<AiOutputReportDraft> {
    validate_message_id(&input.message_id)?;
    let user_comment =
        normalize_optional_text(input.user_comment, MAX_REPORT_COMMENT_BYTES, "user_comment")?;
    let selected_output_excerpt = normalize_optional_text(
        input.selected_output_excerpt,
        MAX_REPORT_EXCERPT_BYTES,
        "selected_output_excerpt",
    )?;
    match (
        input.include_selected_output,
        selected_output_excerpt.is_some(),
    ) {
        (false, true) => return Err(SafetyError::ContentConsentRequired),
        (true, false) => return Err(SafetyError::ContentConsentMismatch),
        _ => {}
    }

    Ok(AiOutputReportDraft {
        contract_version: SAFETY_CONTRACT_VERSION,
        report_id: Uuid::new_v4().simple().to_string(),
        created_at_ms,
        message_id: input.message_id,
        provider: input.provider,
        category: input.category,
        user_comment,
        selected_output_excerpt,
        contains_user_selected_content: input.include_selected_output,
        ready_for_user_review: true,
        submitted: false,
        network_request_created: false,
    })
}

pub fn serialize_ai_output_report_draft(draft: &AiOutputReportDraft) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(draft).map_err(|_| SafetyError::SerializationFailed)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn validate_message_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_MESSAGE_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(SafetyError::InvalidField("message_id"));
    }
    Ok(())
}

fn normalize_optional_text(
    value: Option<String>,
    max_bytes: usize,
    field: &'static str,
) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.len() > max_bytes {
        return Err(SafetyError::FieldTooLarge(field));
    }
    if value.contains('\0') {
        return Err(SafetyError::InvalidField(field));
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_owned()))
}
