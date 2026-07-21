use lorepia_safety::{
    AiOutputReportInput, Architecture, DiagnosticCode, DiagnosticInput, Platform, ProviderKind,
    ReportCategory, SafetyError, StorageDiagnostic, build_ai_output_report_draft,
    build_diagnostic_bundle, product_safety_contract, serialize_ai_output_report_draft,
    serialize_diagnostic_bundle,
};

#[test]
fn store_safe_contract_is_explicit_about_missing_release_support_configuration() {
    let value = serde_json::to_value(product_safety_contract()).unwrap();
    assert_eq!(value["releaseProfile"], "STORE_SAFE");
    assert_eq!(value["importedJavascript"], "DISABLED_BY_SECURITY_POLICY");
    assert_eq!(value["importedLua"], "DISABLED_BY_SECURITY_POLICY");
    assert_eq!(
        value["requestDestination"],
        "USER_SELECTED_LLM_PROVIDER_ONLY"
    );
    assert!(
        !value["support"]["privacyPolicyUrlConfigured"]
            .as_bool()
            .unwrap()
    );
    assert!(!value["support"]["supportUrlConfigured"].as_bool().unwrap());
    assert!(
        !value["support"]["remoteReportSubmissionConfigured"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn diagnostics_have_no_field_that_can_carry_secrets_prompts_messages_or_paths() {
    let bundle = build_diagnostic_bundle(DiagnosticInput {
        generated_at_ms: 1_234,
        app_version: "0.1.0-test+1",
        platform: Platform::Android,
        architecture: Architecture::Aarch64,
        storage: StorageDiagnostic {
            available: true,
            schema_version: Some(3),
            recovered_interrupted_requests: 2,
        },
        recent_codes: &[
            DiagnosticCode::ProviderRateLimited,
            DiagnosticCode::ProviderRateLimited,
            DiagnosticCode::StorageRecoveredInterruptedRequest,
        ],
    })
    .unwrap();
    let bytes = serialize_diagnostic_bundle(&bundle).unwrap();
    let text = String::from_utf8(bytes).unwrap();

    assert_eq!(bundle.recent_codes.len(), 2);
    assert!(!text.contains("diagnostic-sentinel-secret"));
    assert!(!text.contains("prompt"));
    assert!(!text.contains("messageContent"));
    assert!(!text.contains("filePath"));
    assert!(!bundle.privacy.contains_api_credentials);
    assert!(!bundle.privacy.contains_prompt_or_lore_content);
    assert!(!bundle.privacy.contains_chat_or_persona_content);
}

#[test]
fn diagnostics_reject_arbitrary_version_text_and_unbounded_code_lists() {
    let bad_version = build_diagnostic_bundle(DiagnosticInput {
        generated_at_ms: 0,
        app_version: "0.1.0\nsecret",
        platform: Platform::Linux,
        architecture: Architecture::X86_64,
        storage: StorageDiagnostic {
            available: false,
            schema_version: None,
            recovered_interrupted_requests: 0,
        },
        recent_codes: &[],
    })
    .unwrap_err();
    assert_eq!(bad_version, SafetyError::InvalidField("app_version"));

    let codes = vec![DiagnosticCode::StorageUnavailable; 65];
    let too_many = build_diagnostic_bundle(DiagnosticInput {
        generated_at_ms: 0,
        app_version: "0.1.0",
        platform: Platform::Unknown,
        architecture: Architecture::Unknown,
        storage: StorageDiagnostic {
            available: false,
            schema_version: None,
            recovered_interrupted_requests: 0,
        },
        recent_codes: &codes,
    })
    .unwrap_err();
    assert_eq!(too_many, SafetyError::TooManyDiagnosticCodes);
}

#[test]
fn ai_output_content_requires_explicit_consent_and_is_never_submitted_by_builder() {
    let without_consent = build_ai_output_report_draft(
        AiOutputReportInput {
            message_id: "message_123".to_owned(),
            provider: ProviderKind::Anthropic,
            category: ReportCategory::SafetyConcern,
            user_comment: None,
            selected_output_excerpt: Some("selected text".to_owned()),
            include_selected_output: false,
        },
        5,
    )
    .unwrap_err();
    assert_eq!(without_consent, SafetyError::ContentConsentRequired);

    let draft = build_ai_output_report_draft(
        AiOutputReportInput {
            message_id: "message_123".to_owned(),
            provider: ProviderKind::Anthropic,
            category: ReportCategory::SafetyConcern,
            user_comment: Some("  review this  ".to_owned()),
            selected_output_excerpt: Some(" selected text ".to_owned()),
            include_selected_output: true,
        },
        5,
    )
    .unwrap();
    let serialized = String::from_utf8(serialize_ai_output_report_draft(&draft).unwrap()).unwrap();
    assert_eq!(draft.user_comment.as_deref(), Some("review this"));
    assert_eq!(
        draft.selected_output_excerpt.as_deref(),
        Some("selected text")
    );
    assert!(draft.ready_for_user_review);
    assert!(!draft.submitted);
    assert!(!draft.network_request_created);
    assert!(serialized.contains("selected text"));
}

#[test]
fn ai_report_rejects_oversized_or_malformed_fields() {
    let oversized = build_ai_output_report_draft(
        AiOutputReportInput {
            message_id: "message_123".to_owned(),
            provider: ProviderKind::OpenAi,
            category: ReportCategory::Other,
            user_comment: Some("x".repeat(4097)),
            selected_output_excerpt: None,
            include_selected_output: false,
        },
        0,
    )
    .unwrap_err();
    assert_eq!(oversized, SafetyError::FieldTooLarge("user_comment"));

    let malformed = build_ai_output_report_draft(
        AiOutputReportInput {
            message_id: "message/path".to_owned(),
            provider: ProviderKind::OpenAi,
            category: ReportCategory::Other,
            user_comment: None,
            selected_output_excerpt: None,
            include_selected_output: false,
        },
        0,
    )
    .unwrap_err();
    assert_eq!(malformed, SafetyError::InvalidField("message_id"));
}
