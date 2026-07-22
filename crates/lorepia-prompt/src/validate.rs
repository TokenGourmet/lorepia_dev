use std::collections::BTreeSet;

use crate::{
    ContentFormat, PromptBlock, PromptError, PromptPreset, PromptSampling, Result,
    regex_pipeline::validate_regex_contract,
    template::{validate_template_syntax, validate_variable_name},
};

pub(crate) const MAX_IMPORT_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_BLOCKS: usize = 256;
pub(crate) const MAX_FIELD_TEXT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_AUTHORED_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_VARIABLES: usize = 256;
pub(crate) const MAX_VARIABLE_VALUE_BYTES: usize = 256 * 1024;
pub(crate) const MAX_REFERENCES: usize = 32;
pub(crate) const MAX_REGEX_RULES: usize = 128;
pub(crate) const MAX_ACTIVE_REGEX_RULES: usize = 64;
pub(crate) const MAX_REGEX_PATTERN_BYTES: usize = 16 * 1024;
pub(crate) const MAX_REGEX_REPLACEMENT_BYTES: usize = 64 * 1024;

pub fn validate_preset(preset: &PromptPreset) -> Result<()> {
    validate_label("name", &preset.name)?;
    if preset.blocks.len() > MAX_BLOCKS {
        return Err(PromptError::too_many("blocks", MAX_BLOCKS));
    }
    if preset.blocks.is_empty() {
        return Err(PromptError::invalid(
            "blocks",
            "at least one block is required",
        ));
    }

    let mut authored_bytes = 0usize;
    let mut active_chat_blocks = 0usize;
    let mut cache_blocks = 0usize;
    let mut enabled_main_seen = false;
    let mut enabled_global_note_seen = false;
    for (index, block) in preset.blocks.iter().enumerate() {
        validate_label(&format!("blocks[{index}].name"), block.name())?;
        if block.enabled() && matches!(block, PromptBlock::Chat { .. }) {
            active_chat_blocks += 1;
        }
        match block {
            PromptBlock::Raw {
                prompt,
                role,
                special,
                enabled,
                ..
            } => {
                validate_authored_text(
                    &format!("blocks[{index}].prompt"),
                    prompt,
                    false,
                    &mut authored_bytes,
                )?;
                validate_template_syntax(prompt, &format!("blocks[{index}].prompt"))?;
                if let Some(special) = special {
                    if *role != crate::PromptRole::System {
                        return Err(PromptError::invalid(
                            format!("blocks[{index}].role"),
                            "special raw prompts must use the system role",
                        ));
                    }
                    if *enabled {
                        let seen = match special {
                            crate::RawPromptSpecial::Main => &mut enabled_main_seen,
                            crate::RawPromptSpecial::GlobalNote => &mut enabled_global_note_seen,
                        };
                        if *seen {
                            return Err(PromptError::invalid(
                                format!("blocks[{index}].special"),
                                "an enabled raw special may appear only once",
                            ));
                        }
                        *seen = true;
                    }
                }
            }
            PromptBlock::FinalInsertion { prompt, .. } | PromptBlock::ChatMl { prompt, .. } => {
                validate_authored_text(
                    &format!("blocks[{index}].prompt"),
                    prompt,
                    false,
                    &mut authored_bytes,
                )?;
                validate_template_syntax(prompt, &format!("blocks[{index}].prompt"))?;
            }
            PromptBlock::Chat { selection, .. } => {
                if let crate::ChatEnd::Index(end) = selection.end
                    && selection.start >= end
                {
                    return Err(PromptError::invalid(
                        format!("blocks[{index}].selection"),
                        "chat range must be a non-empty half-open interval [start, end)",
                    ));
                }
            }
            PromptBlock::Persona { format, .. }
            | PromptBlock::CharacterDescription { format, .. }
            | PromptBlock::Lorebook { format, .. }
            | PromptBlock::LongTermMemory { format, .. } => {
                validate_content_format(format, index, &mut authored_bytes)?;
            }
            PromptBlock::AuthorNote {
                default_prompt,
                format,
                ..
            } => {
                if let Some(prompt) = default_prompt {
                    validate_authored_text(
                        &format!("blocks[{index}].defaultPrompt"),
                        prompt,
                        false,
                        &mut authored_bytes,
                    )?;
                    validate_template_syntax(prompt, &format!("blocks[{index}].defaultPrompt"))?;
                }
                validate_content_format(format, index, &mut authored_bytes)?;
            }
            PromptBlock::CachePoint { depth, .. } => {
                cache_blocks += 1;
                if *depth == 0 || *depth > 16 {
                    return Err(PromptError::invalid(
                        format!("blocks[{index}].depth"),
                        "cache depth must be in 1..=16",
                    ));
                }
            }
        }
    }
    if active_chat_blocks > 1 {
        return Err(PromptError::too_many("active chat blocks", 1));
    }
    if cache_blocks > 16 {
        return Err(PromptError::too_many("cache-point blocks", 16));
    }

    validate_sampling(&preset.sampling)?;
    validate_template_settings(preset, &mut authored_bytes)?;
    validate_references(preset)?;
    validate_regex_rules(preset, &mut authored_bytes)?;
    if authored_bytes > MAX_AUTHORED_TEXT_BYTES {
        return Err(PromptError::too_large(
            "authored prompt text",
            MAX_AUTHORED_TEXT_BYTES,
        ));
    }
    Ok(())
}

fn validate_template_settings(preset: &PromptPreset, authored_bytes: &mut usize) -> Result<()> {
    let settings = &preset.advanced.template;
    for (field, message) in [
        ("advanced.template.preamble", settings.preamble.as_ref()),
        ("advanced.template.epilogue", settings.epilogue.as_ref()),
    ] {
        if let Some(message) = message {
            validate_authored_text(field, &message.template, false, authored_bytes)?;
            validate_template_syntax(&message.template, field)?;
        }
    }
    if settings.default_variables.len() > MAX_VARIABLES {
        return Err(PromptError::too_many("defaultVariables", MAX_VARIABLES));
    }
    let mut variable_bytes = 0usize;
    for (name, value) in &settings.default_variables {
        if !validate_variable_name(name) || name == "value" {
            return Err(PromptError::invalid(
                "defaultVariables",
                "invalid or reserved variable name",
            ));
        }
        if value.contains('\0') {
            return Err(PromptError::invalid(
                format!("defaultVariables.{name}"),
                "must contain no NUL",
            ));
        }
        if value.len() > MAX_VARIABLE_VALUE_BYTES {
            return Err(PromptError::too_large(
                format!("defaultVariables.{name}"),
                MAX_VARIABLE_VALUE_BYTES,
            ));
        }
        variable_bytes = variable_bytes
            .checked_add(value.len())
            .ok_or_else(|| PromptError::too_large("defaultVariables", MAX_AUTHORED_TEXT_BYTES))?;
        add_authored(authored_bytes, value.len())?;
    }
    if variable_bytes > MAX_AUTHORED_TEXT_BYTES {
        return Err(PromptError::too_large(
            "defaultVariables",
            MAX_AUTHORED_TEXT_BYTES,
        ));
    }
    Ok(())
}

fn validate_content_format(
    format: &ContentFormat,
    index: usize,
    authored_bytes: &mut usize,
) -> Result<()> {
    if let ContentFormat::Custom { template } = format {
        let field = format!("blocks[{index}].format.template");
        validate_authored_text(&field, template, false, authored_bytes)?;
        let variables = validate_template_syntax(template, &field)?;
        if variables
            .iter()
            .filter(|name| name.as_str() == "value")
            .count()
            != 1
        {
            return Err(PromptError::invalid(
                field,
                "custom content format must contain ${value} exactly once",
            ));
        }
    }
    Ok(())
}

fn validate_sampling(sampling: &PromptSampling) -> Result<()> {
    validate_float("temperature", sampling.temperature, 0.0, 10.0, false)?;
    if sampling.top_k == Some(0) {
        return Err(PromptError::invalid("topK", "must be greater than zero"));
    }
    validate_float("minP", sampling.min_p, 0.0, 1.0, false)?;
    validate_float("topA", sampling.top_a, 0.0, 1.0, false)?;
    validate_float(
        "repetitionPenalty",
        sampling.repetition_penalty,
        0.0,
        2.0,
        true,
    )?;
    validate_float("topP", sampling.top_p, 0.0, 1.0, false)?;
    validate_float(
        "presencePenalty",
        sampling.presence_penalty,
        -2.0,
        2.0,
        false,
    )?;
    validate_float(
        "frequencyPenalty",
        sampling.frequency_penalty,
        -2.0,
        2.0,
        false,
    )?;
    if sampling.stop_sequences.len() > 16 {
        return Err(PromptError::too_many("stopSequences", 16));
    }
    for sequence in &sampling.stop_sequences {
        if sequence.is_empty() || sequence.len() > 256 || sequence.contains('\0') {
            return Err(PromptError::invalid(
                "stopSequences",
                "entries must be 1-256 bytes and contain no NUL",
            ));
        }
    }
    Ok(())
}

fn validate_float(
    field: &str,
    value: Option<f64>,
    min: f64,
    max: f64,
    exclusive_min: bool,
) -> Result<()> {
    if let Some(value) = value
        && (!value.is_finite() || value < min || value > max || (exclusive_min && value == min))
    {
        return Err(PromptError::invalid(
            field,
            format!("must be a finite value in the supported range {min}..={max}"),
        ));
    }
    Ok(())
}

fn validate_references(preset: &PromptPreset) -> Result<()> {
    if preset.advanced.tools.len() > MAX_REFERENCES {
        return Err(PromptError::too_many("tools", MAX_REFERENCES));
    }
    if preset.advanced.modules.len() > MAX_REFERENCES {
        return Err(PromptError::too_many("modules", MAX_REFERENCES));
    }

    let mut tool_ids = BTreeSet::new();
    for reference in &preset.advanced.tools {
        validate_reference_id("toolId", &reference.tool_id, true)?;
        if !tool_ids.insert(&reference.tool_id) {
            return Err(PromptError::invalid("tools", "duplicate tool reference"));
        }
    }

    let mut module_ids = BTreeSet::new();
    for reference in &preset.advanced.modules {
        validate_reference_id("moduleId", &reference.module_id, true)?;
        validate_reference_id("module.version", &reference.version, false)?;
        if reference.digest_sha256.len() != 64
            || !reference
                .digest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(PromptError::invalid(
                "module.digestSha256",
                "must be 64 lowercase hexadecimal characters",
            ));
        }
        if !module_ids.insert((&reference.module_id, &reference.version)) {
            return Err(PromptError::invalid(
                "modules",
                "duplicate module reference",
            ));
        }
    }
    Ok(())
}

fn validate_regex_rules(preset: &PromptPreset, authored_bytes: &mut usize) -> Result<()> {
    if preset.advanced.regex.len() > MAX_REGEX_RULES {
        return Err(PromptError::too_many("regex", MAX_REGEX_RULES));
    }
    if preset
        .advanced
        .regex
        .iter()
        .filter(|rule| rule.enabled)
        .count()
        > MAX_ACTIVE_REGEX_RULES
    {
        return Err(PromptError::too_many(
            "active regex rules",
            MAX_ACTIVE_REGEX_RULES,
        ));
    }
    let mut names = BTreeSet::new();
    for (index, rule) in preset.advanced.regex.iter().enumerate() {
        validate_label(&format!("regex[{index}].name"), &rule.name)?;
        if !names.insert(&rule.name) {
            return Err(PromptError::invalid("regex", "duplicate rule name"));
        }
        if rule.pattern.is_empty() || rule.pattern.len() > MAX_REGEX_PATTERN_BYTES {
            return Err(PromptError::invalid(
                format!("regex[{index}].pattern"),
                format!("must be 1-{MAX_REGEX_PATTERN_BYTES} bytes"),
            ));
        }
        if rule.replacement.len() > MAX_REGEX_REPLACEMENT_BYTES {
            return Err(PromptError::too_large(
                format!("regex[{index}].replacement"),
                MAX_REGEX_REPLACEMENT_BYTES,
            ));
        }
        add_authored(authored_bytes, rule.pattern.len())?;
        add_authored(authored_bytes, rule.replacement.len())?;
    }
    validate_regex_contract(preset)?;
    Ok(())
}

fn validate_label(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 128 || value.contains('\0') {
        return Err(PromptError::invalid(
            field,
            "must be 1-128 bytes after trimming and contain no NUL",
        ));
    }
    Ok(())
}

fn validate_reference_id(field: &str, value: &str, require_letter_first: bool) -> Result<()> {
    let first_is_valid = value.bytes().next().is_some_and(|byte| {
        if require_letter_first {
            byte.is_ascii_alphabetic()
        } else {
            byte.is_ascii_alphanumeric()
        }
    });
    if value.is_empty()
        || value.len() > 64
        || !first_is_valid
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(PromptError::invalid(
            field,
            "must be a safe 1-64 byte ASCII identifier without '..'",
        ));
    }
    Ok(())
}

fn validate_authored_text(
    field: &str,
    value: &str,
    allow_empty: bool,
    authored_bytes: &mut usize,
) -> Result<()> {
    if (!allow_empty && value.trim().is_empty()) || value.contains('\0') {
        return Err(PromptError::invalid(
            field,
            "must be non-empty and contain no NUL",
        ));
    }
    if value.len() > MAX_FIELD_TEXT_BYTES {
        return Err(PromptError::too_large(field, MAX_FIELD_TEXT_BYTES));
    }
    add_authored(authored_bytes, value.len())
}

fn add_authored(total: &mut usize, value: usize) -> Result<()> {
    *total = total
        .checked_add(value)
        .ok_or_else(|| PromptError::too_large("authored prompt text", MAX_AUTHORED_TEXT_BYTES))?;
    if *total > MAX_AUTHORED_TEXT_BYTES {
        return Err(PromptError::too_large(
            "authored prompt text",
            MAX_AUTHORED_TEXT_BYTES,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AdvancedSettings, ChatEnd, ChatSelection, PromptRole, RawPromptSpecial, ToolReference,
    };

    fn preset(blocks: Vec<PromptBlock>) -> PromptPreset {
        PromptPreset {
            name: "test".to_owned(),
            blocks,
            sampling: PromptSampling::default(),
            advanced: AdvancedSettings::default(),
        }
    }

    #[test]
    fn rejects_multiple_active_chat_blocks() {
        let chat = || PromptBlock::Chat {
            name: "history".to_owned(),
            enabled: true,
            selection: ChatSelection {
                start: 0,
                end: ChatEnd::EndOfChat,
            },
        };
        assert!(validate_preset(&preset(vec![chat(), chat()])).is_err());
    }

    #[test]
    fn custom_format_requires_exactly_one_value_slot() {
        let block = PromptBlock::Persona {
            name: "persona".to_owned(),
            enabled: true,
            role: PromptRole::System,
            format: ContentFormat::Custom {
                template: "${value}:${value}".to_owned(),
            },
        };
        assert!(validate_preset(&preset(vec![block])).is_err());
    }

    #[test]
    fn raw_special_requires_system_role_and_is_unique_when_enabled() {
        let special = |role, name: &str| PromptBlock::Raw {
            name: name.to_owned(),
            enabled: true,
            role,
            special: Some(RawPromptSpecial::Main),
            prompt: "text".to_owned(),
        };
        assert!(validate_preset(&preset(vec![special(PromptRole::User, "wrong")])).is_err());
        assert!(
            validate_preset(&preset(vec![
                special(PromptRole::System, "one"),
                special(PromptRole::System, "two"),
            ]))
            .is_err()
        );
    }

    #[test]
    fn tool_reference_matches_runtime_name_contract() {
        let base = PromptBlock::Raw {
            name: "main".to_owned(),
            enabled: true,
            role: PromptRole::User,
            special: None,
            prompt: "hello".to_owned(),
        };
        for invalid in ["1tool", "a..b", &"a".repeat(65)] {
            let mut value = preset(vec![base.clone()]);
            value.advanced.tools.push(ToolReference {
                tool_id: invalid.to_owned(),
                enabled: false,
            });
            assert!(validate_preset(&value).is_err(), "accepted {invalid}");
        }

        let mut valid = preset(vec![base]);
        valid.advanced.tools.push(ToolReference {
            tool_id: "web.search-v1".to_owned(),
            enabled: false,
        });
        validate_preset(&valid).unwrap();
    }
}
