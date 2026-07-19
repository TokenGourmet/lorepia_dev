use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::json::{parse_json, validate_json_limits};
use crate::{Result, ToolRuntimeError};

pub const MAX_TOOL_NAME_BYTES: usize = 64;
pub const MAX_CALL_ID_BYTES: usize = 128;
pub const MAX_DESCRIPTION_BYTES: usize = 4 * 1024;
pub const MAX_INPUT_SCHEMA_BYTES: usize = 32 * 1024;
pub const MAX_TOOL_ARGUMENT_BYTES: usize = 64 * 1024;
pub const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_JSON_DEPTH: usize = 16;
pub const MAX_JSON_NODES: usize = 4_096;

const MAX_DEFINITION_DOCUMENT_BYTES: usize = MAX_INPUT_SCHEMA_BYTES + MAX_DESCRIPTION_BYTES + 1024;
const MAX_CALL_DOCUMENT_BYTES: usize = MAX_TOOL_ARGUMENT_BYTES + 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolDefinition {
    name: String,
    description: String,
    input_schema: Value,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Result<Self> {
        let definition = Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        };
        definition.validate()?;
        Ok(definition)
    }

    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let value = parse_json(bytes, "tool_definition", MAX_DEFINITION_DOCUMENT_BYTES)?;
        let definition: Self =
            serde_json::from_value(value).map_err(|_| ToolRuntimeError::InvalidJson)?;
        definition.validate()?;
        Ok(definition)
    }

    pub fn validate(&self) -> Result<()> {
        validate_tool_name(&self.name)?;
        if self.description.trim().is_empty() {
            return Err(ToolRuntimeError::EmptyField("description"));
        }
        if self.description.len() > MAX_DESCRIPTION_BYTES {
            return Err(ToolRuntimeError::FieldTooLong {
                field: "description",
                max_bytes: MAX_DESCRIPTION_BYTES,
            });
        }
        validate_json_limits(
            &self.input_schema,
            "input_schema",
            MAX_INPUT_SCHEMA_BYTES,
            MAX_JSON_DEPTH,
            MAX_JSON_NODES,
        )?;
        validate_schema(&self.input_schema, true)
    }

    pub fn validate_arguments(&self, arguments: &Value) -> Result<()> {
        validate_json_limits(
            arguments,
            "arguments",
            MAX_TOOL_ARGUMENT_BYTES,
            MAX_JSON_DEPTH,
            MAX_JSON_NODES,
        )?;
        if !arguments.is_object() {
            return Err(ToolRuntimeError::JsonMustBeObject { field: "arguments" });
        }
        validate_value_against_schema(arguments, &self.input_schema)
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCall {
    id: String,
    name: String,
    arguments: Value,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Result<Self> {
        let call = Self {
            id: id.into(),
            name: name.into(),
            arguments,
        };
        call.validate()?;
        Ok(call)
    }

    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let value = parse_json(bytes, "tool_call", MAX_CALL_DOCUMENT_BYTES)?;
        let call: Self =
            serde_json::from_value(value).map_err(|_| ToolRuntimeError::InvalidJson)?;
        call.validate()?;
        Ok(call)
    }

    pub fn validate(&self) -> Result<()> {
        validate_call_id(&self.id)?;
        validate_tool_name(&self.name)?;
        validate_json_limits(
            &self.arguments,
            "arguments",
            MAX_TOOL_ARGUMENT_BYTES,
            MAX_JSON_DEPTH,
            MAX_JSON_NODES,
        )?;
        if !self.arguments.is_object() {
            return Err(ToolRuntimeError::JsonMustBeObject { field: "arguments" });
        }
        Ok(())
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolResult {
    call_id: String,
    is_error: bool,
    content: Value,
}

impl ToolResult {
    pub(crate) fn new(call_id: String, is_error: bool, content: Value) -> Result<Self> {
        validate_call_id(&call_id)?;
        validate_json_limits(
            &content,
            "tool_output",
            MAX_TOOL_OUTPUT_BYTES,
            MAX_JSON_DEPTH,
            MAX_JSON_NODES,
        )?;
        Ok(Self {
            call_id,
            is_error,
            content,
        })
    }

    #[must_use]
    pub fn call_id(&self) -> &str {
        &self.call_id
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

#[must_use]
pub fn tool_definition_json_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "description", "inputSchema"],
        "properties": {
            "name": {
                "type": "string",
                "pattern": "^[A-Za-z][A-Za-z0-9_.-]{0,63}$",
                "maxLength": MAX_TOOL_NAME_BYTES
            },
            "description": { "type": "string", "minLength": 1, "maxLength": MAX_DESCRIPTION_BYTES },
            "inputSchema": { "type": "object" }
        }
    })
}

#[must_use]
pub fn tool_call_json_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "name", "arguments"],
        "properties": {
            "id": {
                "type": "string",
                "pattern": "^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$",
                "maxLength": MAX_CALL_ID_BYTES
            },
            "name": {
                "type": "string",
                "pattern": "^[A-Za-z][A-Za-z0-9_.-]{0,63}$",
                "maxLength": MAX_TOOL_NAME_BYTES
            },
            "arguments": { "type": "object" }
        }
    })
}

pub(crate) fn validate_tool_name(name: &str) -> Result<()> {
    validate_ascii_identifier(name, "tool_name", MAX_TOOL_NAME_BYTES, false)
}

fn validate_call_id(id: &str) -> Result<()> {
    validate_ascii_identifier(id, "call_id", MAX_CALL_ID_BYTES, true)
}

fn validate_ascii_identifier(
    value: &str,
    field: &'static str,
    max_bytes: usize,
    allow_colon: bool,
) -> Result<()> {
    if value.is_empty() {
        return Err(ToolRuntimeError::EmptyField(field));
    }
    if value.len() > max_bytes {
        return Err(ToolRuntimeError::FieldTooLong { field, max_bytes });
    }
    let mut bytes = value.bytes();
    let first = bytes.next().ok_or(ToolRuntimeError::EmptyField(field))?;
    let first_is_valid = if field == "tool_name" {
        first.is_ascii_alphabetic()
    } else {
        first.is_ascii_alphanumeric()
    };
    let rest_is_valid = bytes.all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'_' | b'.' | b'-')
            || (allow_colon && byte == b':')
    });
    if !first_is_valid || !rest_is_valid || value.contains("..") {
        return Err(ToolRuntimeError::InvalidName { field });
    }
    Ok(())
}

fn validate_schema(schema: &Value, root: bool) -> Result<()> {
    let object = schema
        .as_object()
        .ok_or(ToolRuntimeError::InvalidInputSchema(
            "schema nodes must be objects",
        ))?;

    const ALLOWED_KEYS: [&str; 15] = [
        "type",
        "title",
        "description",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "const",
        "minLength",
        "maxLength",
        "minimum",
        "maximum",
        "minItems",
        "maxItems",
    ];
    if object
        .keys()
        .any(|key| !ALLOWED_KEYS.contains(&key.as_str()))
    {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "unsupported or reference-based schema keyword",
        ));
    }

    let kind =
        object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ToolRuntimeError::InvalidInputSchema(
                "every schema node requires one string type",
            ))?;
    if root && kind != "object" {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "root input schema must have object type",
        ));
    }
    if !matches!(
        kind,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    ) {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "unsupported JSON type",
        ));
    }

    if let Some(title) = object.get("title") {
        validate_schema_text(title)?;
    }
    if let Some(description) = object.get("description") {
        validate_schema_text(description)?;
    }
    if let Some(values) = object.get("enum") {
        let values = values
            .as_array()
            .ok_or(ToolRuntimeError::InvalidInputSchema(
                "enum must be an array",
            ))?;
        if values.is_empty() || values.len() > 128 {
            return Err(ToolRuntimeError::InvalidInputSchema("enum size is invalid"));
        }
        if values.iter().any(|value| !value_matches_type(value, kind)) {
            return Err(ToolRuntimeError::InvalidInputSchema(
                "enum value has the wrong type",
            ));
        }
    }
    if let Some(value) = object.get("const")
        && !value_matches_type(value, kind)
    {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "const value has the wrong type",
        ));
    }

    match kind {
        "object" => validate_object_schema(object),
        "array" => validate_array_schema(object),
        "string" => validate_string_schema(object),
        "number" | "integer" => validate_number_schema(object),
        "boolean" | "null" => {
            validate_irrelevant_keywords(object, &["type", "title", "description", "enum", "const"])
        }
        _ => Err(ToolRuntimeError::InvalidInputSchema(
            "unsupported JSON type",
        )),
    }
}

fn validate_object_schema(object: &Map<String, Value>) -> Result<()> {
    validate_irrelevant_keywords(
        object,
        &[
            "type",
            "title",
            "description",
            "properties",
            "required",
            "additionalProperties",
            "enum",
            "const",
        ],
    )?;
    let properties = object.get("properties").and_then(Value::as_object).ok_or(
        ToolRuntimeError::InvalidInputSchema("object schema requires properties"),
    )?;
    if properties.len() > 128 {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "too many object properties",
        ));
    }
    for (name, child) in properties {
        validate_argument_name(name)?;
        validate_schema(child, false)?;
    }

    let required = object.get("required").and_then(Value::as_array).ok_or(
        ToolRuntimeError::InvalidInputSchema("object schema requires a required array"),
    )?;
    let mut unique = BTreeSet::new();
    for value in required {
        let name = value.as_str().ok_or(ToolRuntimeError::InvalidInputSchema(
            "required entries must be strings",
        ))?;
        if !properties.contains_key(name) || !unique.insert(name) {
            return Err(ToolRuntimeError::InvalidInputSchema(
                "required entries must be unique declared properties",
            ));
        }
    }

    if object.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "additionalProperties must be false",
        ));
    }
    Ok(())
}

fn validate_array_schema(object: &Map<String, Value>) -> Result<()> {
    validate_irrelevant_keywords(
        object,
        &[
            "type",
            "title",
            "description",
            "items",
            "minItems",
            "maxItems",
            "enum",
            "const",
        ],
    )?;
    let items = object
        .get("items")
        .ok_or(ToolRuntimeError::InvalidInputSchema(
            "array schema requires items",
        ))?;
    validate_schema(items, false)?;
    validate_u64_bounds(object, "minItems", "maxItems", MAX_JSON_NODES as u64)
}

fn validate_string_schema(object: &Map<String, Value>) -> Result<()> {
    validate_irrelevant_keywords(
        object,
        &[
            "type",
            "title",
            "description",
            "minLength",
            "maxLength",
            "enum",
            "const",
        ],
    )?;
    validate_u64_bounds(
        object,
        "minLength",
        "maxLength",
        MAX_TOOL_ARGUMENT_BYTES as u64,
    )
}

fn validate_number_schema(object: &Map<String, Value>) -> Result<()> {
    validate_irrelevant_keywords(
        object,
        &[
            "type",
            "title",
            "description",
            "minimum",
            "maximum",
            "enum",
            "const",
        ],
    )?;
    let minimum = object.get("minimum").map(number_as_f64).transpose()?;
    let maximum = object.get("maximum").map(number_as_f64).transpose()?;
    if minimum.zip(maximum).is_some_and(|(min, max)| min > max) {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "minimum exceeds maximum",
        ));
    }
    Ok(())
}

fn validate_irrelevant_keywords(object: &Map<String, Value>, relevant: &[&str]) -> Result<()> {
    if object.keys().any(|key| !relevant.contains(&key.as_str())) {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "keyword is not valid for this schema type",
        ));
    }
    Ok(())
}

fn validate_schema_text(value: &Value) -> Result<()> {
    let value = value.as_str().ok_or(ToolRuntimeError::InvalidInputSchema(
        "schema text metadata must be a string",
    ))?;
    if value.len() > MAX_DESCRIPTION_BYTES {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "schema text metadata is too long",
        ));
    }
    Ok(())
}

fn validate_argument_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_TOOL_NAME_BYTES {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "property name length is invalid",
        ));
    }
    let mut bytes = name.bytes();
    let first = bytes
        .next()
        .ok_or(ToolRuntimeError::InvalidInputSchema("empty property name"))?;
    if !first.is_ascii_alphabetic()
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "property name is not a safe identifier",
        ));
    }
    Ok(())
}

fn validate_u64_bounds(
    object: &Map<String, Value>,
    minimum_key: &'static str,
    maximum_key: &'static str,
    hard_max: u64,
) -> Result<()> {
    let minimum = object
        .get(minimum_key)
        .map(json_u64)
        .transpose()?
        .unwrap_or(0);
    let maximum = object
        .get(maximum_key)
        .map(json_u64)
        .transpose()?
        .unwrap_or(hard_max);
    if minimum > maximum || maximum > hard_max {
        return Err(ToolRuntimeError::InvalidInputSchema(
            "schema bounds are invalid",
        ));
    }
    Ok(())
}

fn json_u64(value: &Value) -> Result<u64> {
    value.as_u64().ok_or(ToolRuntimeError::InvalidInputSchema(
        "schema bound must be an unsigned integer",
    ))
}

fn number_as_f64(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or(ToolRuntimeError::InvalidInputSchema(
            "numeric bound must be finite",
        ))
}

fn validate_value_against_schema(value: &Value, schema: &Value) -> Result<()> {
    let object = schema
        .as_object()
        .ok_or(ToolRuntimeError::InvalidInputSchema(
            "schema node must be an object",
        ))?;
    let kind =
        object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ToolRuntimeError::InvalidInputSchema(
                "schema node has no type",
            ))?;
    if !value_matches_type(value, kind) {
        return Err(ToolRuntimeError::InvalidToolArguments(
            "value has the wrong type",
        ));
    }
    if object
        .get("const")
        .is_some_and(|expected| expected != value)
    {
        return Err(ToolRuntimeError::InvalidToolArguments(
            "const value does not match",
        ));
    }
    if object
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| !values.contains(value))
    {
        return Err(ToolRuntimeError::InvalidToolArguments(
            "value is not in enum",
        ));
    }

    match kind {
        "object" => validate_object_value(value, object),
        "array" => validate_array_value(value, object),
        "string" => validate_string_value(value, object),
        "number" | "integer" => validate_number_value(value, object),
        "boolean" | "null" => Ok(()),
        _ => Err(ToolRuntimeError::InvalidInputSchema(
            "unsupported JSON type",
        )),
    }
}

fn validate_object_value(value: &Value, schema: &Map<String, Value>) -> Result<()> {
    let value = value
        .as_object()
        .ok_or(ToolRuntimeError::InvalidToolArguments("expected object"))?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(ToolRuntimeError::InvalidInputSchema("properties missing"))?;
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .ok_or(ToolRuntimeError::InvalidInputSchema("required missing"))?;
    for name in required.iter().filter_map(Value::as_str) {
        if !value.contains_key(name) {
            return Err(ToolRuntimeError::InvalidToolArguments(
                "required property is missing",
            ));
        }
    }
    for (name, child) in value {
        let child_schema = properties
            .get(name)
            .ok_or(ToolRuntimeError::InvalidToolArguments(
                "additional property is forbidden",
            ))?;
        validate_value_against_schema(child, child_schema)?;
    }
    Ok(())
}

fn validate_array_value(value: &Value, schema: &Map<String, Value>) -> Result<()> {
    let values = value
        .as_array()
        .ok_or(ToolRuntimeError::InvalidToolArguments("expected array"))?;
    let minimum = schema.get("minItems").and_then(Value::as_u64).unwrap_or(0) as usize;
    let maximum = schema
        .get("maxItems")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_JSON_NODES as u64) as usize;
    if !(minimum..=maximum).contains(&values.len()) {
        return Err(ToolRuntimeError::InvalidToolArguments(
            "array length is outside schema bounds",
        ));
    }
    let item_schema = schema
        .get("items")
        .ok_or(ToolRuntimeError::InvalidInputSchema("items missing"))?;
    for value in values {
        validate_value_against_schema(value, item_schema)?;
    }
    Ok(())
}

fn validate_string_value(value: &Value, schema: &Map<String, Value>) -> Result<()> {
    let value = value
        .as_str()
        .ok_or(ToolRuntimeError::InvalidToolArguments("expected string"))?;
    let character_count = value.chars().count();
    let minimum = schema.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
    let maximum = schema
        .get("maxLength")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_TOOL_ARGUMENT_BYTES as u64) as usize;
    if !(minimum..=maximum).contains(&character_count) {
        return Err(ToolRuntimeError::InvalidToolArguments(
            "string length is outside schema bounds",
        ));
    }
    Ok(())
}

fn validate_number_value(value: &Value, schema: &Map<String, Value>) -> Result<()> {
    let value = value
        .as_f64()
        .ok_or(ToolRuntimeError::InvalidToolArguments("expected number"))?;
    let minimum = schema.get("minimum").and_then(Value::as_f64);
    let maximum = schema.get("maximum").and_then(Value::as_f64);
    if minimum.is_some_and(|minimum| value < minimum)
        || maximum.is_some_and(|maximum| value > maximum)
    {
        return Err(ToolRuntimeError::InvalidToolArguments(
            "number is outside schema bounds",
        ));
    }
    Ok(())
}

fn value_matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}
