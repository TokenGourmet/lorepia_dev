use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::{
    ApprovalAuthority, ApprovalToken, Result, ToolCall, ToolDefinition, ToolPolicy, ToolResult,
    ToolRuntimeError,
};

pub const MAX_REGISTERED_TOOLS: usize = 64;

pub struct AuthorizedToolCall<'a> {
    call: &'a ToolCall,
}

impl AuthorizedToolCall<'_> {
    #[must_use]
    pub fn id(&self) -> &str {
        self.call.id()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        self.call.name()
    }

    #[must_use]
    pub fn arguments(&self) -> &Value {
        self.call.arguments()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExecutorOutput {
    is_error: bool,
    content: Value,
}

impl ExecutorOutput {
    pub fn success(content: Value) -> Result<Self> {
        validate_output(content, false)
    }

    pub fn failure(content: Value) -> Result<Self> {
        validate_output(content, true)
    }

    #[must_use]
    pub const fn is_error(&self) -> bool {
        self.is_error
    }

    #[must_use]
    pub fn content(&self) -> &Value {
        &self.content
    }
}

fn validate_output(content: Value, is_error: bool) -> Result<ExecutorOutput> {
    // ToolResult is the single authoritative output limit check.
    ToolResult::new("validation".to_owned(), is_error, content.clone())?;
    Ok(ExecutorOutput { is_error, content })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutorError {
    code: String,
}

impl ExecutorError {
    pub fn new(code: impl Into<String>) -> Result<Self> {
        let code = code.into();
        if code.is_empty()
            || code.len() > 64
            || !code.bytes().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_uppercase()
                } else {
                    byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                }
            })
        {
            return Err(ToolRuntimeError::InvalidExecutorCode);
        }
        Ok(Self { code })
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

/// An app-owned, audited tool implementation.
///
/// The crate provides no shell, process, filesystem, or native-command
/// implementation. Registration alone is insufficient: every dispatch still
/// requires an allowlist entry and a call-bound approval token.
pub trait ToolExecutor: Send + Sync {
    fn execute(
        &self,
        call: &AuthorizedToolCall<'_>,
    ) -> std::result::Result<ExecutorOutput, ExecutorError>;
}

struct RegisteredTool {
    definition: ToolDefinition,
    executor: Arc<dyn ToolExecutor>,
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, RegisteredTool>,
}

impl ToolRegistry {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        definition: ToolDefinition,
        executor: Arc<dyn ToolExecutor>,
    ) -> Result<()> {
        definition.validate()?;
        if self.tools.contains_key(definition.name()) {
            return Err(ToolRuntimeError::DuplicateTool {
                tool_name: definition.name().to_owned(),
            });
        }
        if self.tools.len() >= MAX_REGISTERED_TOOLS {
            return Err(ToolRuntimeError::TooManyTools {
                max: MAX_REGISTERED_TOOLS,
            });
        }
        self.tools.insert(
            definition.name().to_owned(),
            RegisteredTool {
                definition,
                executor,
            },
        );
        Ok(())
    }

    #[must_use]
    pub fn definition(&self, tool_name: &str) -> Option<&ToolDefinition> {
        self.tools.get(tool_name).map(|entry| &entry.definition)
    }

    pub fn definitions(&self) -> impl Iterator<Item = &ToolDefinition> {
        self.tools.values().map(|entry| &entry.definition)
    }

    pub fn execute(
        &self,
        policy: &ToolPolicy,
        authority: &ApprovalAuthority,
        call: &ToolCall,
        approval: Option<&ApprovalToken>,
        now_unix_ms: u64,
    ) -> Result<ToolResult> {
        call.validate()?;
        let registered =
            self.tools
                .get(call.name())
                .ok_or_else(|| ToolRuntimeError::ToolNotRegistered {
                    tool_name: call.name().to_owned(),
                })?;
        registered.definition.validate_arguments(call.arguments())?;
        policy.authorize(authority, call, approval, now_unix_ms)?;

        let authorized = AuthorizedToolCall { call };
        let output = registered.executor.execute(&authorized).map_err(|error| {
            ToolRuntimeError::ExecutorFailed {
                code: error.code().to_owned(),
            }
        })?;
        ToolResult::new(call.id().to_owned(), output.is_error, output.content)
    }
}
