use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};

use crate::{PromptError, PromptPreset, Result, validate::MAX_IMPORT_BYTES, validate_preset};

pub const IMPORT_FORMAT: &str = "lorepia.prompt-preset";
pub const IMPORT_SCHEMA_VERSION: u32 = 1;
const MAX_JSON_DEPTH: usize = 16;
const MAX_JSON_NODES: usize = 16_384;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportEnvelope {
    format: String,
    schema_version: u32,
    preset: PromptPreset,
}

pub fn import_preset(bytes: &[u8]) -> Result<PromptPreset> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(PromptError::too_large("prompt import", MAX_IMPORT_BYTES));
    }
    let unique: UniqueValue = serde_json::from_slice(bytes)?;
    let mut preset = decode_versioned_envelope(unique.0)?;

    // Imported references and transforms cannot grant themselves local
    // execution authority. A separate local approval flow may enable them.
    for reference in &mut preset.advanced.tools {
        reference.enabled = false;
    }
    for reference in &mut preset.advanced.modules {
        reference.enabled = false;
    }
    for rule in &mut preset.advanced.regex {
        rule.enabled = false;
    }
    validate_preset(&preset)?;
    Ok(preset)
}

fn decode_versioned_envelope(value: Value) -> Result<PromptPreset> {
    let Value::Object(mut envelope) = value else {
        return Err(closed_schema());
    };
    if envelope.len() != 3
        || !envelope.contains_key("format")
        || !envelope.contains_key("schemaVersion")
        || !envelope.contains_key("preset")
    {
        return Err(closed_schema());
    }

    let format = envelope
        .remove("format")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(closed_schema)?;
    if format != IMPORT_FORMAT {
        return Err(PromptError::Import(
            "unsupported prompt import format".to_owned(),
        ));
    }
    let schema_version = envelope
        .remove("schemaVersion")
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(closed_schema)?;
    let preset = envelope.remove("preset").ok_or_else(closed_schema)?;

    match schema_version {
        1 => decode_v1(preset),
        version => Err(PromptError::Import(format!(
            "unsupported schema version {version}; expected {IMPORT_SCHEMA_VERSION}"
        ))),
    }
}

fn decode_v1(value: Value) -> Result<PromptPreset> {
    // When v2 exists, this branch must deserialize a frozen V1 DTO and pass it
    // through an explicit V1 -> V2 -> current migration chain. Version dispatch
    // intentionally happens before current-schema deserialization.
    serde_json::from_value(value).map_err(|_| closed_schema())
}

fn closed_schema() -> PromptError {
    PromptError::Import("prompt import does not match the closed schema".to_owned())
}

pub fn export_preset(preset: &PromptPreset) -> Result<Vec<u8>> {
    validate_preset(preset)?;
    let bytes = serde_json::to_vec_pretty(&ImportEnvelope {
        format: IMPORT_FORMAT.to_owned(),
        schema_version: IMPORT_SCHEMA_VERSION,
        preset: preset.clone(),
    })?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(PromptError::too_large("prompt export", MAX_IMPORT_BYTES));
    }
    Ok(bytes)
}

struct UniqueValue(Value);

#[derive(Default)]
struct ParseBudget {
    nodes: usize,
}

impl ParseBudget {
    fn charge<E: de::Error>(&mut self, depth: usize) -> std::result::Result<(), E> {
        if depth > MAX_JSON_DEPTH {
            return Err(E::custom(format!("JSON depth exceeds {MAX_JSON_DEPTH}")));
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| E::custom("JSON node count overflow"))?;
        if self.nodes > MAX_JSON_NODES {
            return Err(E::custom(format!(
                "JSON node count exceeds {MAX_JSON_NODES}"
            )));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut budget = ParseBudget::default();
        UniqueValueSeed {
            budget: &mut budget,
            depth: 1,
        }
        .deserialize(deserializer)
    }
}

struct UniqueValueSeed<'a> {
    budget: &'a mut ParseBudget,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for UniqueValueSeed<'_> {
    type Value = UniqueValue;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        self.budget.charge::<D::Error>(self.depth)?;
        deserializer.deserialize_any(UniqueValueVisitor {
            budget: self.budget,
            depth: self.depth,
        })
    }
}

struct UniqueValueVisitor<'a> {
    budget: &'a mut ParseBudget,
    depth: usize,
}

impl<'de> Visitor<'de> for UniqueValueVisitor<'_> {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(UniqueValue(value)) = sequence.next_element_seed(UniqueValueSeed {
            budget: &mut *self.budget,
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(UniqueValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON key"));
            }
            let UniqueValue(value) = object.next_value_seed(UniqueValueSeed {
                budget: &mut *self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(UniqueValue(Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AdvancedSettings, ModuleReference, PromptBlock, PromptRole, PromptSampling, RegexFlags,
        RegexRule, ToolReference, TransformTarget,
    };

    fn preset() -> PromptPreset {
        PromptPreset {
            name: "portable".to_owned(),
            blocks: vec![PromptBlock::Raw {
                name: "main".to_owned(),
                enabled: true,
                role: PromptRole::System,
                special: None,
                prompt: "Be concise.".to_owned(),
            }],
            sampling: PromptSampling::default(),
            advanced: AdvancedSettings {
                tools: vec![ToolReference {
                    tool_id: "search".to_owned(),
                    enabled: true,
                }],
                regex: vec![RegexRule {
                    name: "trim".to_owned(),
                    enabled: true,
                    target: TransformTarget::Request,
                    pattern: " +".to_owned(),
                    replacement: " ".to_owned(),
                    flags: RegexFlags::default(),
                }],
                modules: vec![ModuleReference {
                    module_id: "story.module".to_owned(),
                    version: "1.0.0".to_owned(),
                    digest_sha256: "a".repeat(64),
                    enabled: true,
                }],
                ..AdvancedSettings::default()
            },
        }
    }

    #[test]
    fn round_trip_disables_imported_executable_references() {
        let bytes = export_preset(&preset()).unwrap();
        let imported = import_preset(&bytes).unwrap();
        assert!(!imported.advanced.tools[0].enabled);
        assert!(!imported.advanced.regex[0].enabled);
        assert!(!imported.advanced.modules[0].enabled);
    }

    #[test]
    fn duplicate_keys_are_rejected_before_typed_deserialization() {
        let json =
            br#"{"format":"lorepia.prompt-preset","format":"other","schemaVersion":1,"preset":{}}"#;
        let error = import_preset(json).unwrap_err();
        assert!(error.to_string().contains("duplicate JSON key"));
    }

    #[test]
    fn recursive_unknown_fields_are_rejected() {
        let mut value: Value = serde_json::from_slice(&export_preset(&preset()).unwrap()).unwrap();
        value["preset"]["blocks"][0]["foreignField"] = Value::Bool(true);
        let error = import_preset(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.to_string().contains("closed schema"));
    }

    #[test]
    fn version_is_dispatched_before_preset_deserialization() {
        let value = serde_json::json!({
            "format": IMPORT_FORMAT,
            "schemaVersion": 2,
            "preset": {"not": "the current schema"}
        });
        let error = import_preset(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.to_string().contains("unsupported schema version 2"));
    }

    #[test]
    fn direct_preset_deserialization_cannot_restore_execution_authority() {
        let value: Value = serde_json::from_slice(&export_preset(&preset()).unwrap()).unwrap();
        let direct: PromptPreset = serde_json::from_value(value["preset"].clone()).unwrap();
        assert!(!direct.advanced.tools[0].enabled);
        assert!(!direct.advanced.regex[0].enabled);
        assert!(!direct.advanced.modules[0].enabled);
    }

    #[test]
    fn import_rejects_context_free_block_invariants() {
        let bytes = export_preset(&preset()).unwrap();
        let mut wrong_role: Value = serde_json::from_slice(&bytes).unwrap();
        wrong_role["preset"]["blocks"][0]["special"] = Value::String("main".to_owned());
        wrong_role["preset"]["blocks"][0]["role"] = Value::String("user".to_owned());
        assert!(import_preset(&serde_json::to_vec(&wrong_role).unwrap()).is_err());

        let mut duplicate: Value = serde_json::from_slice(&bytes).unwrap();
        duplicate["preset"]["blocks"][0]["special"] = Value::String("main".to_owned());
        let second = duplicate["preset"]["blocks"][0].clone();
        duplicate["preset"]["blocks"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(import_preset(&serde_json::to_vec(&duplicate).unwrap()).is_err());

        let mut bad_cache: Value = serde_json::from_slice(&bytes).unwrap();
        bad_cache["preset"]["blocks"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "type": "cache_point",
                "name": "cache",
                "enabled": true,
                "depth": 0,
                "role": "user"
            }));
        assert!(import_preset(&serde_json::to_vec(&bad_cache).unwrap()).is_err());
    }

    #[test]
    fn import_rejects_disabled_invalid_regex_syntax() {
        let mut value: Value = serde_json::from_slice(&export_preset(&preset()).unwrap()).unwrap();
        value["preset"]["advanced"]["regex"][0]["pattern"] =
            Value::String("(?=lookahead)".to_owned());
        value["preset"]["advanced"]["regex"][0]["enabled"] = Value::Bool(false);
        assert!(import_preset(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn import_enforces_size_depth_and_node_limits_before_schema_use() {
        assert!(import_preset(&vec![b' '; MAX_IMPORT_BYTES + 1]).is_err());

        let mut deep = Value::Null;
        for _ in 0..=MAX_JSON_DEPTH {
            deep = Value::Array(vec![deep]);
        }
        let mut depth_value: Value =
            serde_json::from_slice(&export_preset(&preset()).unwrap()).unwrap();
        depth_value["unexpected"] = deep;
        let depth_error = import_preset(&serde_json::to_vec(&depth_value).unwrap()).unwrap_err();
        assert!(depth_error.to_string().contains("depth exceeds"));

        let mut node_value: Value =
            serde_json::from_slice(&export_preset(&preset()).unwrap()).unwrap();
        node_value["unexpected"] = Value::Array(vec![Value::Null; MAX_JSON_NODES]);
        let node_error = import_preset(&serde_json::to_vec(&node_value).unwrap()).unwrap_err();
        assert!(node_error.to_string().contains("node count exceeds"));
    }
}
