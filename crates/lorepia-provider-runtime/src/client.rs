use std::time::Duration;

use lorepia_providers::{AuthScheme, CompiledProviderRequest, ProviderId, StreamProtocol};
use reqwest::{
    Client, ClientBuilder, Request,
    header::{ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue},
};
use zeroize::Zeroizing;

use crate::{
    CredentialScope, ProviderCredential, Result, RuntimeError, RuntimeErrorKind,
    endpoint::ResolvedEndpoint,
};

pub(crate) struct HttpRequestParts {
    pub(crate) client: Client,
    pub(crate) request: Request,
}

pub(crate) fn build_http_request(
    compiled: &CompiledProviderRequest,
    endpoint: &ResolvedEndpoint,
    credential: &ProviderCredential,
    max_request_body_bytes: usize,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Result<HttpRequestParts> {
    validate_credential_scope(compiled.provider(), endpoint, credential.scope())?;
    let body = serde_json::to_vec(compiled.body()).map_err(|_| {
        RuntimeError::new(
            RuntimeErrorKind::InvalidRequest,
            "REQUEST_SERIALIZATION_FAILED",
            "provider request body could not be serialized",
        )
    })?;
    if body.len() > max_request_body_bytes {
        return Err(RuntimeError::new(
            RuntimeErrorKind::InvalidRequest,
            "REQUEST_BODY_TOO_LARGE",
            "provider request body exceeded the runtime byte limit",
        ));
    }

    let client = build_secure_client(endpoint, connect_timeout, read_timeout)?;

    let mut headers = HeaderMap::new();
    for (name, value) in compiled.static_headers() {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidRequest,
                "INVALID_STATIC_HEADER",
                "compiled request contained an invalid static header name",
            )
        })?;
        let value = HeaderValue::from_str(value).map_err(|_| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidRequest,
                "INVALID_STATIC_HEADER",
                "compiled request contained an invalid static header value",
            )
        })?;
        headers.insert(name, value);
    }
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(match compiled.stream_protocol() {
            StreamProtocol::Sse => "text/event-stream",
            StreamProtocol::Ndjson => "application/x-ndjson",
        }),
    );
    // Compression is intentionally unsupported for streaming responses. This
    // avoids a decompression-allocation boundary before LorePia's frame limit.
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    attach_credential(&mut headers, compiled.auth_scheme(), credential.secret())?;

    let request = client
        .post(endpoint.url.clone())
        .headers(headers)
        .body(body)
        .build()
        .map_err(|_| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidRequest,
                "HTTP_REQUEST_BUILD_FAILED",
                "provider HTTP request could not be built",
            )
        })?;
    Ok(HttpRequestParts { client, request })
}

pub(crate) fn build_secure_client(
    endpoint: &ResolvedEndpoint,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Result<Client> {
    let mut builder = bounded_client_builder(connect_timeout, read_timeout).https_only(true);
    builder = builder.resolve_to_addrs(&endpoint.host, &endpoint.pinned_addresses);
    builder.build().map_err(|_| {
        RuntimeError::new(
            RuntimeErrorKind::Http,
            "HTTP_CLIENT_BUILD_FAILED",
            "secure HTTP client could not be initialized",
        )
    })
}

fn bounded_client_builder(connect_timeout: Duration, read_timeout: Duration) -> ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // reqwest otherwise retries a small class of protocol failures. A
        // streaming POST must never be replayed below the product's explicit
        // request lifecycle.
        .retry(reqwest::retry::never())
        .no_proxy()
        .referer(false)
        .connect_timeout(connect_timeout)
        .read_timeout(read_timeout)
        .pool_max_idle_per_host(0)
        .http2_max_header_list_size(16 * 1024)
        .user_agent("LorePia/0.1")
}

#[cfg(test)]
pub(crate) fn build_loopback_test_client(
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Client {
    bounded_client_builder(connect_timeout, read_timeout)
        .build()
        .expect("bounded loopback test client")
}

pub(crate) fn validate_credential_scope(
    provider: ProviderId,
    endpoint: &ResolvedEndpoint,
    scope: &CredentialScope,
) -> Result<()> {
    let accepted = match (endpoint.is_override, scope) {
        (false, CredentialScope::OfficialProvider(scoped)) => *scoped == provider,
        (true, CredentialScope::OverrideHost(host)) => host.eq_ignore_ascii_case(&endpoint.host),
        _ => false,
    };
    if accepted {
        Ok(())
    } else {
        Err(RuntimeError::new(
            RuntimeErrorKind::CredentialMismatch,
            "CREDENTIAL_SCOPE_MISMATCH",
            "credential scope did not match the selected provider endpoint",
        ))
    }
}

pub(crate) fn attach_credential(
    headers: &mut HeaderMap,
    scheme: AuthScheme,
    secret: &str,
) -> Result<()> {
    let (name, mut value) = match scheme {
        AuthScheme::AuthorizationBearer | AuthScheme::GoogleOAuthBearer => {
            let bearer = Zeroizing::new(format!("Bearer {secret}"));
            let value = HeaderValue::from_str(bearer.as_str()).map_err(|_| invalid_credential())?;
            (AUTHORIZATION, value)
        }
        AuthScheme::AnthropicXApiKey => (
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_str(secret).map_err(|_| invalid_credential())?,
        ),
        AuthScheme::GoogleXGoogApiKey => (
            HeaderName::from_static("x-goog-api-key"),
            HeaderValue::from_str(secret).map_err(|_| invalid_credential())?,
        ),
    };
    value.set_sensitive(true);
    headers.insert(name, value);
    Ok(())
}

fn invalid_credential() -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::InvalidCredential,
        "INVALID_CREDENTIAL",
        "credential could not be encoded as an HTTP header",
    )
}
