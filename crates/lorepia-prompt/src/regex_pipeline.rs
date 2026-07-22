use regex::{Captures, Regex, RegexBuilder};

use crate::{PromptError, PromptPreset, Result, TransformTarget};

const MAX_PATTERN_BYTES: usize = 16 * 1024;
const MAX_REPLACEMENT_BYTES: usize = 64 * 1024;
const MAX_RULES: usize = 128;
const MAX_ACTIVE_RULES: usize = 64;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_CUMULATIVE_SCAN_BYTES: usize = 128 * 1024 * 1024;
const MAX_MATCHES_PER_RULE: usize = 100_000;
const MAX_CUMULATIVE_MATCHES: usize = 100_000;
const REGEX_SIZE_LIMIT_BYTES: usize = 128 * 1024;
const REGEX_DFA_SIZE_LIMIT_BYTES: usize = 128 * 1024;
const MAX_ACTIVE_REGEX_ENGINE_BUDGET_BYTES: usize = 16 * 1024 * 1024;
const _: () = assert!(
    MAX_ACTIVE_RULES * (REGEX_SIZE_LIMIT_BYTES + REGEX_DFA_SIZE_LIMIT_BYTES)
        <= MAX_ACTIVE_REGEX_ENGINE_BUDGET_BYTES
);

/// Applies enabled text transforms for `target` in preset order.
pub fn apply_text_transforms(
    preset: &PromptPreset,
    target: TransformTarget,
    input: &str,
) -> Result<String> {
    CompiledTransformPipeline::new(preset, target)?.apply(input)
}

#[derive(Debug, Default)]
struct TransformBudget {
    scanned_bytes: usize,
    matches: usize,
}

struct CompiledRule<'a> {
    label: String,
    regex: Regex,
    replacement: Vec<ReplacementPart<'a>>,
    global: bool,
    matches: usize,
}

/// A logical request compiles each active rule once and shares one scan budget
/// across every message it transforms.
pub(crate) struct CompiledTransformPipeline<'a> {
    rules: Vec<CompiledRule<'a>>,
    budget: TransformBudget,
}

impl<'a> CompiledTransformPipeline<'a> {
    pub(crate) fn new(preset: &'a PromptPreset, target: TransformTarget) -> Result<Self> {
        validate_rule_table(preset)?;
        let mut rules = Vec::new();
        for (index, rule) in preset.advanced.regex.iter().enumerate() {
            if !rule.enabled || rule.target != target {
                continue;
            }
            let label = rule_label(index, &rule.name);
            rules.push(CompiledRule {
                regex: compile_regex(&label, &rule.pattern, rule.flags)?,
                replacement: parse_replacement(&label, &rule.replacement)?,
                global: rule.flags.global,
                matches: 0,
                label,
            });
        }
        Ok(Self {
            rules,
            budget: TransformBudget::default(),
        })
    }

    pub(crate) fn apply(&mut self, input: &str) -> Result<String> {
        ensure_text_size("regex.input", input.len())?;

        let mut current = input.to_owned();
        for rule in &mut self.rules {
            self.budget.charge(current.len())?;
            current = apply_rule(
                &rule.label,
                &rule.regex,
                &rule.replacement,
                rule.global,
                &current,
                &mut rule.matches,
                &mut self.budget,
            )?;
        }
        Ok(current)
    }
}

/// Validates every stored rule, including disabled imported rules.
///
/// Disabled means "not authorized to run", not "untrusted syntax may bypass
/// validation". This keeps later local approval from activating a rule that
/// was never parsed under the bounded Rust-regex contract.
pub(crate) fn validate_regex_contract(preset: &PromptPreset) -> Result<()> {
    validate_rule_table(preset)?;
    for (index, rule) in preset.advanced.regex.iter().enumerate() {
        let label = rule_label(index, &rule.name);
        let _ = compile_regex(&label, &rule.pattern, rule.flags)?;
        let _ = parse_replacement(&label, &rule.replacement)?;
    }
    Ok(())
}

impl TransformBudget {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.scanned_bytes = self.scanned_bytes.checked_add(bytes).ok_or_else(|| {
            PromptError::too_large("regex.cumulativeScan", MAX_CUMULATIVE_SCAN_BYTES)
        })?;
        if self.scanned_bytes > MAX_CUMULATIVE_SCAN_BYTES {
            return Err(PromptError::too_large(
                "regex.cumulativeScan",
                MAX_CUMULATIVE_SCAN_BYTES,
            ));
        }
        Ok(())
    }

    fn charge_match(&mut self) -> Result<()> {
        self.matches = self.matches.checked_add(1).ok_or_else(|| {
            PromptError::too_many("regex.cumulativeMatches", MAX_CUMULATIVE_MATCHES)
        })?;
        if self.matches > MAX_CUMULATIVE_MATCHES {
            return Err(PromptError::too_many(
                "regex.cumulativeMatches",
                MAX_CUMULATIVE_MATCHES,
            ));
        }
        Ok(())
    }
}

fn validate_rule_table(preset: &PromptPreset) -> Result<()> {
    let rules = &preset.advanced.regex;
    if rules.len() > MAX_RULES {
        return Err(PromptError::too_many("advanced.regex", MAX_RULES));
    }

    let active_count = rules.iter().filter(|rule| rule.enabled).count();
    if active_count > MAX_ACTIVE_RULES {
        return Err(PromptError::too_many(
            "advanced.regex.enabled",
            MAX_ACTIVE_RULES,
        ));
    }

    for (index, rule) in rules.iter().enumerate() {
        if rule.pattern.len() > MAX_PATTERN_BYTES {
            return Err(PromptError::too_large(
                format!("advanced.regex[{index}].pattern"),
                MAX_PATTERN_BYTES,
            ));
        }
        if rule.replacement.len() > MAX_REPLACEMENT_BYTES {
            return Err(PromptError::too_large(
                format!("advanced.regex[{index}].replacement"),
                MAX_REPLACEMENT_BYTES,
            ));
        }
    }

    Ok(())
}

fn ensure_text_size(field: &str, bytes: usize) -> Result<()> {
    if bytes > MAX_TEXT_BYTES {
        return Err(PromptError::too_large(field, MAX_TEXT_BYTES));
    }
    Ok(())
}

fn compile_regex(label: &str, pattern: &str, flags: crate::RegexFlags) -> Result<Regex> {
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(flags.case_insensitive)
        .multi_line(flags.multi_line)
        .unicode(flags.unicode)
        .dot_matches_new_line(flags.dot_matches_new_line)
        .size_limit(REGEX_SIZE_LIMIT_BYTES)
        .dfa_size_limit(REGEX_DFA_SIZE_LIMIT_BYTES)
        .build()
        .map_err(|_| PromptError::Regex {
            rule: label.to_owned(),
            reason: "pattern rejected by the bounded Rust regex contract".to_owned(),
        })?;
    if regex.captures_len() > 64 {
        return Err(PromptError::Regex {
            rule: label.to_owned(),
            reason: "pattern exceeds 63 capture groups plus the whole match".to_owned(),
        });
    }
    Ok(regex)
}

#[derive(Debug, Eq, PartialEq)]
enum ReplacementPart<'a> {
    Literal(&'a str),
    Dollar,
    Capture(usize),
}

fn parse_replacement<'a>(label: &str, replacement: &'a str) -> Result<Vec<ReplacementPart<'a>>> {
    let bytes = replacement.as_bytes();
    let mut parts = Vec::new();
    let mut cursor = 0;
    let mut literal_start = 0;

    while cursor < bytes.len() {
        if bytes[cursor] != b'$' {
            cursor += 1;
            continue;
        }

        if literal_start < cursor {
            parts.push(ReplacementPart::Literal(
                &replacement[literal_start..cursor],
            ));
        }

        match bytes.get(cursor + 1) {
            Some(b'$') => {
                parts.push(ReplacementPart::Dollar);
                cursor += 2;
            }
            Some(b'{') => {
                let digits_start = cursor + 2;
                let Some(relative_end) =
                    bytes[digits_start..].iter().position(|byte| *byte == b'}')
                else {
                    return replacement_error(label, "unterminated capture reference");
                };
                let digits_end = digits_start + relative_end;
                let digits = &replacement[digits_start..digits_end];
                if digits.is_empty()
                    || !digits.bytes().all(|byte| byte.is_ascii_digit())
                    || (digits.len() > 1 && digits.starts_with('0'))
                {
                    return replacement_error(
                        label,
                        "capture references must use canonical `${0}` through `${63}` syntax",
                    );
                }
                let capture = digits.parse::<usize>().map_err(|_| PromptError::Regex {
                    rule: label.to_owned(),
                    reason: "capture reference is outside `${0}` through `${63}`".to_owned(),
                })?;
                if capture > 63 {
                    return replacement_error(
                        label,
                        "capture reference is outside `${0}` through `${63}`",
                    );
                }
                parts.push(ReplacementPart::Capture(capture));
                cursor = digits_end + 1;
            }
            _ => {
                return replacement_error(
                    label,
                    "only `$$` and `${0}` through `${63}` are allowed after `$`",
                );
            }
        }

        literal_start = cursor;
    }

    if literal_start < replacement.len() {
        parts.push(ReplacementPart::Literal(&replacement[literal_start..]));
    }
    Ok(parts)
}

fn replacement_error<T>(label: &str, reason: &str) -> Result<T> {
    Err(PromptError::Regex {
        rule: label.to_owned(),
        reason: format!("replacement rejected: {reason}"),
    })
}

fn apply_rule(
    label: &str,
    regex: &Regex,
    replacement: &[ReplacementPart<'_>],
    global: bool,
    input: &str,
    rule_match_count: &mut usize,
    budget: &mut TransformBudget,
) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut copied_until = 0;
    let mut match_count = 0usize;

    for captures in regex.captures_iter(input) {
        budget.charge_match()?;
        match_count = match_count
            .checked_add(1)
            .ok_or_else(|| PromptError::TooManyItems {
                field: format!("regex.{label}.matches"),
                max: MAX_MATCHES_PER_RULE,
            })?;
        if match_count > MAX_MATCHES_PER_RULE {
            return Err(PromptError::too_many(
                format!("regex.{label}.matches"),
                MAX_MATCHES_PER_RULE,
            ));
        }
        *rule_match_count = rule_match_count.checked_add(1).ok_or_else(|| {
            PromptError::too_many(format!("regex.{label}.matches"), MAX_MATCHES_PER_RULE)
        })?;
        if *rule_match_count > MAX_MATCHES_PER_RULE {
            return Err(PromptError::too_many(
                format!("regex.{label}.matches"),
                MAX_MATCHES_PER_RULE,
            ));
        }

        let whole = captures.get(0).ok_or_else(|| PromptError::Regex {
            rule: label.to_owned(),
            reason: "regex engine returned a capture without the whole match".to_owned(),
        })?;
        append_bounded(&mut output, &input[copied_until..whole.start()])?;
        expand_replacement(&mut output, replacement, &captures)?;
        copied_until = whole.end();

        if !global {
            break;
        }
    }

    append_bounded(&mut output, &input[copied_until..])?;
    Ok(output)
}

fn expand_replacement(
    output: &mut String,
    replacement: &[ReplacementPart<'_>],
    captures: &Captures<'_>,
) -> Result<()> {
    for part in replacement {
        match part {
            ReplacementPart::Literal(value) => append_bounded(output, value)?,
            ReplacementPart::Dollar => append_bounded(output, "$")?,
            ReplacementPart::Capture(index) => {
                if let Some(value) = captures.get(*index) {
                    append_bounded(output, value.as_str())?;
                }
            }
        }
    }
    Ok(())
}

fn append_bounded(output: &mut String, value: &str) -> Result<()> {
    let next_len = output
        .len()
        .checked_add(value.len())
        .ok_or_else(|| PromptError::too_large("regex.output", MAX_TEXT_BYTES))?;
    if next_len > MAX_TEXT_BYTES {
        return Err(PromptError::too_large("regex.output", MAX_TEXT_BYTES));
    }
    output.push_str(value);
    Ok(())
}

fn rule_label(index: usize, name: &str) -> String {
    if !name.is_empty() && name.len() <= 256 {
        name.to_owned()
    } else {
        format!("rule[{index}]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdvancedSettings, PromptSampling, RegexFlags, RegexRule};

    fn preset_with_rules(regex: Vec<RegexRule>) -> PromptPreset {
        PromptPreset {
            name: "test".to_owned(),
            blocks: Vec::new(),
            sampling: PromptSampling::default(),
            advanced: AdvancedSettings {
                regex,
                ..AdvancedSettings::default()
            },
        }
    }

    fn rule(pattern: &str, replacement: &str) -> RegexRule {
        RegexRule {
            name: "rule".to_owned(),
            enabled: true,
            target: TransformTarget::Request,
            pattern: pattern.to_owned(),
            replacement: replacement.to_owned(),
            flags: RegexFlags::default(),
        }
    }

    #[test]
    fn applies_matching_rules_in_order_and_ignores_other_targets() {
        let mut other_target = rule("wrong", "used");
        other_target.target = TransformTarget::Response;
        let mut disabled = rule("wrong", "used");
        disabled.enabled = false;
        let preset = preset_with_rules(vec![
            rule("cat", "dog"),
            rule("dog", "fox"),
            other_target,
            disabled,
        ]);

        let output = apply_text_transforms(&preset, TransformTarget::Request, "cat cat").unwrap();

        assert_eq!(output, "fox fox");
    }

    #[test]
    fn supports_flags_and_non_global_mode() {
        let mut configured = rule("^a.b", "hit");
        configured.flags = RegexFlags {
            global: false,
            case_insensitive: true,
            multi_line: true,
            unicode: true,
            dot_matches_new_line: true,
        };
        let preset = preset_with_rules(vec![configured]);

        let output =
            apply_text_transforms(&preset, TransformTarget::Request, "A\nB\na\nb").unwrap();

        assert_eq!(output, "hit\na\nb");
    }

    #[test]
    fn unicode_flag_can_select_ascii_character_classes() {
        let mut configured = rule(r"\w+", "x");
        configured.flags.unicode = false;
        let preset = preset_with_rules(vec![configured]);

        let output = apply_text_transforms(&preset, TransformTarget::Request, "éclair").unwrap();

        assert_eq!(output, "éx");
    }

    #[test]
    fn expands_only_bounded_numeric_captures_and_literal_dollars() {
        let preset = preset_with_rules(vec![rule(
            "(?P<ignored>[a-z]+)-([0-9]+)(?:-(x))?",
            "${2}:$${1}:${3}:${63}:${0}",
        )]);

        let output = apply_text_transforms(&preset, TransformTarget::Request, "abc-42").unwrap();

        assert_eq!(output, "42:${1}:::abc-42");
    }

    #[test]
    fn rejects_unbounded_or_ambiguous_replacement_forms() {
        for replacement in ["$1", "$name", "${name}", "${64}", "${01}", "${1"] {
            let preset = preset_with_rules(vec![rule("x", replacement)]);
            assert!(matches!(
                apply_text_transforms(&preset, TransformTarget::Request, "x"),
                Err(PromptError::Regex { .. })
            ));
        }
    }

    #[test]
    fn enforces_rule_and_active_rule_counts() {
        let too_many = preset_with_rules(
            (0..=MAX_RULES)
                .map(|_| {
                    let mut value = rule("x", "y");
                    value.enabled = false;
                    value
                })
                .collect(),
        );
        assert!(matches!(
            apply_text_transforms(&too_many, TransformTarget::Request, "x"),
            Err(PromptError::TooManyItems { max: MAX_RULES, .. })
        ));

        let too_many_active =
            preset_with_rules((0..=MAX_ACTIVE_RULES).map(|_| rule("x", "y")).collect());
        assert!(matches!(
            apply_text_transforms(&too_many_active, TransformTarget::Request, "x"),
            Err(PromptError::TooManyItems {
                max: MAX_ACTIVE_RULES,
                ..
            })
        ));
    }

    #[test]
    fn enforces_pattern_replacement_and_input_limits() {
        let oversized_pattern =
            preset_with_rules(vec![rule(&"x".repeat(MAX_PATTERN_BYTES + 1), "")]);
        assert!(matches!(
            apply_text_transforms(&oversized_pattern, TransformTarget::Request, "x"),
            Err(PromptError::PayloadTooLarge {
                max_bytes: MAX_PATTERN_BYTES,
                ..
            })
        ));

        let oversized_replacement =
            preset_with_rules(vec![rule("x", &"y".repeat(MAX_REPLACEMENT_BYTES + 1))]);
        assert!(matches!(
            apply_text_transforms(&oversized_replacement, TransformTarget::Request, "x"),
            Err(PromptError::PayloadTooLarge {
                max_bytes: MAX_REPLACEMENT_BYTES,
                ..
            })
        ));

        let empty = preset_with_rules(Vec::new());
        assert!(matches!(
            apply_text_transforms(
                &empty,
                TransformTarget::Request,
                &"x".repeat(MAX_TEXT_BYTES + 1),
            ),
            Err(PromptError::PayloadTooLarge {
                max_bytes: MAX_TEXT_BYTES,
                ..
            })
        ));
    }

    #[test]
    fn stops_before_output_can_exceed_limit() {
        let preset = preset_with_rules(vec![rule("x", &"y".repeat(MAX_REPLACEMENT_BYTES))]);

        assert!(matches!(
            apply_text_transforms(&preset, TransformTarget::Request, &"x".repeat(65)),
            Err(PromptError::PayloadTooLarge {
                max_bytes: MAX_TEXT_BYTES,
                ..
            })
        ));
    }

    #[test]
    fn limits_matches_per_rule() {
        let preset = preset_with_rules(vec![rule("x", "")]);

        assert!(matches!(
            apply_text_transforms(
                &preset,
                TransformTarget::Request,
                &"x".repeat(MAX_MATCHES_PER_RULE + 1),
            ),
            Err(PromptError::TooManyItems {
                max: MAX_MATCHES_PER_RULE,
                ..
            })
        ));
    }

    #[test]
    fn shared_budget_covers_multiple_inputs() {
        let preset = preset_with_rules(vec![rule("x", "x")]);
        let mut pipeline = CompiledTransformPipeline::new(&preset, TransformTarget::Request)
            .expect("pipeline should compile once");
        pipeline.budget = TransformBudget {
            scanned_bytes: MAX_CUMULATIVE_SCAN_BYTES - 3,
            ..TransformBudget::default()
        };

        assert_eq!(pipeline.apply("xx").unwrap(), "xx");
        assert!(matches!(
            pipeline.apply("xx"),
            Err(PromptError::PayloadTooLarge {
                max_bytes: MAX_CUMULATIVE_SCAN_BYTES,
                ..
            })
        ));
    }

    #[test]
    fn disabled_rules_still_require_safe_syntax() {
        let mut invalid = rule("(?=lookahead)", "$1");
        invalid.enabled = false;
        let preset = preset_with_rules(vec![invalid]);

        assert!(validate_regex_contract(&preset).is_err());
    }

    #[test]
    fn capture_group_count_is_bounded() {
        let pattern = "()".repeat(64);
        let preset = preset_with_rules(vec![rule(&pattern, "")]);
        assert!(apply_text_transforms(&preset, TransformTarget::Request, "").is_err());
    }

    #[test]
    fn configured_active_engine_limits_fit_the_mobile_budget() {
        let rules = (0..MAX_ACTIVE_RULES)
            .map(|index| {
                let mut value = rule(&format!("x{index}"), "y");
                value.name = format!("rule-{index}");
                value
            })
            .collect();
        let preset = preset_with_rules(rules);
        let pipeline = CompiledTransformPipeline::new(&preset, TransformTarget::Request)
            .expect("the maximum number of small rules should fit");
        assert_eq!(pipeline.rules.len(), MAX_ACTIVE_RULES);
    }

    #[test]
    fn one_compiled_pipeline_is_reused_for_many_inputs() {
        let preset = preset_with_rules(vec![rule("x", "y")]);
        let mut pipeline = CompiledTransformPipeline::new(&preset, TransformTarget::Request)
            .expect("pipeline should compile");
        assert_eq!(pipeline.rules.len(), 1);
        for _ in 0..10_000 {
            assert_eq!(pipeline.apply("x").unwrap(), "y");
        }
    }

    #[test]
    fn match_work_is_bounded_across_many_messages() {
        let preset = preset_with_rules(vec![rule("^", "")]);
        let mut pipeline = CompiledTransformPipeline::new(&preset, TransformTarget::Request)
            .expect("pipeline should compile");
        for _ in 0..MAX_CUMULATIVE_MATCHES {
            pipeline.apply("").unwrap();
        }
        assert!(matches!(
            pipeline.apply(""),
            Err(PromptError::TooManyItems {
                max: MAX_CUMULATIVE_MATCHES,
                ..
            })
        ));
    }
}
