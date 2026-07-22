use unicode_normalization::UnicodeNormalization;

/// Deterministic locale-independent search normalization.
///
/// The pipeline is NFKC, Unicode scalar lowercase (when case-insensitive),
/// then collapse every Unicode whitespace run to one ASCII space and trim.
/// It intentionally does not perform language-specific stemming: a Korean
/// key still matches before an attached 조사 because matching is substring
/// based, while spelling and word boundaries are never guessed.
#[must_use]
pub fn normalize_search_text(value: &str, case_sensitive: bool) -> String {
    let compatible = value.nfkc().collect::<String>();
    let cased = if case_sensitive {
        compatible
    } else {
        compatible.chars().flat_map(char::to_lowercase).collect()
    };

    let mut normalized = String::with_capacity(cased.len());
    let mut pending_space = false;
    for character in cased.chars() {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
        } else {
            if pending_space {
                normalized.push(' ');
                pending_space = false;
            }
            normalized.push(character);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_compatibility_case_and_whitespace_aware() {
        assert_eq!(
            normalize_search_text("  ＬＯＲＥ\n\tBook  ", false),
            "lore book"
        );
        assert_eq!(normalize_search_text("Ａbc", true), "Abc");
    }
}
