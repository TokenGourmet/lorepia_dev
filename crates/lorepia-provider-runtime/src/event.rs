use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl TokenUsage {
    pub(crate) fn merge_from(&mut self, newer: &Self) {
        if newer.input_tokens.is_some() {
            self.input_tokens = newer.input_tokens;
        }
        if newer.output_tokens.is_some() {
            self.output_tokens = newer.output_tokens;
        }
        if newer.reasoning_tokens.is_some() {
            self.reasoning_tokens = newer.reasoning_tokens;
        }
        if newer.cached_input_tokens.is_some() {
            self.cached_input_tokens = newer.cached_input_tokens;
        }
        if newer.total_tokens.is_some() {
            self.total_tokens = newer.total_tokens;
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionReason {
    Stop,
    Length,
    ContentFilter,
    Refusal,
    ResourceLimit,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProviderStreamEvent {
    ProviderResponseId { id: String },
    TextDelta { text: String },
    ReasoningDelta { text: String },
    RefusalDelta { text: String },
    Usage { usage: TokenUsage },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProviderRunOutcome {
    Completed {
        reason: Option<CompletionReason>,
        usage: Option<TokenUsage>,
    },
    Cancelled,
}
