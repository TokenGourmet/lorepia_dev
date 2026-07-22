#![forbid(unsafe_code)]

//! Deny-by-default contracts for model-requested tools.
//!
//! This crate owns validation, authorization, and dispatch to app-supplied,
//! audited executors. It does not provide arbitrary native execution or an MCP
//! transport.

mod error;
mod json;
mod mcp;
mod policy;
mod registry;
mod schema;

pub use error::{Result, ToolRuntimeError};
pub use mcp::{
    CredentialRef, MAX_CREDENTIAL_REF_BYTES, MAX_MCP_ENDPOINT_BYTES, MAX_MCP_SERVER_ID_BYTES,
    McpTransport, RemoteMcpServerConfig,
};
pub use policy::{ApprovalAuthority, ApprovalToken, ToolPolicy};
pub use registry::{
    AuthorizedToolCall, ExecutorError, ExecutorOutput, MAX_REGISTERED_TOOLS, ToolExecutor,
    ToolRegistry,
};
pub use schema::{
    MAX_CALL_ID_BYTES, MAX_DESCRIPTION_BYTES, MAX_INPUT_SCHEMA_BYTES, MAX_JSON_DEPTH,
    MAX_JSON_NODES, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_NAME_BYTES, MAX_TOOL_OUTPUT_BYTES, ToolCall,
    ToolDefinition, ToolResult, tool_call_json_schema, tool_definition_json_schema,
};
