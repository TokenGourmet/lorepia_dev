use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use lorepia_providers::{
    AuthScheme, CompiledProviderRequest, ProviderId, ProviderOptions, ProviderRequest,
    StreamProtocol, VertexRequestType,
};
use serde::{Deserialize, Serialize};
use tokio::net::lookup_host;
use url::{Host, Url};

use crate::{Result, RuntimeError, RuntimeErrorKind};

const MAX_ENDPOINT_BYTES: usize = 2_048;
const MAX_PATH_BYTES: usize = 1_024;
const MAX_QUERY_BYTES: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EndpointSelection {
    Official,
    Override { endpoint: OverrideEndpoint },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OverrideEndpoint(String);

impl OverrideEndpoint {
    pub fn parse(endpoint: impl Into<String>) -> Result<Self> {
        let endpoint = endpoint.into();
        validate_override_url(&endpoint)?;
        Ok(Self(endpoint))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for OverrideEndpoint {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let endpoint = String::deserialize(deserializer)?;
        Self::parse(endpoint).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedEndpoint {
    pub(crate) url: Url,
    pub(crate) host: String,
    pub(crate) pinned_addresses: Vec<SocketAddr>,
    pub(crate) is_override: bool,
}

pub(crate) async fn resolve_endpoint(
    request: &ProviderRequest,
    compiled: &CompiledProviderRequest,
    selection: &EndpointSelection,
    dns_timeout: Duration,
) -> Result<ResolvedEndpoint> {
    validate_compiled_wire_contract(request, compiled)?;

    let (url, is_override) = match selection {
        EndpointSelection::Official => (official_url(request, compiled)?, false),
        EndpointSelection::Override { endpoint } => {
            (validate_override_url(endpoint.as_str())?, true)
        }
    };
    let host = url
        .host_str()
        .ok_or_else(|| invalid_endpoint("endpoint must contain a DNS host"))?
        .to_ascii_lowercase();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| invalid_endpoint("endpoint must use the HTTPS default port"))?;

    let resolved = resolve_addresses(lookup_host((host.as_str(), port)), dns_timeout).await?;

    let mut addresses = BTreeSet::new();
    for address in resolved {
        if !is_public_ip(address.ip()) {
            return Err(RuntimeError::new(
                RuntimeErrorKind::UnsafeEndpoint,
                "NON_PUBLIC_DNS_RESULT",
                "every endpoint DNS result must be a globally routable public address",
            ));
        }
        addresses.insert(SocketAddr::new(address.ip(), port));
    }
    if addresses.is_empty() {
        return Err(RuntimeError::new(
            RuntimeErrorKind::DnsResolution,
            "DNS_NO_ADDRESSES",
            "endpoint DNS resolution returned no addresses",
        ));
    }

    Ok(ResolvedEndpoint {
        url,
        host,
        pinned_addresses: addresses.into_iter().collect(),
        is_override,
    })
}

async fn resolve_addresses<F, I>(future: F, dns_timeout: Duration) -> Result<Vec<SocketAddr>>
where
    F: Future<Output = io::Result<I>>,
    I: IntoIterator<Item = SocketAddr>,
{
    tokio::time::timeout(dns_timeout, future)
        .await
        .map_err(|_| {
            RuntimeError::new(
                RuntimeErrorKind::Timeout,
                "DNS_TIMEOUT",
                "endpoint DNS resolution timed out",
            )
            .retriable(true)
        })?
        .map(|addresses| addresses.into_iter().collect())
        .map_err(|_| {
            RuntimeError::new(
                RuntimeErrorKind::DnsResolution,
                "DNS_RESOLUTION_FAILED",
                "endpoint DNS resolution failed",
            )
            .retriable(true)
        })
}

fn validate_override_url(value: &str) -> Result<Url> {
    if value.is_empty() || value.len() > MAX_ENDPOINT_BYTES {
        return Err(invalid_endpoint(
            "override must be a complete HTTPS endpoint URL of at most 2048 bytes",
        ));
    }
    let url = Url::parse(value)
        .map_err(|_| invalid_endpoint("override must be a valid absolute endpoint URL"))?;
    validate_common_url(&url)?;
    if url.path().is_empty() || url.path() == "/" || url.path().len() > MAX_PATH_BYTES {
        return Err(invalid_endpoint(
            "override must name an exact non-root endpoint path of at most 1024 bytes",
        ));
    }
    if url
        .query()
        .is_some_and(|query| query.len() > MAX_QUERY_BYTES)
    {
        return Err(invalid_endpoint(
            "override endpoint query must be at most 512 bytes",
        ));
    }
    for (key, _) in url.query_pairs() {
        let canonical: String = key
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        if [
            "key",
            "token",
            "secret",
            "password",
            "authorization",
            "apikey",
        ]
        .iter()
        .any(|sensitive| canonical.contains(sensitive))
        {
            return Err(invalid_endpoint(
                "credentials must not be embedded in an override URL query",
            ));
        }
    }
    Ok(url)
}

fn validate_common_url(url: &Url) -> Result<()> {
    if url.scheme() != "https" {
        return Err(invalid_endpoint("endpoint scheme must be https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_endpoint("endpoint userinfo is forbidden"));
    }
    if url.fragment().is_some() {
        return Err(invalid_endpoint("endpoint fragments are forbidden"));
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err(invalid_endpoint(
            "endpoint ports other than 443 are forbidden",
        ));
    }
    let Some(host) = url.host() else {
        return Err(invalid_endpoint("endpoint must contain a host"));
    };
    let Host::Domain(domain) = host else {
        return Err(invalid_endpoint("IP-literal endpoints are forbidden"));
    };
    if domain.ends_with('.') {
        return Err(invalid_endpoint(
            "endpoint host must use its canonical form without a trailing dot",
        ));
    }
    let domain = domain.to_ascii_lowercase();
    if domain.is_empty()
        || domain == "localhost"
        || domain.ends_with(".localhost")
        || domain.ends_with(".local")
        || domain.ends_with(".internal")
        || domain.ends_with(".home.arpa")
    {
        return Err(invalid_endpoint("local endpoint host names are forbidden"));
    }
    Ok(())
}

fn official_url(request: &ProviderRequest, compiled: &CompiledProviderRequest) -> Result<Url> {
    let expected = match request.provider {
        ProviderId::OpenAi => "https://api.openai.com/v1/responses".to_owned(),
        ProviderId::Anthropic => "https://api.anthropic.com/v1/messages".to_owned(),
        ProviderId::DeepSeek => "https://api.deepseek.com/chat/completions".to_owned(),
        ProviderId::OllamaCloud => "https://ollama.com/api/chat".to_owned(),
        ProviderId::GoogleGemini => format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
            request.model_id
        ),
        ProviderId::GoogleVertexAi => {
            let ProviderOptions::GoogleVertexAi(options) = &request.provider_options else {
                return Err(invalid_compiled("Vertex options changed after compilation"));
            };
            let origin = if options.location == "global" {
                "https://aiplatform.googleapis.com".to_owned()
            } else {
                format!("https://{}-aiplatform.googleapis.com", options.location)
            };
            format!(
                "{origin}/v1/projects/{}/locations/{}/publishers/google/models/{}:streamGenerateContent?alt=sse",
                options.project_id, options.location, request.model_id
            )
        }
    };
    let compiled_url = format!("{}{}", compiled.origin(), compiled.path_and_query());
    if compiled_url != expected {
        return Err(invalid_compiled(
            "compiled official endpoint did not match the provider allowlist",
        ));
    }
    let url = Url::parse(&expected)
        .map_err(|_| invalid_compiled("compiled official endpoint was not a valid URL"))?;
    validate_common_url(&url)?;
    Ok(url)
}

fn validate_compiled_wire_contract(
    request: &ProviderRequest,
    compiled: &CompiledProviderRequest,
) -> Result<()> {
    if compiled.provider() != request.provider {
        return Err(invalid_compiled("compiled provider identity mismatch"));
    }
    let (expected_auth, expected_protocol) = match request.provider {
        ProviderId::OpenAi | ProviderId::DeepSeek | ProviderId::OllamaCloud => (
            AuthScheme::AuthorizationBearer,
            if request.provider == ProviderId::OllamaCloud {
                StreamProtocol::Ndjson
            } else {
                StreamProtocol::Sse
            },
        ),
        ProviderId::Anthropic => (AuthScheme::AnthropicXApiKey, StreamProtocol::Sse),
        ProviderId::GoogleGemini => (AuthScheme::GoogleXGoogApiKey, StreamProtocol::Sse),
        ProviderId::GoogleVertexAi => (AuthScheme::GoogleOAuthBearer, StreamProtocol::Sse),
    };
    if compiled.auth_scheme() != expected_auth || compiled.stream_protocol() != expected_protocol {
        return Err(invalid_compiled(
            "compiled authentication or stream protocol mismatch",
        ));
    }

    let mut expected_headers =
        BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);
    if request.provider == ProviderId::Anthropic {
        expected_headers.insert("anthropic-version".to_owned(), "2023-06-01".to_owned());
    }
    if let ProviderOptions::GoogleVertexAi(options) = &request.provider_options {
        let request_type = match options.request_type {
            VertexRequestType::Automatic => None,
            VertexRequestType::Shared => Some("shared"),
            VertexRequestType::Dedicated => Some("dedicated"),
        };
        if let Some(request_type) = request_type {
            expected_headers.insert(
                "X-Vertex-AI-LLM-Request-Type".to_owned(),
                request_type.to_owned(),
            );
        }
    }
    if compiled.static_headers() != &expected_headers {
        return Err(invalid_compiled(
            "compiled static headers did not match the provider allowlist",
        ));
    }
    Ok(())
}

fn invalid_endpoint(message: &'static str) -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::InvalidEndpoint,
        "INVALID_ENDPOINT",
        message,
    )
}

fn invalid_compiled(message: &'static str) -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::InvalidRequest,
        "INVALID_COMPILED_REQUEST",
        message,
    )
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [a, b, c, _] = address.octets();
            !(a == 0
                || a == 10
                || a == 127
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 192 && b == 88 && c == 99)
                || (a == 192 && b == 168)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || a >= 224)
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let segments = address.segments();
            let globally_routed_prefix = (segments[0] & 0xe000) == 0x2000;
            let documentation = segments[0] == 0x2001 && segments[1] == 0x0db8;
            let benchmarking = segments[0] == 0x2001 && segments[1] == 0x0002 && segments[2] == 0;
            globally_routed_prefix && !documentation && !benchmarking
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_requires_an_exact_public_https_style_url() {
        for value in [
            "http://example.com/v1/chat",
            "https://127.0.0.1/v1/chat",
            "https://user@example.com/v1/chat",
            "https://example.com:8443/v1/chat",
            "https://example.com/#fragment",
            "https://localhost/v1/chat",
            "https://example.com/v1/chat?api_key=secret",
        ] {
            assert!(OverrideEndpoint::parse(value).is_err(), "accepted {value}");
        }
        assert!(OverrideEndpoint::parse("https://llm.example.com/v1/chat").is_ok());
    }

    #[test]
    fn non_public_address_classes_are_rejected() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.1.1",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "ff02::1",
        ] {
            assert!(
                !is_public_ip(address.parse().expect("fixture IP")),
                "{address}"
            );
        }
        assert!(is_public_ip("8.8.8.8".parse().expect("public IP")));
        assert!(is_public_ip(
            "2606:4700:4700::1111".parse().expect("public IPv6")
        ));
    }

    #[tokio::test]
    async fn dns_deadline_is_independent_of_the_system_resolver() {
        let pending = std::future::pending::<io::Result<Vec<SocketAddr>>>();
        let error = resolve_addresses(pending, Duration::from_millis(1))
            .await
            .expect_err("pending DNS must hit the configured deadline");
        assert_eq!(error.kind(), RuntimeErrorKind::Timeout);
        assert_eq!(error.code(), "DNS_TIMEOUT");
    }
}
