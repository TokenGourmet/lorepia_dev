use serde_json::Value;

use super::{
    DecodedFrame, completed, ensure_no_terminal, map_common_reason, parse_json, protocol_error,
    provider_error, value_u64,
};
use crate::{ProviderRunOutcome, ProviderStreamEvent, Result, TokenUsage};

#[derive(Default)]
pub(crate) struct OllamaDecoder {
    usage: TokenUsage,
    terminal: Option<ProviderRunOutcome>,
}

impl OllamaDecoder {
    pub(crate) fn decode(&mut self, frame: &str) -> Result<DecodedFrame> {
        ensure_no_terminal(&self.terminal)?;
        let value = parse_json(frame)?;
        if let Some(message) = value.get("error").and_then(Value::as_str) {
            return Err(provider_error("OLLAMA_STREAM_ERROR", message));
        }
        let mut output = DecodedFrame::default();
        if let Some(message) = value.get("message") {
            if message
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty())
            {
                return Err(provider_error(
                    "UNEXPECTED_TOOL_EVENT",
                    "provider emitted a tool call although tools are disabled",
                ));
            }
            if let Some(thinking) = message.get("thinking").and_then(Value::as_str)
                && !thinking.is_empty()
            {
                output.events.push(ProviderStreamEvent::ReasoningDelta {
                    text: thinking.to_owned(),
                });
            }
            if let Some(text) = message.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                output.events.push(ProviderStreamEvent::TextDelta {
                    text: text.to_owned(),
                });
            }
        }

        if value.get("done").and_then(Value::as_bool) == Some(true) {
            let input = value_u64(&value, "/prompt_eval_count");
            let output_tokens = value_u64(&value, "/eval_count");
            self.usage.merge_from(&TokenUsage {
                input_tokens: input,
                output_tokens,
                total_tokens: input
                    .zip(output_tokens)
                    .map(|(input, output)| input.saturating_add(output)),
                ..TokenUsage::default()
            });
            if !self.usage.is_empty() {
                output.events.push(ProviderStreamEvent::Usage {
                    usage: self.usage.clone(),
                });
            }
            let reason = value
                .get("done_reason")
                .and_then(Value::as_str)
                .map(map_common_reason);
            let terminal = completed(reason, &self.usage);
            self.terminal = Some(terminal.clone());
            output.terminal = Some(terminal);
        } else if value.get("done").and_then(Value::as_bool) != Some(false) {
            return Err(protocol_error(
                "INVALID_OLLAMA_CHUNK",
                "Ollama chunk did not contain a boolean done field",
            ));
        }
        Ok(output)
    }

    pub(crate) fn finish(self) -> Result<ProviderRunOutcome> {
        self.terminal.ok_or_else(|| {
            protocol_error(
                "MISSING_TERMINAL_EVENT",
                "Ollama stream ended without done=true",
            )
        })
    }
}
