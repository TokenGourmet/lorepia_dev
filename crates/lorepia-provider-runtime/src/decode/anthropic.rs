use serde_json::Value;

use super::{
    DecodedFrame, completed, ensure_no_terminal, map_common_reason, parse_json, protocol_error,
    provider_error, response_id_event, value_u64,
};
use crate::{ProviderRunOutcome, ProviderStreamEvent, Result, TokenUsage, framing::SseFrame};

#[derive(Default)]
pub(crate) struct AnthropicDecoder {
    usage: TokenUsage,
    terminal: Option<ProviderRunOutcome>,
    finish_reason: Option<String>,
    response_id_seen: bool,
}

impl AnthropicDecoder {
    pub(crate) fn decode(&mut self, frame: &SseFrame) -> Result<DecodedFrame> {
        ensure_no_terminal(&self.terminal)?;
        let value = parse_json(&frame.data)?;
        let event_type = value.get("type").and_then(Value::as_str).ok_or_else(|| {
            protocol_error("MISSING_EVENT_TYPE", "Anthropic event did not contain type")
        })?;
        if frame
            .event
            .as_deref()
            .is_some_and(|name| name != event_type)
        {
            return Err(protocol_error(
                "EVENT_TYPE_MISMATCH",
                "Anthropic SSE event name did not match its JSON type",
            ));
        }
        let mut output = DecodedFrame::default();
        match event_type {
            "message_start" => {
                if !self.response_id_seen
                    && let Some(id) = value.pointer("/message/id").and_then(Value::as_str)
                {
                    output.events.push(response_id_event(id)?);
                    self.response_id_seen = true;
                }
                merge_start_usage(&mut self.usage, &value);
            }
            "content_block_start" => {
                let kind = value
                    .pointer("/content_block/type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        protocol_error(
                            "INVALID_CONTENT_BLOCK",
                            "Anthropic content block did not contain a type",
                        )
                    })?;
                if !matches!(kind, "text" | "thinking" | "redacted_thinking") {
                    return Err(provider_error(
                        "UNEXPECTED_TOOL_EVENT",
                        "provider emitted a non-text content block although tools are disabled",
                    ));
                }
            }
            "content_block_delta" => match value.pointer("/delta/type").and_then(Value::as_str) {
                Some("text_delta") => output.events.push(ProviderStreamEvent::TextDelta {
                    text: required_string(&value, "/delta/text")?.to_owned(),
                }),
                Some("thinking_delta") => {
                    output.events.push(ProviderStreamEvent::ReasoningDelta {
                        text: required_string(&value, "/delta/thinking")?.to_owned(),
                    });
                }
                Some("signature_delta") => {}
                Some("input_json_delta") => {
                    return Err(provider_error(
                        "UNEXPECTED_TOOL_EVENT",
                        "provider emitted tool input although tools are disabled",
                    ));
                }
                Some(_) => {}
                None => {
                    return Err(protocol_error(
                        "INVALID_CONTENT_DELTA",
                        "Anthropic content delta did not contain a delta type",
                    ));
                }
            },
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.finish_reason = Some(reason.to_owned());
                }
                self.usage.merge_from(&TokenUsage {
                    output_tokens: value_u64(&value, "/usage/output_tokens"),
                    ..TokenUsage::default()
                });
            }
            "message_stop" => {
                let reason = self.finish_reason.as_deref().map(map_common_reason);
                let terminal = completed(reason, &self.usage);
                self.terminal = Some(terminal.clone());
                output.terminal = Some(terminal);
            }
            "error" => {
                return Err(provider_error(
                    value
                        .pointer("/error/type")
                        .and_then(Value::as_str)
                        .unwrap_or("ANTHROPIC_STREAM_ERROR"),
                    value
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Anthropic stream failed"),
                ));
            }
            "ping" | "content_block_stop" => {}
            _ => {}
        }
        Ok(output)
    }

    pub(crate) fn finish(self) -> Result<ProviderRunOutcome> {
        self.terminal.ok_or_else(|| {
            protocol_error(
                "MISSING_TERMINAL_EVENT",
                "Anthropic stream ended without message_stop",
            )
        })
    }
}

fn required_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            protocol_error(
                "INVALID_CONTENT_DELTA",
                "Anthropic content delta was missing text",
            )
        })
}

fn merge_start_usage(usage: &mut TokenUsage, value: &Value) {
    let input = value_u64(value, "/message/usage/input_tokens");
    let cached = value_u64(value, "/message/usage/cache_read_input_tokens");
    usage.merge_from(&TokenUsage {
        input_tokens: input,
        output_tokens: value_u64(value, "/message/usage/output_tokens"),
        cached_input_tokens: cached,
        total_tokens: input
            .zip(value_u64(value, "/message/usage/output_tokens"))
            .map(|(input, output)| input.saturating_add(output)),
        ..TokenUsage::default()
    });
}
