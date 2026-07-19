use serde_json::Value;

use super::{
    DecodedFrame, completed, ensure_no_terminal, map_common_reason, parse_json, protocol_error,
    provider_error, response_id_event, value_u64,
};
use crate::{ProviderRunOutcome, ProviderStreamEvent, Result, TokenUsage, framing::SseFrame};

#[derive(Default)]
pub(crate) struct GoogleDecoder {
    usage: TokenUsage,
    terminal: Option<ProviderRunOutcome>,
    response_id: Option<String>,
}

impl GoogleDecoder {
    pub(crate) fn decode(&mut self, frame: &SseFrame) -> Result<DecodedFrame> {
        ensure_no_terminal(&self.terminal)?;
        let value = parse_json(&frame.data)?;
        if let Some(error) = value.get("error") {
            return Err(provider_error(
                error
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("GOOGLE_STREAM_ERROR"),
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Google model stream failed"),
            ));
        }
        if let Some(reason) = value
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
        {
            return Err(provider_error(
                "GOOGLE_PROMPT_BLOCKED",
                format!("Google blocked the prompt: {reason}"),
            ));
        }
        let mut output = DecodedFrame::default();
        if let Some(id) = value.get("responseId").and_then(Value::as_str)
            && self.response_id.as_deref() != Some(id)
        {
            if self.response_id.is_some() {
                return Err(protocol_error(
                    "RESPONSE_ID_CHANGED",
                    "Google response id changed during a stream",
                ));
            }
            let event = response_id_event(id)?;
            self.response_id = Some(id.to_owned());
            output.events.push(event);
        }
        merge_usage(&mut self.usage, &value);
        let candidates = value
            .get("candidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if candidates.len() > 1 {
            return Err(protocol_error(
                "MULTIPLE_CANDIDATES",
                "LorePia accepts exactly one streamed candidate",
            ));
        }
        if let Some(candidate) = candidates.first() {
            if candidate.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                return Err(protocol_error(
                    "INVALID_CANDIDATE_INDEX",
                    "Google streamed a nonzero candidate index",
                ));
            }
            if let Some(parts) = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
            {
                for part in parts {
                    if part.get("functionCall").is_some()
                        || part.get("toolCall").is_some()
                        || part.get("executableCode").is_some()
                    {
                        return Err(provider_error(
                            "UNEXPECTED_TOOL_EVENT",
                            "provider emitted executable or tool content although tools are disabled",
                        ));
                    }
                    if let Some(text) = part.get("text").and_then(Value::as_str)
                        && !text.is_empty()
                    {
                        let event = if part.get("thought").and_then(Value::as_bool) == Some(true) {
                            ProviderStreamEvent::ReasoningDelta {
                                text: text.to_owned(),
                            }
                        } else {
                            ProviderStreamEvent::TextDelta {
                                text: text.to_owned(),
                            }
                        };
                        output.events.push(event);
                    }
                }
            }
            if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
                if matches!(
                    reason,
                    "UNEXPECTED_TOOL_CALL" | "TOO_MANY_TOOL_CALLS" | "MALFORMED_FUNCTION_CALL"
                ) {
                    return Err(provider_error(
                        "UNEXPECTED_TOOL_EVENT",
                        format!("Google stopped for a disabled tool action: {reason}"),
                    ));
                }
                let terminal = completed(Some(map_common_reason(reason)), &self.usage);
                self.terminal = Some(terminal.clone());
                output.terminal = Some(terminal);
            }
        }
        if !self.usage.is_empty() {
            output.events.push(ProviderStreamEvent::Usage {
                usage: self.usage.clone(),
            });
        }
        Ok(output)
    }

    pub(crate) fn finish(self) -> Result<ProviderRunOutcome> {
        self.terminal.ok_or_else(|| {
            protocol_error(
                "MISSING_TERMINAL_EVENT",
                "Google stream ended without candidate finishReason",
            )
        })
    }
}

fn merge_usage(usage: &mut TokenUsage, value: &Value) {
    usage.merge_from(&TokenUsage {
        input_tokens: value_u64(value, "/usageMetadata/promptTokenCount"),
        output_tokens: value_u64(value, "/usageMetadata/candidatesTokenCount"),
        reasoning_tokens: value_u64(value, "/usageMetadata/thoughtsTokenCount"),
        cached_input_tokens: value_u64(value, "/usageMetadata/cachedContentTokenCount"),
        total_tokens: value_u64(value, "/usageMetadata/totalTokenCount"),
    });
}
