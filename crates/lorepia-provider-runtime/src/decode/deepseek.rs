use serde_json::Value;

use super::{
    DecodedFrame, completed, ensure_no_terminal, map_common_reason, parse_json, protocol_error,
    provider_error, response_id_event, value_u64,
};
use crate::{ProviderRunOutcome, ProviderStreamEvent, Result, TokenUsage, framing::SseFrame};

#[derive(Default)]
pub(crate) struct DeepSeekDecoder {
    usage: TokenUsage,
    terminal: Option<ProviderRunOutcome>,
    finish_reason: Option<String>,
    response_id: Option<String>,
}

impl DeepSeekDecoder {
    pub(crate) fn decode(&mut self, frame: &SseFrame) -> Result<DecodedFrame> {
        ensure_no_terminal(&self.terminal)?;
        if frame.data.trim() == "[DONE]" {
            let terminal = completed(
                self.finish_reason.as_deref().map(map_common_reason),
                &self.usage,
            );
            self.terminal = Some(terminal.clone());
            return Ok(DecodedFrame {
                terminal: Some(terminal),
                ..DecodedFrame::default()
            });
        }

        let value = parse_json(&frame.data)?;
        if let Some(error) = value.get("error") {
            return Err(provider_error(
                error
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("DEEPSEEK_STREAM_ERROR"),
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("DeepSeek stream failed"),
            ));
        }
        let mut output = DecodedFrame::default();
        if let Some(id) = value.get("id").and_then(Value::as_str)
            && self.response_id.as_deref() != Some(id)
        {
            if self.response_id.is_some() {
                return Err(protocol_error(
                    "RESPONSE_ID_CHANGED",
                    "DeepSeek response id changed during a stream",
                ));
            }
            let event = response_id_event(id)?;
            self.response_id = Some(id.to_owned());
            output.events.push(event);
        }

        merge_usage(&mut self.usage, &value);
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                protocol_error(
                    "INVALID_CHAT_CHUNK",
                    "DeepSeek chunk did not contain choices",
                )
            })?;
        if choices.len() > 1 {
            return Err(protocol_error(
                "MULTIPLE_CHOICES",
                "LorePia accepts exactly one streamed completion choice",
            ));
        }
        if let Some(choice) = choices.first() {
            if choice.get("index").and_then(Value::as_u64) != Some(0) {
                return Err(protocol_error(
                    "INVALID_CHOICE_INDEX",
                    "DeepSeek streamed a nonzero choice index",
                ));
            }
            let delta = choice.get("delta").ok_or_else(|| {
                protocol_error(
                    "INVALID_CHAT_CHUNK",
                    "DeepSeek choice did not contain delta",
                )
            })?;
            if delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty())
            {
                return Err(provider_error(
                    "UNEXPECTED_TOOL_EVENT",
                    "provider emitted a tool call although tools are disabled",
                ));
            }
            if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str)
                && !text.is_empty()
            {
                output.events.push(ProviderStreamEvent::ReasoningDelta {
                    text: text.to_owned(),
                });
            }
            if let Some(text) = delta.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                output.events.push(ProviderStreamEvent::TextDelta {
                    text: text.to_owned(),
                });
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                if reason == "tool_calls" {
                    return Err(provider_error(
                        "UNEXPECTED_TOOL_EVENT",
                        "provider stopped for a tool call although tools are disabled",
                    ));
                }
                self.finish_reason = Some(reason.to_owned());
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
                "MISSING_DONE_MARKER",
                "DeepSeek stream ended without [DONE]",
            )
        })
    }
}

fn merge_usage(usage: &mut TokenUsage, value: &Value) {
    usage.merge_from(&TokenUsage {
        input_tokens: value_u64(value, "/usage/prompt_tokens"),
        output_tokens: value_u64(value, "/usage/completion_tokens"),
        reasoning_tokens: value_u64(value, "/usage/completion_tokens_details/reasoning_tokens"),
        cached_input_tokens: value_u64(value, "/usage/prompt_cache_hit_tokens"),
        total_tokens: value_u64(value, "/usage/total_tokens"),
    });
}
