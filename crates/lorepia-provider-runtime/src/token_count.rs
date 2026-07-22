use std::{future::Future, pin::Pin};

use futures_util::StreamExt;
use lorepia_prompt::{
    ExactTokenCountError, ExactTokenCountErrorKind, ExactTokenCounter, ExactTokenInput,
    ExactTokenResult,
};
use lorepia_providers::{CompiledProviderRequest, ProviderId, ProviderOptions, ProviderRequest};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    EndpointSelection, ProviderCredential, RuntimeError, RuntimeErrorKind, RuntimeLimits,
    client::{attach_credential, build_secure_client, validate_credential_scope},
    endpoint::resolve_endpoint,
    runner::validate_response_metadata,
};

const JSON_CONTENT_TYPE: &str = "application/json";

pub(crate) struct ProviderExactTokenCounter<'a> {
    limits: RuntimeLimits,
    endpoint_selection: &'a EndpointSelection,
    credential: &'a ProviderCredential,
    cancellation: &'a CancellationToken,
}

impl<'a> ProviderExactTokenCounter<'a> {
    pub(crate) const fn new(
        limits: RuntimeLimits,
        endpoint_selection: &'a EndpointSelection,
        credential: &'a ProviderCredential,
        cancellation: &'a CancellationToken,
    ) -> Self {
        Self {
            limits,
            endpoint_selection,
            credential,
            cancellation,
        }
    }

    async fn count(&self, input: ExactTokenInput<'_>) -> ExactTokenResult<usize> {
        if !matches!(self.endpoint_selection, EndpointSelection::Official) {
            return Err(unavailable(
                "EXACT_TOKEN_OVERRIDE_UNAVAILABLE",
                "custom endpoints have no reviewed exact-token preflight contract",
            ));
        }

        tokio::select! {
            _ = self.cancellation.cancelled() => Err(cancelled()),
            result = tokio::time::timeout(
                self.limits.token_count_timeout,
                self.count_inner(input),
            ) => result.map_err(|_| ExactTokenCountError::new(
                ExactTokenCountErrorKind::Timeout,
                "EXACT_TOKEN_COUNT_TIMEOUT",
                "provider token counting exceeded the runtime timeout",
            ).retriable(true))?,
        }
    }

    async fn count_inner(&self, input: ExactTokenInput<'_>) -> ExactTokenResult<usize> {
        let request = input.request();
        let compiled = input.compiled();
        let spec = build_count_spec(request, compiled)?;

        let mut endpoint = resolve_endpoint(
            request,
            compiled,
            self.endpoint_selection,
            self.limits.dns_timeout,
        )
        .await
        .map_err(map_runtime_error)?;
        endpoint.url.set_path(&spec.path);
        endpoint.url.set_query(None);

        validate_credential_scope(request.provider, &endpoint, self.credential.scope())
            .map_err(map_runtime_error)?;
        let body = serde_json::to_vec(&spec.body).map_err(|_| {
            invalid_response(
                "EXACT_TOKEN_REQUEST_SERIALIZATION_FAILED",
                "token counting request could not be serialized",
            )
        })?;
        if body.len() > self.limits.max_request_body_bytes {
            return Err(ExactTokenCountError::new(
                ExactTokenCountErrorKind::InvalidRequest,
                "EXACT_TOKEN_REQUEST_TOO_LARGE",
                "token counting request exceeded the runtime byte limit",
            ));
        }

        let client = build_secure_client(
            &endpoint,
            self.limits.connect_timeout,
            self.limits.token_count_timeout,
        )
        .map_err(map_runtime_error)?;
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static(JSON_CONTENT_TYPE));
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
        for (name, value) in compiled.static_headers() {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                invalid_response(
                    "INVALID_STATIC_HEADER",
                    "compiled request contained an invalid static header name",
                )
            })?;
            let value = HeaderValue::from_str(value).map_err(|_| {
                invalid_response(
                    "INVALID_STATIC_HEADER",
                    "compiled request contained an invalid static header value",
                )
            })?;
            headers.insert(name, value);
        }
        attach_credential(
            &mut headers,
            compiled.auth_scheme(),
            self.credential.secret(),
        )
        .map_err(map_runtime_error)?;

        let http_request = client
            .post(endpoint.url)
            .headers(headers)
            .body(body)
            .build()
            .map_err(|_| {
                invalid_response(
                    "EXACT_TOKEN_HTTP_REQUEST_BUILD_FAILED",
                    "token counting HTTP request could not be built",
                )
            })?;
        let response = client.execute(http_request).await.map_err(|_| {
            ExactTokenCountError::new(
                ExactTokenCountErrorKind::Transport,
                "EXACT_TOKEN_TRANSPORT_FAILED",
                "provider token counting transport failed",
            )
            .retriable(true)
        })?;
        validate_response_metadata(
            &response,
            self.limits.max_response_header_count,
            self.limits.max_response_header_bytes,
        )
        .map_err(map_runtime_error)?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(ExactTokenCountError::new(
                ExactTokenCountErrorKind::HttpStatus,
                "EXACT_TOKEN_HTTP_ERROR",
                format!("provider token counting returned HTTP {}", status.as_u16()),
            )
            .with_http_status(status.as_u16())
            .retriable(is_retriable_status(status.as_u16())));
        }
        validate_json_content_type(&response)?;
        let bytes = read_bounded_json(
            response,
            self.limits.max_token_count_response_bytes,
            self.cancellation,
        )
        .await?;
        parse_count_response(spec.response_kind, &bytes)
    }
}

impl ExactTokenCounter for ProviderExactTokenCounter<'_> {
    fn count_input_tokens<'a>(
        &'a self,
        input: ExactTokenInput<'a>,
    ) -> Pin<Box<dyn Future<Output = ExactTokenResult<usize>> + Send + 'a>> {
        Box::pin(async move { self.count(input).await })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CountResponseKind {
    OpenAi,
    Google,
}

#[derive(Debug, PartialEq)]
struct CountSpec {
    path: String,
    body: Value,
    response_kind: CountResponseKind,
}

fn build_count_spec(
    request: &ProviderRequest,
    compiled: &CompiledProviderRequest,
) -> ExactTokenResult<CountSpec> {
    if request.provider != compiled.provider() {
        return Err(invalid_response(
            "EXACT_TOKEN_PROVIDER_MISMATCH",
            "logical and compiled provider requests did not match",
        ));
    }

    match request.provider {
        ProviderId::OpenAi => Ok(CountSpec {
            path: "/v1/responses/input_tokens".to_owned(),
            body: openai_count_body(compiled.body())?,
            response_kind: CountResponseKind::OpenAi,
        }),
        ProviderId::GoogleGemini => {
            let mut generate_request = compiled.body().as_object().cloned().ok_or_else(|| {
                invalid_response(
                    "EXACT_TOKEN_WIRE_SHAPE_INVALID",
                    "Gemini compiled request body was not an object",
                )
            })?;
            generate_request.insert(
                "model".to_owned(),
                Value::String(format!("models/{}", request.model_id)),
            );
            Ok(CountSpec {
                path: format!("/v1beta/models/{}:countTokens", request.model_id),
                body: Value::Object(Map::from_iter([(
                    "generateContentRequest".to_owned(),
                    Value::Object(generate_request),
                )])),
                response_kind: CountResponseKind::Google,
            })
        }
        ProviderId::GoogleVertexAi => vertex_count_spec(request, compiled),
        ProviderId::Anthropic => Err(unavailable(
            "EXACT_TOKEN_COUNT_IS_ESTIMATE",
            "Anthropic documents its token counting result as an estimate",
        )),
        ProviderId::DeepSeek => Err(unavailable(
            "EXACT_TOKEN_PREFLIGHT_UNAVAILABLE",
            "DeepSeek exposes post-generation usage and an offline estimate, not an authoritative preflight",
        )),
        ProviderId::OllamaCloud => Err(unavailable(
            "EXACT_TOKEN_PREFLIGHT_UNAVAILABLE",
            "Ollama Cloud exposes post-generation usage, not an authoritative preflight",
        )),
    }
}

fn openai_count_body(body: &Value) -> ExactTokenResult<Value> {
    const INCLUDED: &[&str] = &[
        "model",
        "input",
        "instructions",
        "parallel_tool_calls",
        "personality",
        "previous_response_id",
        "reasoning",
        "text",
        "tool_choice",
        "tools",
        "truncation",
    ];
    const NON_INPUT: &[&str] = &[
        "stream",
        "store",
        "temperature",
        "max_output_tokens",
        "top_p",
        "prompt_cache_key",
        "prompt_cache_retention",
        "service_tier",
    ];

    let object = body.as_object().ok_or_else(|| {
        invalid_response(
            "EXACT_TOKEN_WIRE_SHAPE_INVALID",
            "OpenAI compiled request body was not an object",
        )
    })?;
    let mut count = Map::new();
    for (key, value) in object {
        if INCLUDED.contains(&key.as_str()) {
            count.insert(key.clone(), value.clone());
        } else if !NON_INPUT.contains(&key.as_str()) {
            return Err(unavailable(
                "EXACT_TOKEN_FIELD_UNMAPPED",
                &format!("OpenAI token counting has no reviewed mapping for field {key}"),
            ));
        }
    }
    if !count.contains_key("model") || !count.contains_key("input") {
        return Err(invalid_response(
            "EXACT_TOKEN_WIRE_SHAPE_INVALID",
            "OpenAI compiled request lacked model or input",
        ));
    }
    Ok(Value::Object(count))
}

fn vertex_count_spec(
    request: &ProviderRequest,
    compiled: &CompiledProviderRequest,
) -> ExactTokenResult<CountSpec> {
    const INCLUDED: &[&str] = &["contents", "tools", "systemInstruction", "generationConfig"];
    const NON_INPUT: &[&str] = &["safetySettings", "serviceTier", "store", "toolConfig"];

    let ProviderOptions::GoogleVertexAi(options) = &request.provider_options else {
        return Err(invalid_response(
            "EXACT_TOKEN_PROVIDER_OPTIONS_MISMATCH",
            "Vertex request did not contain Vertex provider options",
        ));
    };
    let object = compiled.body().as_object().ok_or_else(|| {
        invalid_response(
            "EXACT_TOKEN_WIRE_SHAPE_INVALID",
            "Vertex compiled request body was not an object",
        )
    })?;
    if object.contains_key("cachedContent") {
        return Err(unavailable(
            "EXACT_TOKEN_CACHED_CONTENT_UNAVAILABLE",
            "the reviewed Vertex token-count contract cannot account for cached content",
        ));
    }

    let resource = format!(
        "projects/{}/locations/{}/publishers/google/models/{}",
        options.project_id, options.location, request.model_id
    );
    let mut body = Map::from_iter([("model".to_owned(), Value::String(resource.clone()))]);
    for (key, value) in object {
        if INCLUDED.contains(&key.as_str()) {
            body.insert(key.clone(), value.clone());
        } else if !NON_INPUT.contains(&key.as_str()) {
            return Err(unavailable(
                "EXACT_TOKEN_FIELD_UNMAPPED",
                &format!("Vertex token counting has no reviewed mapping for field {key}"),
            ));
        }
    }
    if !body.contains_key("contents") {
        return Err(invalid_response(
            "EXACT_TOKEN_WIRE_SHAPE_INVALID",
            "Vertex compiled request lacked contents",
        ));
    }

    Ok(CountSpec {
        path: format!("/v1beta1/{resource}:countTokens"),
        body: Value::Object(body),
        response_kind: CountResponseKind::Google,
    })
}

async fn read_bounded_json(
    response: reqwest::Response,
    maximum: usize,
    cancellation: &CancellationToken,
) -> ExactTokenResult<Vec<u8>> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            return Ok(bytes);
        };
        let chunk = chunk.map_err(|_| {
            ExactTokenCountError::new(
                ExactTokenCountErrorKind::Transport,
                "EXACT_TOKEN_RESPONSE_READ_FAILED",
                "provider token counting response could not be read",
            )
            .retriable(true)
        })?;
        if bytes.len().saturating_add(chunk.len()) > maximum {
            return Err(invalid_response(
                "EXACT_TOKEN_RESPONSE_TOO_LARGE",
                "provider token counting response exceeded the runtime byte limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
}

fn validate_json_content_type(response: &reqwest::Response) -> ExactTokenResult<()> {
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
    if content_type == JSON_CONTENT_TYPE {
        Ok(())
    } else {
        Err(invalid_response(
            "EXACT_TOKEN_CONTENT_TYPE_INVALID",
            "provider token counting success response was not JSON",
        ))
    }
}

#[derive(Deserialize)]
struct OpenAiCountResponse {
    object: String,
    input_tokens: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCountResponse {
    total_tokens: u64,
}

fn parse_count_response(kind: CountResponseKind, bytes: &[u8]) -> ExactTokenResult<usize> {
    let count = match kind {
        CountResponseKind::OpenAi => {
            let response: OpenAiCountResponse = serde_json::from_slice(bytes).map_err(|_| {
                invalid_response(
                    "EXACT_TOKEN_RESPONSE_INVALID",
                    "OpenAI token counting response did not match the reviewed schema",
                )
            })?;
            if response.object != "response.input_tokens" {
                return Err(invalid_response(
                    "EXACT_TOKEN_RESPONSE_INVALID",
                    "OpenAI token counting response had an unexpected object type",
                ));
            }
            response.input_tokens
        }
        CountResponseKind::Google => {
            let response: GoogleCountResponse = serde_json::from_slice(bytes).map_err(|_| {
                invalid_response(
                    "EXACT_TOKEN_RESPONSE_INVALID",
                    "Google token counting response did not match the reviewed schema",
                )
            })?;
            response.total_tokens
        }
    };
    usize::try_from(count).map_err(|_| {
        invalid_response(
            "EXACT_TOKEN_COUNT_OVERFLOW",
            "provider token count did not fit the local platform integer",
        )
    })
}

fn map_runtime_error(error: RuntimeError) -> ExactTokenCountError {
    let kind = match error.kind() {
        RuntimeErrorKind::Cancelled => ExactTokenCountErrorKind::Cancelled,
        RuntimeErrorKind::Timeout => ExactTokenCountErrorKind::Timeout,
        RuntimeErrorKind::HttpStatus => ExactTokenCountErrorKind::HttpStatus,
        RuntimeErrorKind::Http
        | RuntimeErrorKind::DnsResolution
        | RuntimeErrorKind::UnsafeEndpoint => ExactTokenCountErrorKind::Transport,
        RuntimeErrorKind::InvalidRequest
        | RuntimeErrorKind::InvalidEndpoint
        | RuntimeErrorKind::CredentialMismatch
        | RuntimeErrorKind::InvalidCredential => ExactTokenCountErrorKind::InvalidRequest,
        RuntimeErrorKind::UnexpectedContentType
        | RuntimeErrorKind::StreamTooLarge
        | RuntimeErrorKind::StreamProtocol
        | RuntimeErrorKind::Provider
        | RuntimeErrorKind::ConsumerClosed => ExactTokenCountErrorKind::InvalidResponse,
    };
    let mut mapped = ExactTokenCountError::new(kind, error.code(), error.message())
        .retriable(error.is_retriable());
    if let Some(status) = error.http_status() {
        mapped = mapped.with_http_status(status);
    }
    mapped
}

fn unavailable(code: &str, message: &str) -> ExactTokenCountError {
    ExactTokenCountError::new(ExactTokenCountErrorKind::Unavailable, code, message)
}

fn invalid_response(code: &str, message: &str) -> ExactTokenCountError {
    ExactTokenCountError::new(ExactTokenCountErrorKind::InvalidResponse, code, message)
}

fn cancelled() -> ExactTokenCountError {
    ExactTokenCountError::new(
        ExactTokenCountErrorKind::Cancelled,
        "EXACT_TOKEN_COUNT_CANCELLED",
        "provider token counting was cancelled",
    )
}

const fn is_retriable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lorepia_providers::{
        ChatMessage, GenerationOptions, GoogleOptions, MessageRole, OpenAiOptions, VertexAiOptions,
        VertexRequestType, compile_request,
    };
    use serde_json::json;

    use super::*;

    fn request(provider_options: ProviderOptions, model_id: &str) -> ProviderRequest {
        ProviderRequest {
            provider: provider_options.provider_id(),
            model_id: model_id.to_owned(),
            messages: vec![ChatMessage::new(MessageRole::User, "hello")],
            generation: GenerationOptions {
                max_output_tokens: Some(128),
                ..GenerationOptions::default()
            },
            provider_options,
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
        }
    }

    #[test]
    fn openai_count_uses_only_reviewed_input_fields() {
        let mut request = request(
            ProviderOptions::OpenAi(OpenAiOptions::default()),
            "gpt-test",
        );
        request.generation.temperature = Some(0.7);
        let compiled = compile_request(&request).unwrap();
        let spec = build_count_spec(&request, &compiled).unwrap();

        assert_eq!(spec.path, "/v1/responses/input_tokens");
        assert_eq!(spec.body["model"], "gpt-test");
        assert!(spec.body.get("input").is_some());
        assert!(spec.body.get("stream").is_none());
        assert!(spec.body.get("store").is_none());
        assert!(spec.body.get("temperature").is_none());
        assert!(spec.body.get("max_output_tokens").is_none());
    }

    #[test]
    fn gemini_count_wraps_the_exact_compiled_generate_request() {
        let request = request(
            ProviderOptions::GoogleGemini(GoogleOptions::default()),
            "gemini-test",
        );
        let compiled = compile_request(&request).unwrap();
        let spec = build_count_spec(&request, &compiled).unwrap();

        assert_eq!(spec.path, "/v1beta/models/gemini-test:countTokens");
        assert_eq!(
            spec.body["generateContentRequest"]["contents"],
            compiled.body()["contents"]
        );
        assert_eq!(
            spec.body["generateContentRequest"]["generationConfig"],
            compiled.body()["generationConfig"]
        );
        assert_eq!(
            spec.body["generateContentRequest"]["model"],
            "models/gemini-test"
        );
    }

    #[test]
    fn vertex_count_carries_model_and_reviewed_input_shape() {
        let request = request(
            ProviderOptions::GoogleVertexAi(VertexAiOptions {
                project_id: "project-123".to_owned(),
                location: "us-central1".to_owned(),
                thinking: None,
                cached_content: None,
                safety_settings: Vec::new(),
                request_type: VertexRequestType::Automatic,
            }),
            "gemini-test",
        );
        let compiled = compile_request(&request).unwrap();
        let spec = build_count_spec(&request, &compiled).unwrap();

        assert_eq!(
            spec.path,
            "/v1beta1/projects/project-123/locations/us-central1/publishers/google/models/gemini-test:countTokens"
        );
        assert_eq!(
            spec.body["model"],
            "projects/project-123/locations/us-central1/publishers/google/models/gemini-test"
        );
        assert_eq!(spec.body["contents"], compiled.body()["contents"]);
        assert_eq!(
            spec.body["generationConfig"],
            compiled.body()["generationConfig"]
        );
    }

    #[test]
    fn providers_without_authoritative_preflight_fail_closed() {
        for options in [
            ProviderOptions::Anthropic(Default::default()),
            ProviderOptions::DeepSeek(Default::default()),
            ProviderOptions::OllamaCloud(Default::default()),
        ] {
            let mut request = request(options, "model");
            if request.provider == ProviderId::Anthropic {
                request.generation.max_output_tokens = Some(128);
            }
            let compiled = compile_request(&request).unwrap();
            let error = build_count_spec(&request, &compiled).unwrap_err();
            assert_eq!(error.kind(), ExactTokenCountErrorKind::Unavailable);
        }
    }

    #[test]
    fn response_parsers_require_the_reviewed_count_fields() {
        assert_eq!(
            parse_count_response(
                CountResponseKind::OpenAi,
                br#"{"object":"response.input_tokens","input_tokens":42}"#,
            )
            .unwrap(),
            42
        );
        assert_eq!(
            parse_count_response(CountResponseKind::Google, br#"{"totalTokens":17}"#).unwrap(),
            17
        );
        assert!(
            parse_count_response(
                CountResponseKind::OpenAi,
                &serde_json::to_vec(&json!({
                    "object": "response",
                    "input_tokens": 42
                }))
                .unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn transient_count_http_statuses_are_retriable() {
        for status in [408, 425, 429, 500, 502, 503, 504] {
            assert!(is_retriable_status(status));
        }
        for status in [400, 401, 403, 404, 422] {
            assert!(!is_retriable_status(status));
        }
    }
}
