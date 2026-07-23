use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeBackStatus {
    pub supported: bool,
    pub active: bool,
    pub gesture_enabled: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetEnabledRequest {
    pub enabled: bool,
}
