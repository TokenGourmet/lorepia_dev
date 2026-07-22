use regex::{Regex, RegexBuilder};
use serde::Serialize;

use crate::{
    BASIS_POINTS_SCALE, EmbeddingProfileRef, MAX_CHUNK_SEPARATOR_REGEX_BYTES,
    MAX_MEMORY_LABEL_BYTES, MAX_MESSAGES_PER_SUMMARY, MAX_QUERY_MESSAGE_COUNT,
    MAX_SUMMARY_PROMPT_BYTES, MemoryError, MemoryPresetId, ModelPresetRef, PromptPresetRef, Result,
};

const REGEX_SIZE_LIMIT_BYTES: usize = 128 * 1024;
const REGEX_DFA_SIZE_LIMIT_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BasisPoints(u16);

impl BasisPoints {
    pub fn new(value: u16) -> Result<Self> {
        if value > BASIS_POINTS_SCALE {
            return Err(MemoryError::invalid(
                "basisPoints",
                format!("must be in 0..={BASIS_POINTS_SCALE}"),
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMix {
    recent: BasisPoints,
    similar: BasisPoints,
    random: BasisPoints,
}

impl MemoryMix {
    pub fn new(recent: BasisPoints, similar: BasisPoints, random: BasisPoints) -> Result<Self> {
        let sum = u32::from(recent.get())
            .checked_add(u32::from(similar.get()))
            .and_then(|value| value.checked_add(u32::from(random.get())))
            .ok_or_else(|| MemoryError::invalid("memoryMix", "ratio sum overflowed"))?;
        if sum != u32::from(BASIS_POINTS_SCALE) {
            return Err(MemoryError::invalid(
                "memoryMix",
                format!("recent, similar, and random must sum to {BASIS_POINTS_SCALE}"),
            ));
        }
        Ok(Self {
            recent,
            similar,
            random,
        })
    }

    #[must_use]
    pub const fn recent(self) -> BasisPoints {
        self.recent
    }

    #[must_use]
    pub const fn similar(self) -> BasisPoints {
        self.similar
    }

    #[must_use]
    pub const fn random(self) -> BasisPoints {
        self.random
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ChunkSeparatorRegex(String);

impl ChunkSeparatorRegex {
    pub fn new(pattern: impl Into<String>) -> Result<Self> {
        let pattern = pattern.into();
        compile_chunk_separator(&pattern)?;
        Ok(Self(pattern))
    }

    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.0
    }

    pub(crate) fn compile(&self) -> Result<Regex> {
        compile_chunk_separator(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingQueryScope {
    Conversation,
    AssistantOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegenerationRegexPolicy {
    Skip,
    ApplyPinnedPromptResponseRules,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "preset", rename_all = "snake_case")]
pub enum SummaryModelRef {
    ActiveChatModel,
    ModelPreset(ModelPresetRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryPolicy {
    summary_prompt: String,
    resummary_prompt: String,
    max_messages_per_summary: u16,
    skip_user_messages: bool,
    chunk_separator: ChunkSeparatorRegex,
}

impl SummaryPolicy {
    pub fn new(
        summary_prompt: impl Into<String>,
        resummary_prompt: impl Into<String>,
        max_messages_per_summary: u16,
        skip_user_messages: bool,
        chunk_separator: ChunkSeparatorRegex,
    ) -> Result<Self> {
        let value = Self {
            summary_prompt: summary_prompt.into(),
            resummary_prompt: resummary_prompt.into(),
            max_messages_per_summary,
            skip_user_messages,
            chunk_separator,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<()> {
        validate_authored_prompt("summaryPrompt", &self.summary_prompt)?;
        validate_authored_prompt("resummaryPrompt", &self.resummary_prompt)?;
        if self.max_messages_per_summary == 0
            || self.max_messages_per_summary > MAX_MESSAGES_PER_SUMMARY
        {
            return Err(MemoryError::invalid(
                "maxMessagesPerSummary",
                format!("must be in 1..={MAX_MESSAGES_PER_SUMMARY}"),
            ));
        }
        self.chunk_separator.compile()?;
        Ok(())
    }

    #[must_use]
    pub fn summary_prompt(&self) -> &str {
        &self.summary_prompt
    }

    #[must_use]
    pub fn resummary_prompt(&self) -> &str {
        &self.resummary_prompt
    }

    #[must_use]
    pub const fn max_messages_per_summary(&self) -> u16 {
        self.max_messages_per_summary
    }

    #[must_use]
    pub const fn skip_user_messages(&self) -> bool {
        self.skip_user_messages
    }

    #[must_use]
    pub const fn chunk_separator(&self) -> &ChunkSeparatorRegex {
        &self.chunk_separator
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertionPolicy {
    memory_budget: BasisPoints,
    additional_detail_budget: BasisPoints,
}

impl InsertionPolicy {
    pub fn new(memory_budget: BasisPoints, additional_detail_budget: BasisPoints) -> Result<Self> {
        if memory_budget.get() == 0 {
            return Err(MemoryError::invalid(
                "memoryBudgetBps",
                "must be greater than zero",
            ));
        }
        Ok(Self {
            memory_budget,
            additional_detail_budget,
        })
    }

    #[must_use]
    pub const fn memory_budget(self) -> BasisPoints {
        self.memory_budget
    }

    #[must_use]
    pub const fn additional_detail_budget(self) -> BasisPoints {
        self.additional_detail_budget
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridRetrievalPolicy {
    query_message_count: u16,
    query_scope: EmbeddingQueryScope,
    selection_mix: MemoryMix,
    embedding_profile: EmbeddingProfileRef,
}

impl HybridRetrievalPolicy {
    pub fn new(
        query_message_count: u16,
        query_scope: EmbeddingQueryScope,
        selection_mix: MemoryMix,
        embedding_profile: EmbeddingProfileRef,
    ) -> Result<Self> {
        if query_message_count == 0 || query_message_count > MAX_QUERY_MESSAGE_COUNT {
            return Err(MemoryError::invalid(
                "queryMessageCount",
                format!("must be in 1..={MAX_QUERY_MESSAGE_COUNT}"),
            ));
        }
        if selection_mix.similar().get() == 0 {
            return Err(MemoryError::invalid(
                "selectionMix.similar",
                "hybrid retrieval requires a non-zero similar-memory share",
            ));
        }
        Ok(Self {
            query_message_count,
            query_scope,
            selection_mix,
            embedding_profile,
        })
    }

    #[must_use]
    pub const fn query_message_count(&self) -> u16 {
        self.query_message_count
    }

    #[must_use]
    pub const fn query_scope(&self) -> EmbeddingQueryScope {
        self.query_scope
    }

    #[must_use]
    pub const fn selection_mix(&self) -> MemoryMix {
        self.selection_mix
    }

    #[must_use]
    pub const fn embedding_profile(&self) -> &EmbeddingProfileRef {
        &self.embedding_profile
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "retrieval", rename_all = "snake_case")]
pub enum MemoryStrategy {
    RollingSummary,
    HybridRetrieval(HybridRetrievalPolicy),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPresetDraft {
    pub label: String,
    pub strategy: MemoryStrategy,
    pub prompt_preset: PromptPresetRef,
    pub summary_model: SummaryModelRef,
    pub summary: SummaryPolicy,
    pub insertion: InsertionPolicy,
    pub preserve_compacted_orphans: bool,
    pub regeneration_regex: RegenerationRegexPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPreset {
    id: MemoryPresetId,
    revision: u64,
    label: String,
    strategy: MemoryStrategy,
    prompt_preset: PromptPresetRef,
    summary_model: SummaryModelRef,
    summary: SummaryPolicy,
    insertion: InsertionPolicy,
    preserve_compacted_orphans: bool,
    regeneration_regex: RegenerationRegexPolicy,
}

impl MemoryPreset {
    pub fn create(id: MemoryPresetId, draft: MemoryPresetDraft) -> Result<Self> {
        Self::from_parts(id, 1, draft)
    }

    pub fn updated(&self, expected_revision: u64, draft: MemoryPresetDraft) -> Result<Self> {
        if expected_revision != self.revision {
            return Err(MemoryError::RevisionConflict {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(MemoryError::RevisionOverflow)?;
        Self::from_parts(self.id.clone(), revision, draft)
    }

    pub(crate) fn from_parts(
        id: MemoryPresetId,
        revision: u64,
        draft: MemoryPresetDraft,
    ) -> Result<Self> {
        if revision == 0 {
            return Err(MemoryError::invalid("revision", "must be at least 1"));
        }
        validate_label(&draft.label)?;
        draft.summary.validate()?;
        Ok(Self {
            id,
            revision,
            label: draft.label,
            strategy: draft.strategy,
            prompt_preset: draft.prompt_preset,
            summary_model: draft.summary_model,
            summary: draft.summary,
            insertion: draft.insertion,
            preserve_compacted_orphans: draft.preserve_compacted_orphans,
            regeneration_regex: draft.regeneration_regex,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &MemoryPresetId {
        &self.id
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn strategy(&self) -> &MemoryStrategy {
        &self.strategy
    }

    #[must_use]
    pub const fn prompt_preset(&self) -> &PromptPresetRef {
        &self.prompt_preset
    }

    #[must_use]
    pub const fn summary_model(&self) -> &SummaryModelRef {
        &self.summary_model
    }

    #[must_use]
    pub const fn summary(&self) -> &SummaryPolicy {
        &self.summary
    }

    #[must_use]
    pub const fn insertion(&self) -> InsertionPolicy {
        self.insertion
    }

    #[must_use]
    pub const fn preserve_compacted_orphans(&self) -> bool {
        self.preserve_compacted_orphans
    }

    #[must_use]
    pub const fn regeneration_regex(&self) -> RegenerationRegexPolicy {
        self.regeneration_regex
    }
}

pub(crate) fn validate_authored_prompt(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(MemoryError::invalid(field, "must not be empty"));
    }
    if value.len() > MAX_SUMMARY_PROMPT_BYTES {
        return Err(MemoryError::too_large(field, MAX_SUMMARY_PROMPT_BYTES));
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(MemoryError::invalid(
            field,
            "must contain no disallowed control characters",
        ));
    }
    Ok(())
}

fn validate_label(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_MEMORY_LABEL_BYTES {
        return Err(MemoryError::invalid(
            "label",
            format!("must be 1-{MAX_MEMORY_LABEL_BYTES} bytes"),
        ));
    }
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(MemoryError::invalid(
            "label",
            "must have no surrounding whitespace or control characters",
        ));
    }
    Ok(())
}

fn compile_chunk_separator(pattern: &str) -> Result<Regex> {
    if pattern.is_empty() || pattern.len() > MAX_CHUNK_SEPARATOR_REGEX_BYTES {
        return Err(MemoryError::invalid(
            "chunkSeparatorRegex",
            format!("must be 1-{MAX_CHUNK_SEPARATOR_REGEX_BYTES} bytes"),
        ));
    }
    let regex = RegexBuilder::new(pattern)
        .unicode(true)
        .size_limit(REGEX_SIZE_LIMIT_BYTES)
        .dfa_size_limit(REGEX_DFA_SIZE_LIMIT_BYTES)
        .build()
        .map_err(|_| MemoryError::Regex("pattern is outside the bounded Rust regex contract"))?;
    if regex.is_match("") {
        return Err(MemoryError::Regex(
            "separator must not match an empty string",
        ));
    }
    Ok(regex)
}

#[cfg(test)]
mod tests {
    use crate::{EmbeddingProfileId, PromptPresetId};

    use super::*;

    #[test]
    fn mix_is_integer_and_exact() {
        assert!(
            MemoryMix::new(
                BasisPoints::new(4_000).unwrap(),
                BasisPoints::new(4_000).unwrap(),
                BasisPoints::new(2_000).unwrap(),
            )
            .is_ok()
        );
        assert!(
            MemoryMix::new(
                BasisPoints::new(4_000).unwrap(),
                BasisPoints::new(4_000).unwrap(),
                BasisPoints::new(1_999).unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn hybrid_requires_similarity_and_a_versioned_embedding_profile() {
        let profile =
            EmbeddingProfileRef::new(EmbeddingProfileId::parse("embedding.main").unwrap(), 1)
                .unwrap();
        let mix = MemoryMix::new(
            BasisPoints::new(10_000).unwrap(),
            BasisPoints::new(0).unwrap(),
            BasisPoints::new(0).unwrap(),
        )
        .unwrap();
        assert!(
            HybridRetrievalPolicy::new(8, EmbeddingQueryScope::Conversation, mix, profile,)
                .is_err()
        );
    }

    #[test]
    fn chunk_separator_rejects_empty_matching_patterns() {
        assert!(ChunkSeparatorRegex::new("^|").is_err());
        assert!(ChunkSeparatorRegex::new(r"\n{2,}").is_ok());
    }

    #[test]
    fn prompt_preset_reference_contains_no_prompt_or_secret() {
        let reference = PromptPresetRef::new(PromptPresetId::parse("summary").unwrap(), 1).unwrap();
        let json = serde_json::to_string(&reference).unwrap();
        assert_eq!(json, r#"{"id":"summary","revision":1}"#);
    }
}
