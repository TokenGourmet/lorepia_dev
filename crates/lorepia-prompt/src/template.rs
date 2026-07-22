use std::collections::BTreeMap;

use crate::{ContentFormat, PromptError, Result};

pub(crate) const MAX_RENDERED_TEXT_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn validate_variable_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != b'_' {
        return false;
    }
    name.len() <= 64
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

pub(crate) fn validate_template_syntax(template: &str, field: &str) -> Result<Vec<String>> {
    let mut variables = Vec::new();
    let bytes = template.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor] != b'$' || bytes.get(cursor + 1) != Some(&b'{') {
            cursor += 1;
            continue;
        }
        let name_start = cursor + 2;
        let Some(relative_end) = bytes[name_start..].iter().position(|byte| *byte == b'}') else {
            return Err(PromptError::invalid(field, "unterminated ${name} variable"));
        };
        let name_end = name_start + relative_end;
        let name = std::str::from_utf8(&bytes[name_start..name_end])
            .map_err(|_| PromptError::invalid(field, "variable name is not UTF-8"))?;
        if !validate_variable_name(name) {
            return Err(PromptError::invalid(
                field,
                "variable names must be 1-64 ASCII letters, digits, '_', '.' or '-' and cannot start with a digit",
            ));
        }
        variables.push(name.to_owned());
        cursor = name_end + 1;
    }
    Ok(variables)
}

pub(crate) fn render_template(
    template: &str,
    variables: &BTreeMap<String, String>,
) -> Result<String> {
    render_with_reserved(template, variables, None)
}

pub(crate) fn render_content(
    format: &ContentFormat,
    source: &str,
    variables: &BTreeMap<String, String>,
) -> Result<String> {
    match format {
        ContentFormat::Plain => {
            if source.len() > MAX_RENDERED_TEXT_BYTES {
                return Err(PromptError::too_large(
                    "rendered content",
                    MAX_RENDERED_TEXT_BYTES,
                ));
            }
            Ok(source.to_owned())
        }
        ContentFormat::Custom { template } => {
            render_with_reserved(template, variables, Some(("value", source)))
        }
    }
}

fn render_with_reserved(
    template: &str,
    variables: &BTreeMap<String, String>,
    reserved: Option<(&str, &str)>,
) -> Result<String> {
    let bytes = template.as_bytes();
    let mut output = String::with_capacity(template.len().min(MAX_RENDERED_TEXT_BYTES));
    let mut literal_start = 0usize;
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        if bytes[cursor] != b'$' || bytes.get(cursor + 1) != Some(&b'{') {
            cursor += 1;
            continue;
        }

        append_bounded(&mut output, &template[literal_start..cursor])?;
        let name_start = cursor + 2;
        let Some(relative_end) = bytes[name_start..].iter().position(|byte| *byte == b'}') else {
            return Err(PromptError::invalid(
                "template",
                "unterminated ${name} variable",
            ));
        };
        let name_end = name_start + relative_end;
        let name = &template[name_start..name_end];
        if !validate_variable_name(name) {
            return Err(PromptError::invalid("template", "invalid variable name"));
        }
        let value = reserved
            .filter(|(reserved_name, _)| *reserved_name == name)
            .map(|(_, value)| value)
            .or_else(|| variables.get(name).map(String::as_str))
            .ok_or_else(|| PromptError::UnknownVariable(name.to_owned()))?;
        append_bounded(&mut output, value)?;
        cursor = name_end + 1;
        literal_start = cursor;
    }

    append_bounded(&mut output, &template[literal_start..])?;
    Ok(output)
}

fn append_bounded(output: &mut String, value: &str) -> Result<()> {
    let next = output
        .len()
        .checked_add(value.len())
        .ok_or_else(|| PromptError::too_large("rendered content", MAX_RENDERED_TEXT_BYTES))?;
    if next > MAX_RENDERED_TEXT_BYTES {
        return Err(PromptError::too_large(
            "rendered content",
            MAX_RENDERED_TEXT_BYTES,
        ));
    }
    output.push_str(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_is_one_pass() {
        let variables = BTreeMap::from([
            ("name".to_owned(), "${nested}".to_owned()),
            ("nested".to_owned(), "must-not-expand".to_owned()),
        ]);
        assert_eq!(
            render_template("Hello ${name}", &variables).unwrap(),
            "Hello ${nested}"
        );
    }

    #[test]
    fn unknown_variable_is_not_silently_empty() {
        let error = render_template("${missing}", &BTreeMap::new()).unwrap_err();
        assert!(matches!(error, PromptError::UnknownVariable(name) if name == "missing"));
    }

    #[test]
    fn content_value_overrides_same_named_runtime_variable() {
        let variables = BTreeMap::from([("value".to_owned(), "attacker".to_owned())]);
        let format = ContentFormat::Custom {
            template: "before ${value} after".to_owned(),
        };
        assert_eq!(
            render_content(&format, "trusted source", &variables).unwrap(),
            "before trusted source after"
        );
    }
}
