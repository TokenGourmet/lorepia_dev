#![forbid(unsafe_code)]

use serde::Serialize;

pub const PRODUCT_BOOTSTRAP_CONTRACT_VERSION: u16 = 2;

/// The platform-independent product state owned by the native application.
///
/// This is deliberately small. Storage, providers, imports, scripting, and
/// audio are added only after their product contracts have passed review.
#[derive(Debug, Default)]
pub struct LorePiaCore;

impl LorePiaCore {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn product_bootstrap(&self) -> ProductBootstrap {
        ProductBootstrap {
            contract_version: PRODUCT_BOOTSTRAP_CONTRACT_VERSION,
            product_name: "LorePia",
            core_version: env!("CARGO_PKG_VERSION"),
            data_policy: DataPolicy::DeviceLocalExceptUserSelectedLlmRequests,
            imported_executable_content: ImportedExecutableContent::DisabledBySecurityPolicy,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum DataPolicy {
    DeviceLocalExceptUserSelectedLlmRequests,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ImportedExecutableContent {
    DisabledBySecurityPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductBootstrap {
    contract_version: u16,
    product_name: &'static str,
    core_version: &'static str,
    data_policy: DataPolicy,
    imported_executable_content: ImportedExecutableContent,
}

#[cfg(test)]
mod tests {
    use super::{LorePiaCore, PRODUCT_BOOTSTRAP_CONTRACT_VERSION};
    use serde_json::json;

    #[test]
    fn bootstrap_contract_has_the_exact_safe_startup_shape() {
        let actual = serde_json::to_value(LorePiaCore::new().product_bootstrap())
            .expect("bootstrap must serialize");

        assert_eq!(
            actual,
            json!({
                "contractVersion": PRODUCT_BOOTSTRAP_CONTRACT_VERSION,
                "productName": "LorePia",
                "coreVersion": env!("CARGO_PKG_VERSION"),
                "dataPolicy": "DEVICE_LOCAL_EXCEPT_USER_SELECTED_LLM_REQUESTS",
                "importedExecutableContent": "DISABLED_BY_SECURITY_POLICY"
            })
        );
    }

    #[test]
    fn bootstrap_is_deterministic() {
        let core = LorePiaCore::new();
        assert_eq!(core.product_bootstrap(), core.product_bootstrap());
    }
}
