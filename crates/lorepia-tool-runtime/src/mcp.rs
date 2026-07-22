use serde::{Deserialize, Serialize};
use url::{Host, Url};

use crate::{Result, ToolRuntimeError};

pub const MAX_MCP_SERVER_ID_BYTES: usize = 64;
pub const MAX_MCP_ENDPOINT_BYTES: usize = 2_048;
pub const MAX_CREDENTIAL_REF_BYTES: usize = 128;
const MAX_MCP_CONFIG_DOCUMENT_BYTES: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    StreamableHttp,
    /// Parsed only so the common mobile contract can reject it explicitly.
    /// There are intentionally no command or argument fields in this crate.
    Stdio,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialRef(String);

impl CredentialRef {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let reference = Self(value.into());
        reference.validate()?;
        Ok(reference)
    }

    fn validate(&self) -> Result<()> {
        let value = &self.0;
        if value.is_empty() || value.len() > MAX_CREDENTIAL_REF_BYTES {
            return Err(ToolRuntimeError::InvalidCredentialReference);
        }
        let mut bytes = value.bytes();
        let Some(first) = bytes.next() else {
            return Err(ToolRuntimeError::InvalidCredentialReference);
        };
        if !first.is_ascii_alphabetic()
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
            || value.contains("..")
        {
            return Err(ToolRuntimeError::InvalidCredentialReference);
        }
        Ok(())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteMcpServerConfig {
    server_id: String,
    transport: McpTransport,
    endpoint_url: String,
    credential_ref: CredentialRef,
}

impl RemoteMcpServerConfig {
    pub fn new(
        server_id: impl Into<String>,
        endpoint_url: impl Into<String>,
        credential_ref: CredentialRef,
    ) -> Result<Self> {
        let config = Self {
            server_id: server_id.into(),
            transport: McpTransport::StreamableHttp,
            endpoint_url: endpoint_url.into(),
            credential_ref,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_MCP_CONFIG_DOCUMENT_BYTES {
            return Err(ToolRuntimeError::JsonTooLarge {
                field: "mcp_server_config",
                max_bytes: MAX_MCP_CONFIG_DOCUMENT_BYTES,
            });
        }
        let config: Self =
            serde_json::from_slice(bytes).map_err(|_| ToolRuntimeError::InvalidJson)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        validate_server_id(&self.server_id)?;
        if self.transport != McpTransport::StreamableHttp {
            return Err(ToolRuntimeError::UnsupportedMcpTransport);
        }
        self.credential_ref.validate()?;
        validate_endpoint(&self.endpoint_url)
    }

    #[must_use]
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    #[must_use]
    pub const fn transport(&self) -> McpTransport {
        self.transport
    }

    #[must_use]
    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }

    #[must_use]
    pub fn credential_ref(&self) -> &CredentialRef {
        &self.credential_ref
    }
}

fn validate_server_id(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(ToolRuntimeError::EmptyField("mcp_server_id"));
    }
    if value.len() > MAX_MCP_SERVER_ID_BYTES {
        return Err(ToolRuntimeError::FieldTooLong {
            field: "mcp_server_id",
            max_bytes: MAX_MCP_SERVER_ID_BYTES,
        });
    }
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(ToolRuntimeError::EmptyField("mcp_server_id"));
    };
    if !first.is_ascii_alphabetic()
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        || value.contains("..")
    {
        return Err(ToolRuntimeError::InvalidName {
            field: "mcp_server_id",
        });
    }
    Ok(())
}

fn validate_endpoint(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(ToolRuntimeError::EmptyField("mcp_endpoint"));
    }
    if value.len() > MAX_MCP_ENDPOINT_BYTES {
        return Err(ToolRuntimeError::FieldTooLong {
            field: "mcp_endpoint",
            max_bytes: MAX_MCP_ENDPOINT_BYTES,
        });
    }
    let endpoint = Url::parse(value)
        .map_err(|_| ToolRuntimeError::InvalidMcpEndpoint("URL parsing failed"))?;
    if endpoint.scheme() != "https" {
        return Err(ToolRuntimeError::InvalidMcpEndpoint("HTTPS is required"));
    }
    if endpoint.cannot_be_a_base() {
        return Err(ToolRuntimeError::InvalidMcpEndpoint(
            "hierarchical URL is required",
        ));
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(ToolRuntimeError::InvalidMcpEndpoint(
            "userinfo credentials are forbidden",
        ));
    }
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return Err(ToolRuntimeError::InvalidMcpEndpoint(
            "query and fragment are forbidden",
        ));
    }

    match endpoint.host() {
        Some(Host::Domain(domain)) => validate_public_domain(domain),
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => Err(ToolRuntimeError::ForbiddenMcpHost),
        None => Err(ToolRuntimeError::InvalidMcpEndpoint("host is required")),
    }
}

fn validate_public_domain(domain: &str) -> Result<()> {
    let domain = domain.to_ascii_lowercase();
    if domain.ends_with('.') || domain.len() > 253 || !domain.contains('.') {
        return Err(ToolRuntimeError::ForbiddenMcpHost);
    }
    const FORBIDDEN_SUFFIXES: [&str; 7] = [
        "localhost",
        ".localhost",
        ".local",
        ".lan",
        ".internal",
        ".home",
        ".home.arpa",
    ];
    if FORBIDDEN_SUFFIXES
        .iter()
        .any(|suffix| domain == suffix.trim_start_matches('.') || domain.ends_with(suffix))
    {
        return Err(ToolRuntimeError::ForbiddenMcpHost);
    }
    for label in domain.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(ToolRuntimeError::ForbiddenMcpHost);
        }
    }
    Ok(())
}
