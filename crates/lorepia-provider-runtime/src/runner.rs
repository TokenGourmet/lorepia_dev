use std::time::Duration;

use futures_util::StreamExt;
use lorepia_providers::{ProviderId, ProviderRequest, StreamProtocol, compile_request};
use reqwest::{Response, StatusCode, header::CONTENT_TYPE};
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    EndpointSelection, ProviderCredential, ProviderRunOutcome, ProviderStreamEvent, Result,
    RuntimeError, RuntimeErrorKind,
    client::build_http_request,
    decode::{DecodedFrame, ProviderDecoder},
    endpoint::resolve_endpoint,
    framing::{NdjsonFramer, SseFramer},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLimits {
    pub max_request_body_bytes: usize,
    pub max_stream_frame_bytes: usize,
    pub max_http_error_body_bytes: usize,
    pub dns_timeout: Duration,
    pub connect_timeout: Duration,
    pub response_header_timeout: Duration,
    pub stream_idle_timeout: Duration,
    pub overall_timeout: Duration,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_request_body_bytes: 5 * 1024 * 1024,
            max_stream_frame_bytes: 256 * 1024,
            max_http_error_body_bytes: 64 * 1024,
            dns_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(10),
            response_header_timeout: Duration::from_secs(60),
            stream_idle_timeout: Duration::from_secs(120),
            overall_timeout: Duration::from_secs(30 * 60),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProviderRuntime {
    limits: RuntimeLimits,
}

impl ProviderRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: RuntimeLimits::default(),
        }
    }

    pub fn with_limits(limits: RuntimeLimits) -> Result<Self> {
        validate_limits(&limits)?;
        Ok(Self { limits })
    }

    /// Compile, authorize, and stream one provider request.
    ///
    /// `events` must be a bounded channel. The runtime awaits channel capacity,
    /// so consumer pressure propagates to the HTTP body rather than accumulating
    /// an unbounded native queue.
    pub async fn run_stream(
        &self,
        request: ProviderRequest,
        endpoint_selection: EndpointSelection,
        credential: ProviderCredential,
        cancellation: CancellationToken,
        events: mpsc::Sender<ProviderStreamEvent>,
    ) -> Result<ProviderRunOutcome> {
        if events.max_capacity() == usize::MAX {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidRequest,
                "UNBOUNDED_EVENT_CHANNEL",
                "provider runtime requires a bounded event channel",
            ));
        }
        if cancellation.is_cancelled() {
            return Ok(ProviderRunOutcome::Cancelled);
        }
        let compiled = compile_request(&request).map_err(|error| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidRequest,
                "PROVIDER_REQUEST_INVALID",
                error.to_string(),
            )
        })?;
        let endpoint = tokio::select! {
            _ = cancellation.cancelled() => return Ok(ProviderRunOutcome::Cancelled),
            result = resolve_endpoint(
                &request,
                &compiled,
                &endpoint_selection,
                self.limits.dns_timeout,
            ) => result?,
        };
        let parts = build_http_request(
            &compiled,
            &endpoint,
            &credential,
            self.limits.max_request_body_bytes,
            self.limits.connect_timeout,
            self.limits.stream_idle_timeout,
        )?;
        let started_at = Instant::now();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Ok(ProviderRunOutcome::Cancelled),
            response = tokio::time::timeout(
                self.limits.response_header_timeout,
                parts.client.execute(parts.request),
            ) => response.map_err(|_| timeout_error("RESPONSE_HEADER_TIMEOUT"))?
                .map_err(map_http_error)?,
        };
        let deadline = started_at + self.limits.overall_timeout;

        if !response.status().is_success() {
            let error = read_http_error(
                response,
                request.provider,
                self.limits.max_http_error_body_bytes,
                &cancellation,
                self.limits.stream_idle_timeout,
                deadline,
            )
            .await;
            if error.kind() == RuntimeErrorKind::Cancelled {
                return Ok(ProviderRunOutcome::Cancelled);
            }
            return Err(error);
        }
        validate_content_type(&response, compiled.stream_protocol())?;
        match compiled.stream_protocol() {
            StreamProtocol::Sse => {
                self.consume_sse(response, request.provider, cancellation, events, deadline)
                    .await
            }
            StreamProtocol::Ndjson => {
                self.consume_ndjson(response, request.provider, cancellation, events, deadline)
                    .await
            }
        }
    }

    async fn consume_sse(
        &self,
        response: Response,
        provider: ProviderId,
        cancellation: CancellationToken,
        events: mpsc::Sender<ProviderStreamEvent>,
        deadline: Instant,
    ) -> Result<ProviderRunOutcome> {
        let mut body = response.bytes_stream();
        let mut framer = SseFramer::new(self.limits.max_stream_frame_bytes);
        let mut decoder = ProviderDecoder::new(provider);
        loop {
            let chunk = match next_chunk(
                &mut body,
                &cancellation,
                self.limits.stream_idle_timeout,
                deadline,
            )
            .await
            {
                Ok(chunk) => chunk,
                Err(error) if error.kind() == RuntimeErrorKind::Cancelled => {
                    return Ok(ProviderRunOutcome::Cancelled);
                }
                Err(error) => return Err(error),
            };
            let Some(chunk) = chunk else {
                framer.finish()?;
                return decoder.finish();
            };
            for frame in framer.push(&chunk)? {
                if let Some(outcome) = deliver_decoded(
                    decoder.decode_sse(&frame)?,
                    &events,
                    &cancellation,
                    deadline,
                )
                .await?
                {
                    return Ok(outcome);
                }
            }
        }
    }

    async fn consume_ndjson(
        &self,
        response: Response,
        provider: ProviderId,
        cancellation: CancellationToken,
        events: mpsc::Sender<ProviderStreamEvent>,
        deadline: Instant,
    ) -> Result<ProviderRunOutcome> {
        let mut body = response.bytes_stream();
        let mut framer = NdjsonFramer::new(self.limits.max_stream_frame_bytes);
        let mut decoder = ProviderDecoder::new(provider);
        loop {
            let chunk = match next_chunk(
                &mut body,
                &cancellation,
                self.limits.stream_idle_timeout,
                deadline,
            )
            .await
            {
                Ok(chunk) => chunk,
                Err(error) if error.kind() == RuntimeErrorKind::Cancelled => {
                    return Ok(ProviderRunOutcome::Cancelled);
                }
                Err(error) => return Err(error),
            };
            let Some(chunk) = chunk else {
                for frame in framer.finish()? {
                    if let Some(outcome) = deliver_decoded(
                        decoder.decode_ndjson(&frame)?,
                        &events,
                        &cancellation,
                        deadline,
                    )
                    .await?
                    {
                        return Ok(outcome);
                    }
                }
                return decoder.finish();
            };
            for frame in framer.push(&chunk)? {
                if let Some(outcome) = deliver_decoded(
                    decoder.decode_ndjson(&frame)?,
                    &events,
                    &cancellation,
                    deadline,
                )
                .await?
                {
                    return Ok(outcome);
                }
            }
        }
    }
}

impl Default for ProviderRuntime {
    fn default() -> Self {
        Self::new()
    }
}

async fn deliver_decoded(
    decoded: DecodedFrame,
    events: &mpsc::Sender<ProviderStreamEvent>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Option<ProviderRunOutcome>> {
    for event in decoded.events {
        tokio::select! {
            _ = cancellation.cancelled() => return Ok(Some(ProviderRunOutcome::Cancelled)),
            _ = tokio::time::sleep_until(deadline) => return Err(timeout_error("OVERALL_TIMEOUT")),
            result = events.send(event) => result.map_err(|_| RuntimeError::new(
                RuntimeErrorKind::ConsumerClosed,
                "EVENT_CONSUMER_CLOSED",
                "provider event consumer closed before the stream completed",
            ))?,
        }
    }
    Ok(decoded.terminal)
}

async fn next_chunk<S>(
    body: &mut S,
    cancellation: &CancellationToken,
    idle_timeout: Duration,
    deadline: Instant,
) -> Result<Option<bytes::Bytes>>
where
    S: futures_util::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    tokio::select! {
        _ = cancellation.cancelled() => Err(RuntimeError::new(
            RuntimeErrorKind::Cancelled,
            "CANCELLED",
            "provider request was cancelled",
        )),
        _ = tokio::time::sleep_until(deadline) => Err(timeout_error("OVERALL_TIMEOUT")),
        result = tokio::time::timeout(idle_timeout, body.next()) => {
            result.map_err(|_| timeout_error("STREAM_IDLE_TIMEOUT"))?
                .transpose()
                .map_err(map_http_error)
        }
    }
}

fn validate_content_type(response: &Response, protocol: StreamProtocol) -> Result<()> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let expected = match protocol {
        StreamProtocol::Sse => "text/event-stream",
        StreamProtocol::Ndjson => "application/x-ndjson",
    };
    if content_type == expected {
        Ok(())
    } else {
        Err(RuntimeError::new(
            RuntimeErrorKind::UnexpectedContentType,
            "UNEXPECTED_CONTENT_TYPE",
            format!("provider success response must use {expected}"),
        ))
    }
}

async fn read_http_error(
    response: Response,
    provider: ProviderId,
    maximum: usize,
    cancellation: &CancellationToken,
    idle_timeout: Duration,
    deadline: Instant,
) -> RuntimeError {
    let status = response.status();
    let mut body = response.bytes_stream();
    // The body is deliberately never parsed or reflected to the caller. Some
    // providers/proxies echo request material in error payloads. Zeroize the
    // bounded buffer as an additional native-memory hygiene measure.
    let mut bytes = Zeroizing::new(Vec::new());
    loop {
        let chunk = match next_chunk(&mut body, cancellation, idle_timeout, deadline).await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => return error,
        };
        if bytes.len().saturating_add(chunk.len()) > maximum {
            return RuntimeError::new(
                RuntimeErrorKind::HttpStatus,
                "HTTP_ERROR_BODY_TOO_LARGE",
                "provider returned an oversized HTTP error body",
            )
            .with_http_status(status.as_u16())
            .retriable(is_retriable_status(status));
        }
        bytes.extend_from_slice(&chunk);
    }
    let (code, message) = stable_http_error(provider);
    RuntimeError::new(RuntimeErrorKind::HttpStatus, code, message)
        .with_http_status(status.as_u16())
        .retriable(is_retriable_status(status))
}

fn stable_http_error(provider: ProviderId) -> (&'static str, &'static str) {
    match provider {
        ProviderId::OpenAi => ("OPENAI_HTTP_ERROR", "provider returned an HTTP error"),
        ProviderId::Anthropic => ("ANTHROPIC_HTTP_ERROR", "provider returned an HTTP error"),
        ProviderId::DeepSeek => ("DEEPSEEK_HTTP_ERROR", "provider returned an HTTP error"),
        ProviderId::OllamaCloud => ("OLLAMA_HTTP_ERROR", "provider returned an HTTP error"),
        ProviderId::GoogleGemini | ProviderId::GoogleVertexAi => {
            ("GOOGLE_HTTP_ERROR", "provider returned an HTTP error")
        }
    }
}

fn map_http_error(_error: reqwest::Error) -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::Http,
        "HTTP_TRANSPORT_FAILED",
        "provider HTTP transport failed",
    )
    .retriable(true)
}

fn timeout_error(code: &'static str) -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::Timeout,
        code,
        "provider request exceeded a runtime timeout",
    )
    .retriable(true)
}

fn is_retriable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

fn validate_limits(limits: &RuntimeLimits) -> Result<()> {
    const MAX_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;
    const MAX_STREAM_FRAME_BYTES: usize = 256 * 1024;
    const MAX_HTTP_ERROR_BODY_BYTES: usize = 64 * 1024;
    if !(1024..=MAX_REQUEST_BODY_BYTES).contains(&limits.max_request_body_bytes)
        || limits.max_stream_frame_bytes < 1024
        || limits.max_stream_frame_bytes > MAX_STREAM_FRAME_BYTES
        || limits.max_http_error_body_bytes < 1024
        || limits.max_http_error_body_bytes > MAX_HTTP_ERROR_BODY_BYTES
        || limits.dns_timeout.is_zero()
        || limits.dns_timeout > Duration::from_secs(60)
        || limits.connect_timeout.is_zero()
        || limits.connect_timeout > Duration::from_secs(60)
        || limits.response_header_timeout.is_zero()
        || limits.response_header_timeout > Duration::from_secs(5 * 60)
        || limits.stream_idle_timeout.is_zero()
        || limits.stream_idle_timeout > Duration::from_secs(10 * 60)
        || limits.overall_timeout.is_zero()
        || limits.overall_timeout > Duration::from_secs(30 * 60)
    {
        return Err(RuntimeError::new(
            RuntimeErrorKind::InvalidRequest,
            "INVALID_RUNTIME_LIMITS",
            "runtime byte and duration limits must be positive and bounded",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_cannot_disable_runtime_resource_bounds() {
        let limits = RuntimeLimits {
            max_request_body_bytes: usize::MAX,
            ..RuntimeLimits::default()
        };
        assert_eq!(
            ProviderRuntime::with_limits(limits)
                .expect_err("unbounded body limit must fail")
                .code(),
            "INVALID_RUNTIME_LIMITS"
        );

        let limits = RuntimeLimits {
            max_stream_frame_bytes: 256 * 1024 + 1,
            ..RuntimeLimits::default()
        };
        assert!(ProviderRuntime::with_limits(limits).is_err());

        let limits = RuntimeLimits {
            overall_timeout: Duration::from_secs(30 * 60 + 1),
            ..RuntimeLimits::default()
        };
        assert!(ProviderRuntime::with_limits(limits).is_err());
    }

    #[test]
    fn http_errors_are_stable_and_provider_body_independent() {
        assert_eq!(
            stable_http_error(ProviderId::OpenAi),
            ("OPENAI_HTTP_ERROR", "provider returned an HTTP error")
        );
        assert_eq!(
            stable_http_error(ProviderId::GoogleVertexAi),
            ("GOOGLE_HTTP_ERROR", "provider returned an HTTP error")
        );
    }

    #[tokio::test]
    async fn cancelled_body_wait_ends_as_cancelled() {
        let mut body =
            futures_util::stream::pending::<std::result::Result<bytes::Bytes, reqwest::Error>>();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = next_chunk(
            &mut body,
            &cancellation,
            Duration::from_secs(1),
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .expect_err("cancelled stream must stop");
        assert_eq!(error.kind(), RuntimeErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn pending_body_wait_obeys_idle_timeout() {
        let mut body =
            futures_util::stream::pending::<std::result::Result<bytes::Bytes, reqwest::Error>>();
        let error = next_chunk(
            &mut body,
            &CancellationToken::new(),
            Duration::from_millis(1),
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .expect_err("idle stream must time out");
        assert_eq!(error.kind(), RuntimeErrorKind::Timeout);
        assert_eq!(error.code(), "STREAM_IDLE_TIMEOUT");
    }
}
