use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use lorepia_tool_runtime::{
    ApprovalAuthority, AuthorizedToolCall, CredentialRef, ExecutorError, ExecutorOutput,
    MAX_JSON_DEPTH, McpTransport, RemoteMcpServerConfig, ToolCall, ToolDefinition, ToolExecutor,
    ToolPolicy, ToolRegistry, ToolRuntimeError, tool_call_json_schema, tool_definition_json_schema,
};
use serde_json::{Value, json};

const NOW: u64 = 1_900_000_000_000;

struct EchoExecutor {
    calls: Arc<AtomicUsize>,
}

impl ToolExecutor for EchoExecutor {
    fn execute(
        &self,
        call: &AuthorizedToolCall<'_>,
    ) -> std::result::Result<ExecutorOutput, ExecutorError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ExecutorOutput::success(json!({
            "tool": call.name(),
            "arguments": call.arguments()
        }))
        .map_err(|_| ExecutorError::new("OUTPUT_REJECTED").expect("constant code is valid"))
    }
}

fn definition() -> ToolDefinition {
    ToolDefinition::new(
        "lookup.weather",
        "Read a forecast for one named city.",
        json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 80
                }
            },
            "required": ["city"],
            "additionalProperties": false
        }),
    )
    .expect("fixture definition is valid")
}

fn call(city: &str) -> ToolCall {
    ToolCall::new("call:1", "lookup.weather", json!({ "city": city }))
        .expect("fixture call is valid")
}

fn authority() -> ApprovalAuthority {
    ApprovalAuthority::new([0x5a; 32]).expect("fixture key is valid")
}

fn registry(counter: Arc<AtomicUsize>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(definition(), Arc::new(EchoExecutor { calls: counter }))
        .expect("fixture registration succeeds");
    registry
}

#[test]
fn schemas_describe_closed_json_envelopes() {
    assert_eq!(tool_definition_json_schema()["additionalProperties"], false);
    assert_eq!(tool_call_json_schema()["additionalProperties"], false);

    let serialized = serde_json::to_vec(&definition()).expect("definition serializes");
    assert_eq!(
        ToolDefinition::from_json_slice(&serialized).expect("definition parses"),
        definition()
    );
}

#[test]
fn definition_rejects_unsafe_names_and_open_schemas() {
    assert!(matches!(
        ToolDefinition::new(
            "../shell",
            "bad",
            json!({
                "type": "object", "properties": {}, "required": [], "additionalProperties": false
            })
        ),
        Err(ToolRuntimeError::InvalidName { .. })
    ));
    assert!(matches!(
        ToolDefinition::new(
            "unsafe",
            "bad",
            json!({
                "type": "object", "properties": {}, "required": [], "additionalProperties": true
            })
        ),
        Err(ToolRuntimeError::InvalidInputSchema(_))
    ));
    assert!(matches!(
        ToolDefinition::new(
            "remote_ref",
            "bad",
            json!({
                "type": "object", "$ref": "https://attacker.invalid/schema"
            })
        ),
        Err(ToolRuntimeError::InvalidInputSchema(_))
    ));
}

#[test]
fn call_rejects_non_object_and_excessive_depth() {
    assert!(matches!(
        ToolCall::new("call:2", "lookup.weather", json!(["Seoul"])),
        Err(ToolRuntimeError::JsonMustBeObject { .. })
    ));

    let mut nested = Value::Null;
    for _ in 0..=MAX_JSON_DEPTH {
        nested = json!({ "next": nested });
    }
    assert!(matches!(
        ToolCall::new("call:3", "lookup.weather", json!({ "city": nested })),
        Err(ToolRuntimeError::JsonTooDeep { .. })
    ));
}

#[test]
fn registered_tool_is_still_denied_by_default() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(Arc::clone(&calls));
    let authority = authority();
    let call = call("Seoul");
    let token = authority
        .issue_token(&call, NOW, 30_000)
        .expect("approval issues");

    assert!(matches!(
        registry.execute(
            &ToolPolicy::deny_all(),
            &authority,
            &call,
            Some(&token),
            NOW
        ),
        Err(ToolRuntimeError::PolicyDenied { .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn allowlist_still_requires_per_call_approval() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(Arc::clone(&calls));
    let authority = authority();
    let policy = ToolPolicy::allow_only(["lookup.weather"]).expect("allowlist is valid");
    let call = call("Seoul");

    assert_eq!(
        registry.execute(&policy, &authority, &call, None, NOW),
        Err(ToolRuntimeError::ApprovalRequired)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn approved_call_executes_once_and_token_cannot_replay() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(Arc::clone(&calls));
    let authority = authority();
    let policy = ToolPolicy::allow_only(["lookup.weather"]).expect("allowlist is valid");
    let call = call("Seoul");
    let token = authority
        .issue_token(&call, NOW, 30_000)
        .expect("approval issues");

    let result = registry
        .execute(&policy, &authority, &call, Some(&token), NOW)
        .expect("approved tool executes");
    assert_eq!(result.call_id(), "call:1");
    assert!(!result.is_error());
    assert_eq!(result.content()["arguments"]["city"], "Seoul");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    assert_eq!(
        registry.execute(&policy, &authority, &call, Some(&token), NOW),
        Err(ToolRuntimeError::ApprovalReplay)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn approval_is_bound_to_exact_call_arguments() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(Arc::clone(&calls));
    let authority = authority();
    let policy = ToolPolicy::allow_only(["lookup.weather"]).expect("allowlist is valid");
    let approved_call = call("Seoul");
    let changed_call = call("Busan");
    let token = authority
        .issue_token(&approved_call, NOW, 30_000)
        .expect("approval issues");

    assert_eq!(
        registry.execute(&policy, &authority, &changed_call, Some(&token), NOW),
        Err(ToolRuntimeError::InvalidApprovalToken)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn schema_is_enforced_before_approval_is_consumed() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(Arc::clone(&calls));
    let authority = authority();
    let policy = ToolPolicy::allow_only(["lookup.weather"]).expect("allowlist is valid");
    let bad_call = ToolCall::new(
        "call:4",
        "lookup.weather",
        json!({ "city": 42, "unexpected": true }),
    )
    .expect("envelope is valid before schema matching");
    let token = authority
        .issue_token(&bad_call, NOW, 30_000)
        .expect("approval issues");

    assert!(matches!(
        registry.execute(&policy, &authority, &bad_call, Some(&token), NOW),
        Err(ToolRuntimeError::InvalidToolArguments(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn expired_approval_is_rejected() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(calls);
    let authority = authority();
    let policy = ToolPolicy::allow_only(["lookup.weather"]).expect("allowlist is valid");
    let call = call("Seoul");
    let token = authority
        .issue_token(&call, NOW, 1_000)
        .expect("approval issues");

    assert_eq!(
        registry.execute(&policy, &authority, &call, Some(&token), NOW + 1_001),
        Err(ToolRuntimeError::ApprovalExpired)
    );
}

#[test]
fn remote_mcp_accepts_only_https_public_dns_and_credential_reference() {
    let credential = CredentialRef::new("mcp.weather.primary").expect("reference is valid");
    let config =
        RemoteMcpServerConfig::new("weather", "https://mcp.example.com/v1", credential.clone())
            .expect("public HTTPS endpoint is accepted");
    assert_eq!(config.transport(), McpTransport::StreamableHttp);
    assert_eq!(config.credential_ref(), &credential);

    for endpoint in [
        "http://mcp.example.com/v1",
        "https://localhost/v1",
        "https://service.local/v1",
        "https://printer.lan/v1",
        "https://192.168.1.4/v1",
        "https://[::1]/v1",
        "https://secret@mcp.example.com/v1",
        "https://mcp.example.com/v1?token=secret",
        "https://mcp/v1",
    ] {
        assert!(
            RemoteMcpServerConfig::new("blocked", endpoint, credential.clone()).is_err(),
            "endpoint should be rejected: {endpoint}"
        );
    }
}

#[test]
fn stdio_and_inline_secret_fields_are_rejected() {
    let stdio = br#"{
        "serverId":"local",
        "transport":"stdio",
        "endpointUrl":"https://mcp.example.com/v1",
        "credentialRef":"mcp.local"
    }"#;
    assert_eq!(
        RemoteMcpServerConfig::from_json_slice(stdio),
        Err(ToolRuntimeError::UnsupportedMcpTransport)
    );

    let inline_secret = br#"{
        "serverId":"remote",
        "transport":"streamable_http",
        "endpointUrl":"https://mcp.example.com/v1",
        "credentialRef":"mcp.remote",
        "apiKey":"secret"
    }"#;
    assert_eq!(
        RemoteMcpServerConfig::from_json_slice(inline_secret),
        Err(ToolRuntimeError::InvalidJson)
    );
}
