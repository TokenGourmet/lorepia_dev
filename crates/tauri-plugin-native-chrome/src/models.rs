use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeChromeTab {
    Home,
    #[default]
    Library,
    Create,
    Account,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeChromeAppearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeChromeState {
    pub visible: bool,
    pub selected_tab: NativeChromeTab,
    pub minimized: bool,
    pub appearance: NativeChromeAppearance,
    pub compact: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeChromeStatus {
    pub supported: bool,
    pub active: bool,
    pub compact: bool,
    pub visible: bool,
    pub selected_tab: NativeChromeTab,
    pub minimized: bool,
}

#[cfg(test)]
mod tests {
    use super::{NativeChromeAppearance, NativeChromeState, NativeChromeStatus, NativeChromeTab};

    #[test]
    fn state_uses_the_closed_camel_case_wire_contract() {
        let state = NativeChromeState {
            visible: true,
            selected_tab: NativeChromeTab::Create,
            minimized: true,
            appearance: NativeChromeAppearance::Dark,
            compact: true,
        };

        let encoded = serde_json::to_value(state).expect("state should serialize");
        assert_eq!(
            encoded,
            serde_json::json!({
                "visible": true,
                "selectedTab": "create",
                "minimized": true,
                "appearance": "dark",
                "compact": true,
            })
        );
    }

    #[test]
    fn unsupported_status_fails_closed() {
        assert_eq!(
            NativeChromeStatus::default(),
            NativeChromeStatus {
                supported: false,
                active: false,
                compact: false,
                visible: false,
                selected_tab: NativeChromeTab::Library,
                minimized: false,
            }
        );
    }
}
