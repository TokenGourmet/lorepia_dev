#![forbid(unsafe_code)]

mod diagnostics;
mod disclosure;
mod error;
mod report;

pub use diagnostics::{
    Architecture, DiagnosticBundle, DiagnosticCode, DiagnosticInput, Platform, StorageDiagnostic,
    build_diagnostic_bundle, serialize_diagnostic_bundle,
};
pub use disclosure::{
    CredentialHandling, DataCategory, DataDestination, DiagnosticsPolicy, ExecutableImportPolicy,
    ProductSafetyContract, ProviderKind, ReleaseProfile, SupportReadiness, product_safety_contract,
};
pub use error::{Result, SafetyError};
pub use report::{
    AI_REPORT_MEDIA_TYPE, AiOutputReportDraft, AiOutputReportInput, ReportCategory,
    build_ai_output_report_draft, serialize_ai_output_report_draft,
};

pub const SAFETY_CONTRACT_VERSION: u16 = 1;
