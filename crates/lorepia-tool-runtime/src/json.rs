use serde_json::Value;

use crate::{Result, ToolRuntimeError};

pub(crate) fn parse_json(bytes: &[u8], field: &'static str, max_bytes: usize) -> Result<Value> {
    if bytes.len() > max_bytes {
        return Err(ToolRuntimeError::JsonTooLarge { field, max_bytes });
    }
    serde_json::from_slice(bytes).map_err(|_| ToolRuntimeError::InvalidJson)
}

pub(crate) fn validate_json_limits(
    value: &Value,
    field: &'static str,
    max_bytes: usize,
    max_depth: usize,
    max_nodes: usize,
) -> Result<()> {
    let mut stack = vec![(value, 1_usize)];
    let mut node_count = 0_usize;

    while let Some((node, depth)) = stack.pop() {
        node_count = node_count
            .checked_add(1)
            .ok_or(ToolRuntimeError::JsonTooManyNodes { field, max_nodes })?;
        if node_count > max_nodes {
            return Err(ToolRuntimeError::JsonTooManyNodes { field, max_nodes });
        }
        if depth > max_depth {
            return Err(ToolRuntimeError::JsonTooDeep { field, max_depth });
        }

        match node {
            Value::Array(items) => {
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Value::Object(object) => {
                stack.extend(object.values().map(|item| (item, depth + 1)));
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    let serialized = serde_json::to_vec(value).map_err(|_| ToolRuntimeError::InvalidJson)?;
    if serialized.len() > max_bytes {
        return Err(ToolRuntimeError::JsonTooLarge { field, max_bytes });
    }
    Ok(())
}

pub(crate) fn canonical_json(value: &Value) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    write_canonical(value, &mut output)?;
    Ok(output)
}

fn write_canonical(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
        Value::String(value) => output.extend_from_slice(
            serde_json::to_string(value)
                .map_err(|_| ToolRuntimeError::InvalidJson)?
                .as_bytes(),
        ),
        Value::Array(items) => {
            output.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_canonical(item, output)?;
            }
            output.push(b']');
        }
        Value::Object(object) => {
            output.push(b'{');
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                output.extend_from_slice(
                    serde_json::to_string(key)
                        .map_err(|_| ToolRuntimeError::InvalidJson)?
                        .as_bytes(),
                );
                output.push(b':');
                write_canonical(item, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}
