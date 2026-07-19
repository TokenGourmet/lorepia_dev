use std::fmt;

pub type Result<T> = std::result::Result<T, ToolRuntimeError>;

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ToolRuntimeError {
    EmptyField(&'static str),
    FieldTooLong {
        field: &'static str,
        max_bytes: usize,
    },
    InvalidName {
        field: &'static str,
    },
    InvalidJson,
    JsonTooLarge {
        field: &'static str,
        max_bytes: usize,
    },
    JsonTooDeep {
        field: &'static str,
        max_depth: usize,
    },
    JsonTooManyNodes {
        field: &'static str,
        max_nodes: usize,
    },
    JsonMustBeObject {
        field: &'static str,
    },
    InvalidInputSchema(&'static str),
    InvalidToolArguments(&'static str),
    PolicyDenied {
        tool_name: String,
    },
    ApprovalRequired,
    InvalidApprovalLifetime,
    InvalidApprovalToken,
    ApprovalExpired,
    ApprovalReplay,
    ApprovalLedgerFull,
    InternalState,
    ToolNotRegistered {
        tool_name: String,
    },
    DuplicateTool {
        tool_name: String,
    },
    TooManyTools {
        max: usize,
    },
    InvalidExecutorCode,
    ExecutorFailed {
        code: String,
    },
    UnsupportedMcpTransport,
    InvalidMcpEndpoint(&'static str),
    ForbiddenMcpHost,
    InvalidCredentialReference,
}

impl fmt::Display for ToolRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "field must not be empty: {field}"),
            Self::FieldTooLong { field, max_bytes } => {
                write!(formatter, "field exceeds {max_bytes} bytes: {field}")
            }
            Self::InvalidName { field } => write!(formatter, "invalid identifier: {field}"),
            Self::InvalidJson => formatter.write_str("invalid JSON document"),
            Self::JsonTooLarge { field, max_bytes } => {
                write!(formatter, "JSON field exceeds {max_bytes} bytes: {field}")
            }
            Self::JsonTooDeep { field, max_depth } => {
                write!(formatter, "JSON field exceeds depth {max_depth}: {field}")
            }
            Self::JsonTooManyNodes { field, max_nodes } => {
                write!(formatter, "JSON field exceeds {max_nodes} nodes: {field}")
            }
            Self::JsonMustBeObject { field } => {
                write!(formatter, "JSON field must be an object: {field}")
            }
            Self::InvalidInputSchema(reason) => write!(formatter, "invalid input schema: {reason}"),
            Self::InvalidToolArguments(reason) => {
                write!(
                    formatter,
                    "tool arguments do not match the input schema: {reason}"
                )
            }
            Self::PolicyDenied { tool_name } => {
                write!(formatter, "tool is not allowlisted: {tool_name}")
            }
            Self::ApprovalRequired => formatter.write_str("a per-call approval token is required"),
            Self::InvalidApprovalLifetime => formatter.write_str("invalid approval lifetime"),
            Self::InvalidApprovalToken => formatter.write_str("invalid approval token"),
            Self::ApprovalExpired => formatter.write_str("approval token has expired"),
            Self::ApprovalReplay => formatter.write_str("approval token was already consumed"),
            Self::ApprovalLedgerFull => formatter.write_str("approval replay ledger is full"),
            Self::InternalState => formatter.write_str("tool runtime internal state unavailable"),
            Self::ToolNotRegistered { tool_name } => {
                write!(formatter, "tool is not registered: {tool_name}")
            }
            Self::DuplicateTool { tool_name } => {
                write!(formatter, "tool is already registered: {tool_name}")
            }
            Self::TooManyTools { max } => write!(formatter, "tool registry exceeds {max} entries"),
            Self::InvalidExecutorCode => formatter.write_str("invalid executor error code"),
            Self::ExecutorFailed { code } => write!(formatter, "tool executor failed: {code}"),
            Self::UnsupportedMcpTransport => {
                formatter.write_str("only remote MCP Streamable HTTP is allowed")
            }
            Self::InvalidMcpEndpoint(reason) => write!(formatter, "invalid MCP endpoint: {reason}"),
            Self::ForbiddenMcpHost => formatter.write_str("MCP endpoint host is forbidden"),
            Self::InvalidCredentialReference => formatter.write_str("invalid credential reference"),
        }
    }
}

impl std::error::Error for ToolRuntimeError {}
