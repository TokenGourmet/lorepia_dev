mod anthropic;
mod deepseek;
mod google;
mod ollama;
mod openai;

use lorepia_providers::ProviderId;

use crate::{
    CompletionReason, ProviderRunOutcome, ProviderStreamEvent, Result, RuntimeError,
    RuntimeErrorKind, TokenUsage, framing::SseFrame,
};

pub(crate) enum ProviderDecoder {
    OpenAi(openai::OpenAiDecoder),
    Anthropic(anthropic::AnthropicDecoder),
    DeepSeek(deepseek::DeepSeekDecoder),
    Ollama(ollama::OllamaDecoder),
    Google(google::GoogleDecoder),
}

#[derive(Debug, Default)]
pub(crate) struct DecodedFrame {
    pub(crate) events: Vec<ProviderStreamEvent>,
    pub(crate) terminal: Option<ProviderRunOutcome>,
}

impl ProviderDecoder {
    pub(crate) fn new(provider: ProviderId) -> Self {
        match provider {
            ProviderId::OpenAi => Self::OpenAi(openai::OpenAiDecoder::default()),
            ProviderId::Anthropic => Self::Anthropic(anthropic::AnthropicDecoder::default()),
            ProviderId::DeepSeek => Self::DeepSeek(deepseek::DeepSeekDecoder::default()),
            ProviderId::OllamaCloud => Self::Ollama(ollama::OllamaDecoder::default()),
            ProviderId::GoogleGemini | ProviderId::GoogleVertexAi => {
                Self::Google(google::GoogleDecoder::default())
            }
        }
    }

    pub(crate) fn decode_sse(&mut self, frame: &SseFrame) -> Result<DecodedFrame> {
        match self {
            Self::OpenAi(decoder) => decoder.decode(frame),
            Self::Anthropic(decoder) => decoder.decode(frame),
            Self::DeepSeek(decoder) => decoder.decode(frame),
            Self::Google(decoder) => decoder.decode(frame),
            Self::Ollama(_) => Err(protocol_error(
                "WRONG_STREAM_PROTOCOL",
                "Ollama Cloud responses must use NDJSON",
            )),
        }
    }

    pub(crate) fn decode_ndjson(&mut self, frame: &str) -> Result<DecodedFrame> {
        match self {
            Self::Ollama(decoder) => decoder.decode(frame),
            _ => Err(protocol_error(
                "WRONG_STREAM_PROTOCOL",
                "this provider response must use SSE",
            )),
        }
    }

    pub(crate) fn finish(self) -> Result<ProviderRunOutcome> {
        match self {
            Self::OpenAi(decoder) => decoder.finish(),
            Self::Anthropic(decoder) => decoder.finish(),
            Self::DeepSeek(decoder) => decoder.finish(),
            Self::Ollama(decoder) => decoder.finish(),
            Self::Google(decoder) => decoder.finish(),
        }
    }
}

pub(crate) fn completed(
    reason: Option<CompletionReason>,
    usage: &TokenUsage,
) -> ProviderRunOutcome {
    ProviderRunOutcome::Completed {
        reason,
        usage: (!usage.is_empty()).then(|| usage.clone()),
    }
}

pub(crate) fn protocol_error(code: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(RuntimeErrorKind::StreamProtocol, code, message)
}

pub(crate) fn provider_error(code: &str, message: impl Into<String>) -> RuntimeError {
    // Provider-controlled error fields are intentionally discarded. Some
    // upstreams echo request headers or user content in these fields, and this
    // error crosses the native/WebView boundary.
    drop(message.into());
    match code {
        "UNEXPECTED_TOOL_EVENT" => RuntimeError::new(
            RuntimeErrorKind::Provider,
            "UNEXPECTED_TOOL_EVENT",
            "provider emitted a tool event although tools are disabled",
        ),
        "GOOGLE_PROMPT_BLOCKED" => RuntimeError::new(
            RuntimeErrorKind::Provider,
            "GOOGLE_PROMPT_BLOCKED",
            "provider blocked the request according to its safety policy",
        ),
        _ => RuntimeError::new(
            RuntimeErrorKind::Provider,
            "PROVIDER_STREAM_ERROR",
            "provider reported a streaming error",
        ),
    }
}

pub(crate) fn response_id_event(id: &str) -> Result<ProviderStreamEvent> {
    const MAX_RESPONSE_ID_BYTES: usize = 256;
    if id.is_empty()
        || id.len() > MAX_RESPONSE_ID_BYTES
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(protocol_error(
            "INVALID_PROVIDER_RESPONSE_ID",
            "provider response id was missing, oversized, or contained unsafe characters",
        ));
    }
    Ok(ProviderStreamEvent::ProviderResponseId { id: id.to_owned() })
}

pub(crate) fn parse_json(data: &str) -> Result<serde_json::Value> {
    serde_json::from_str(data).map_err(|_| {
        protocol_error(
            "INVALID_PROVIDER_JSON",
            "provider stream contained malformed JSON",
        )
    })
}

pub(crate) fn value_u64(value: &serde_json::Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(serde_json::Value::as_u64)
}

pub(crate) fn map_common_reason(reason: &str) -> CompletionReason {
    match reason.to_ascii_lowercase().as_str() {
        "stop" | "end_turn" | "stop_sequence" => CompletionReason::Stop,
        "length" | "max_tokens" | "max_output_tokens" => CompletionReason::Length,
        "content_filter" | "safety" | "recitation" | "blocklist" | "prohibited_content"
        | "spii" | "image_safety" | "refusal" => CompletionReason::ContentFilter,
        "insufficient_system_resource" | "resource_limit" => CompletionReason::ResourceLimit,
        _ => CompletionReason::Other("other".to_owned()),
    }
}

pub(crate) fn ensure_no_terminal(terminal: &Option<ProviderRunOutcome>) -> Result<()> {
    if terminal.is_some() {
        Err(protocol_error(
            "EVENT_AFTER_TERMINAL",
            "provider emitted data after a terminal event",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::{NdjsonFramer, SseFramer};

    fn decode_sse_fixture(
        provider: ProviderId,
        fixture: &[u8],
        split: usize,
    ) -> (Vec<ProviderStreamEvent>, ProviderRunOutcome) {
        let mut framer = SseFramer::new(16 * 1024);
        let mut decoder = ProviderDecoder::new(provider);
        let mut events = Vec::new();
        let mut terminal = None;
        for chunk in [&fixture[..split], &fixture[split..]] {
            for frame in framer.push(chunk).expect("valid fixture framing") {
                let decoded = decoder.decode_sse(&frame).expect("valid provider fixture");
                events.extend(decoded.events);
                if decoded.terminal.is_some() {
                    assert!(terminal.is_none(), "only one terminal event is permitted");
                    terminal = decoded.terminal;
                }
            }
        }
        framer.finish().expect("complete SSE fixture");
        (
            events,
            terminal.expect("fixture must contain a terminal event"),
        )
    }

    fn decode_ndjson_fixture(
        fixture: &[u8],
        split: usize,
    ) -> (Vec<ProviderStreamEvent>, ProviderRunOutcome) {
        let mut framer = NdjsonFramer::new(16 * 1024);
        let mut decoder = ProviderDecoder::new(ProviderId::OllamaCloud);
        let mut events = Vec::new();
        let mut terminal = None;
        let mut frames = Vec::new();
        frames.extend(
            framer
                .push(&fixture[..split])
                .expect("valid fixture framing"),
        );
        frames.extend(
            framer
                .push(&fixture[split..])
                .expect("valid fixture framing"),
        );
        frames.extend(framer.finish().expect("complete NDJSON fixture"));
        for frame in frames {
            let decoded = decoder
                .decode_ndjson(&frame)
                .expect("valid provider fixture");
            events.extend(decoded.events);
            if decoded.terminal.is_some() {
                assert!(terminal.is_none(), "only one terminal event is permitted");
                terminal = decoded.terminal;
            }
        }
        (
            events,
            terminal.expect("fixture must contain a terminal event"),
        )
    }

    fn assert_every_sse_boundary(provider: ProviderId, fixture: &[u8]) {
        let baseline = decode_sse_fixture(provider, fixture, 0);
        for split in 0..=fixture.len() {
            assert_eq!(
                decode_sse_fixture(provider, fixture, split),
                baseline,
                "provider {provider:?}, split {split}"
            );
        }
    }

    #[test]
    fn openai_fixture_survives_every_fragment_boundary() {
        let fixture = br#"event: response.created
data: {"type":"response.created","response":{"id":"resp_123"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello"}

event: response.completed
data: {"type":"response.completed","response":{"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}}

"#;
        assert_every_sse_boundary(ProviderId::OpenAi, fixture);
        let (events, outcome) = decode_sse_fixture(ProviderId::OpenAi, fixture, 11);
        assert!(events.contains(&ProviderStreamEvent::TextDelta {
            text: "hello".to_owned(),
        }));
        assert!(matches!(
            outcome,
            ProviderRunOutcome::Completed {
                reason: Some(CompletionReason::Stop),
                ..
            }
        ));
    }

    #[test]
    fn anthropic_fixture_survives_every_fragment_boundary() {
        let fixture = br#"event: message_start
data: {"type":"message_start","message":{"id":"msg_123","usage":{"input_tokens":3,"output_tokens":0}}}

event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"plan"}}

event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}

event: message_stop
data: {"type":"message_stop"}

"#;
        assert_every_sse_boundary(ProviderId::Anthropic, fixture);
    }

    #[test]
    fn deepseek_fixture_survives_every_fragment_boundary() {
        let fixture = br#"data: {"id":"chat_123","choices":[{"index":0,"delta":{"reasoning_content":"plan","content":"hello"},"finish_reason":null}]}

data: {"id":"chat_123","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}

data: [DONE]

"#;
        assert_every_sse_boundary(ProviderId::DeepSeek, fixture);
    }

    #[test]
    fn google_fixture_is_shared_by_gemini_and_vertex() {
        let fixture = br#"data: {"responseId":"google_123","candidates":[{"index":0,"content":{"parts":[{"text":"plan","thought":true},{"text":"hello"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":1,"thoughtsTokenCount":1,"totalTokenCount":5}}

"#;
        assert_every_sse_boundary(ProviderId::GoogleGemini, fixture);
        assert_every_sse_boundary(ProviderId::GoogleVertexAi, fixture);
    }

    #[test]
    fn ollama_fixture_survives_every_fragment_boundary() {
        let fixture = br#"{"message":{"thinking":"plan","content":"hello"},"done":false}
{"message":{"content":""},"done":true,"done_reason":"stop","prompt_eval_count":3,"eval_count":1}
"#;
        let baseline = decode_ndjson_fixture(fixture, 0);
        for split in 0..=fixture.len() {
            assert_eq!(
                decode_ndjson_fixture(fixture, split),
                baseline,
                "split {split}"
            );
        }
    }

    #[test]
    fn provider_controlled_errors_are_not_reflected() {
        let secret = "Bearer sk-super-secret";
        let mut decoder = ProviderDecoder::new(ProviderId::OpenAi);
        let error = decoder
            .decode_sse(&SseFrame {
                event: Some("error".to_owned()),
                data: format!(
                    r#"{{"type":"error","code":"{secret}","message":"Authorization: {secret}"}}"#
                ),
            })
            .expect_err("provider error must fail");
        assert_eq!(error.code(), "PROVIDER_STREAM_ERROR");
        assert!(!format!("{error:?}").contains(secret));
    }

    #[test]
    fn response_ids_and_unknown_finish_reasons_are_bounded() {
        let oversized = "a".repeat(257);
        assert_eq!(
            response_id_event(&oversized)
                .expect_err("oversized id must fail")
                .code(),
            "INVALID_PROVIDER_RESPONSE_ID"
        );
        assert_eq!(
            map_common_reason(&"provider-controlled".repeat(100)),
            CompletionReason::Other("other".to_owned())
        );
    }
}
