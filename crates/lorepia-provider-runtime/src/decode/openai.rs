use serde_json::Value;

use super::{
    DecodedFrame, completed, ensure_no_terminal, map_common_reason, parse_json, protocol_error,
    provider_error, response_id_event, value_u64,
};
use crate::{
    CompletionReason, ProviderRunOutcome, ProviderStreamEvent, Result, TokenUsage,
    framing::SseFrame,
};

#[derive(Default)]
pub(crate) struct OpenAiDecoder {
    usage: TokenUsage,
    terminal: Option<ProviderRunOutcome>,
    response_id_seen: bool,
}

impl OpenAiDecoder {
    pub(crate) fn decode(&mut self, frame: &SseFrame) -> Result<DecodedFrame> {
        ensure_no_terminal(&self.terminal)?;
        let value = parse_json(&frame.data)?;
        let event_type = value.get("type").and_then(Value::as_str).ok_or_else(|| {
            protocol_error("MISSING_EVENT_TYPE", "OpenAI event did not contain type")
        })?;
        if frame
            .event
            .as_deref()
            .is_some_and(|name| name != "message" && name != event_type)
        {
            return Err(protocol_error(
                "EVENT_TYPE_MISMATCH",
                "OpenAI SSE event name did not match its JSON type",
            ));
        }
        if event_type.contains("_call") {
            return Err(provider_error(
                "UNEXPECTED_TOOL_EVENT",
                "provider emitted a tool call although tools are disabled",
            ));
        }

        let mut output = DecodedFrame::default();
        match event_type {
            "response.created" => {
                if !self.response_id_seen
                    && let Some(id) = value.pointer("/response/id").and_then(Value::as_str)
                {
                    output.events.push(response_id_event(id)?);
                    self.response_id_seen = true;
                }
            }
            "response.output_text.delta" => {
                output.events.push(ProviderStreamEvent::TextDelta {
                    text: required_delta(&value)?.to_owned(),
                });
            }
            "response.reasoning_summary_text.delta" => {
                output.events.push(ProviderStreamEvent::ReasoningDelta {
                    text: required_delta(&value)?.to_owned(),
                });
            }
            "response.refusal.delta" => {
                output.events.push(ProviderStreamEvent::RefusalDelta {
                    text: required_delta(&value)?.to_owned(),
                });
            }
            "response.completed" => {
                merge_usage(&mut self.usage, &value);
                let terminal = completed(Some(CompletionReason::Stop), &self.usage);
                self.terminal = Some(terminal.clone());
                output.terminal = Some(terminal);
            }
            "response.incomplete" => {
                merge_usage(&mut self.usage, &value);
                let reason = value
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .map(map_common_reason)
                    .or(Some(CompletionReason::Length));
                let terminal = completed(reason, &self.usage);
                self.terminal = Some(terminal.clone());
                output.terminal = Some(terminal);
            }
            "response.failed" => {
                let code = value
                    .pointer("/response/error/code")
                    .and_then(Value::as_str)
                    .unwrap_or("OPENAI_RESPONSE_FAILED");
                let message = value
                    .pointer("/response/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("OpenAI response failed");
                return Err(provider_error(code, message));
            }
            "error" => {
                return Err(provider_error(
                    value
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("OPENAI_STREAM_ERROR"),
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("OpenAI stream failed"),
                ));
            }
            _ => {}
        }
        Ok(output)
    }

    pub(crate) fn finish(self) -> Result<ProviderRunOutcome> {
        self.terminal.ok_or_else(|| {
            protocol_error(
                "MISSING_TERMINAL_EVENT",
                "OpenAI stream ended without a terminal response event",
            )
        })
    }
}

fn required_delta(value: &Value) -> Result<&str> {
    value.get("delta").and_then(Value::as_str).ok_or_else(|| {
        protocol_error(
            "INVALID_TEXT_DELTA",
            "OpenAI delta event did not contain text",
        )
    })
}

fn merge_usage(usage: &mut TokenUsage, value: &Value) {
    usage.merge_from(&TokenUsage {
        input_tokens: value_u64(value, "/response/usage/input_tokens"),
        output_tokens: value_u64(value, "/response/usage/output_tokens"),
        reasoning_tokens: value_u64(
            value,
            "/response/usage/output_tokens_details/reasoning_tokens",
        ),
        cached_input_tokens: value_u64(value, "/response/usage/input_tokens_details/cached_tokens"),
        total_tokens: value_u64(value, "/response/usage/total_tokens"),
    });
}
