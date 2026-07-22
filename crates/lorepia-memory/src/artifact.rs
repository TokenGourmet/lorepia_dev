use std::collections::BTreeSet;

use lorepia_persona::{CharacterCardId, ChatId};
use serde::Serialize;

use crate::{
    EmbeddingProfileRef, MAX_EMBEDDING_DIMENSIONS, MAX_MEMORY_ARTIFACT_BYTES,
    MAX_MEMORY_CANDIDATES, MemoryArtifactId, MemoryError, MemoryPresetId, MemorySessionBinding,
    MemoryStrategy, ModelPresetRef, ProcessedSummary, PromptPresetRef, Result, SummaryJobKind,
    SummaryJobSource,
};

const SIMILARITY_SCALE: i32 = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryArtifactKind {
    Leaf,
    Consolidated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryValidity {
    Attached,
    SourceCompacted,
    InvalidatedByEdit,
    InvalidatedByDeletion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryArtifact {
    id: MemoryArtifactId,
    revision: u64,
    chat_id: ChatId,
    character_card_id: CharacterCardId,
    memory_generation: u64,
    memory_preset_id: MemoryPresetId,
    memory_preset_revision: u64,
    prompt_preset: PromptPresetRef,
    summary_model: ModelPresetRef,
    kind: SummaryArtifactKind,
    first_source_sequence: u64,
    last_source_sequence: u64,
    covered_leaf_artifact_ids: Vec<MemoryArtifactId>,
    content: String,
    validity: MemoryValidity,
}

impl SummaryArtifact {
    pub fn new(
        binding: &MemorySessionBinding,
        expected_chat_id: &ChatId,
        expected_character_card_id: &CharacterCardId,
        processed: ProcessedSummary,
        id: MemoryArtifactId,
    ) -> Result<Self> {
        binding.verify_scope(expected_chat_id, expected_character_card_id)?;
        let (job, content) = processed.into_parts();
        job.verify_binding(binding)?;
        if !job.requires_generation() {
            return Err(MemoryError::invalid(
                "summaryJob.source",
                "a filtered-empty summary job cannot produce an artifact",
            ));
        }
        validate_artifact_content(&content)?;
        let (kind, covered_leaf_artifact_ids) = match (job.kind(), job.source()) {
            (SummaryJobKind::Initial, SummaryJobSource::Messages(_)) => {
                (SummaryArtifactKind::Leaf, vec![id.clone()])
            }
            (SummaryJobKind::Resummary, SummaryJobSource::Artifacts(artifacts)) => {
                let mut covered = BTreeSet::new();
                for artifact in artifacts {
                    for leaf_id in &artifact.covered_leaf_artifact_ids {
                        covered.insert(leaf_id.clone());
                        if covered.len() > MAX_MEMORY_CANDIDATES {
                            return Err(MemoryError::too_many(
                                "coveredLeafArtifactIds",
                                MAX_MEMORY_CANDIDATES,
                            ));
                        }
                    }
                }
                if covered.is_empty() {
                    return Err(MemoryError::invalid(
                        "coveredLeafArtifactIds",
                        "a consolidated summary must cover at least one leaf",
                    ));
                }
                (
                    SummaryArtifactKind::Consolidated,
                    covered.into_iter().collect(),
                )
            }
            _ => {
                return Err(MemoryError::invalid(
                    "summaryJob.source",
                    "summary job kind and source type differ",
                ));
            }
        };
        Ok(Self {
            id,
            revision: 1,
            chat_id: binding.chat_id().clone(),
            character_card_id: binding.character_card_id().clone(),
            memory_generation: binding.memory_generation(),
            memory_preset_id: binding.preset().id().clone(),
            memory_preset_revision: binding.preset().revision(),
            prompt_preset: binding.preset().prompt_preset().clone(),
            summary_model: binding.resolved_summary_model().clone(),
            kind,
            first_source_sequence: job.first_source_sequence(),
            last_source_sequence: job.last_source_sequence(),
            covered_leaf_artifact_ids,
            content,
            validity: MemoryValidity::Attached,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &MemoryArtifactId {
        &self.id
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }

    #[must_use]
    pub const fn character_card_id(&self) -> &CharacterCardId {
        &self.character_card_id
    }

    #[must_use]
    pub const fn memory_generation(&self) -> u64 {
        self.memory_generation
    }

    #[must_use]
    pub const fn memory_preset_id(&self) -> &MemoryPresetId {
        &self.memory_preset_id
    }

    #[must_use]
    pub const fn memory_preset_revision(&self) -> u64 {
        self.memory_preset_revision
    }

    #[must_use]
    pub const fn prompt_preset(&self) -> &PromptPresetRef {
        &self.prompt_preset
    }

    #[must_use]
    pub const fn summary_model(&self) -> &ModelPresetRef {
        &self.summary_model
    }

    #[must_use]
    pub const fn kind(&self) -> SummaryArtifactKind {
        self.kind
    }

    #[must_use]
    pub const fn first_source_sequence(&self) -> u64 {
        self.first_source_sequence
    }

    #[must_use]
    pub const fn last_source_sequence(&self) -> u64 {
        self.last_source_sequence
    }

    #[must_use]
    pub fn covered_leaf_artifact_ids(&self) -> &[MemoryArtifactId] {
        &self.covered_leaf_artifact_ids
    }

    #[must_use]
    pub fn covers_leaf(&self, id: &MemoryArtifactId) -> bool {
        self.covered_leaf_artifact_ids.binary_search(id).is_ok()
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    #[must_use]
    pub const fn validity(&self) -> MemoryValidity {
        self.validity
    }

    #[must_use]
    pub fn source_compacted(mut self) -> Self {
        if self.validity == MemoryValidity::Attached {
            self.validity = MemoryValidity::SourceCompacted;
        }
        self
    }

    pub fn invalidated_by_edit(mut self) -> Result<Self> {
        self.bump_revision()?;
        self.validity = MemoryValidity::InvalidatedByEdit;
        self.content.clear();
        Ok(self)
    }

    pub fn invalidated_by_deletion(mut self) -> Result<Self> {
        self.bump_revision()?;
        self.validity = MemoryValidity::InvalidatedByDeletion;
        self.content.clear();
        Ok(self)
    }

    fn bump_revision(&mut self) -> Result<()> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(MemoryError::RevisionOverflow)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingVector {
    profile: EmbeddingProfileRef,
    values: Vec<f32>,
}

impl EmbeddingVector {
    pub fn new(profile: EmbeddingProfileRef, values: Vec<f32>) -> Result<Self> {
        if values.is_empty() || values.len() > MAX_EMBEDDING_DIMENSIONS {
            return Err(MemoryError::invalid(
                "embedding.values",
                format!("dimension must be in 1..={MAX_EMBEDDING_DIMENSIONS}"),
            ));
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err(MemoryError::invalid(
                "embedding.values",
                "all values must be finite",
            ));
        }
        let norm = values
            .iter()
            .map(|value| f64::from(*value) * f64::from(*value))
            .sum::<f64>();
        if norm == 0.0 {
            return Err(MemoryError::invalid(
                "embedding.values",
                "zero-norm vectors are not comparable",
            ));
        }
        Ok(Self { profile, values })
    }

    #[must_use]
    pub const fn profile(&self) -> &EmbeddingProfileRef {
        &self.profile
    }

    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SimilarityScore(i32);

impl SimilarityScore {
    pub(crate) fn from_quantized(value: i32) -> Result<Self> {
        if !(-SIMILARITY_SCALE..=SIMILARITY_SCALE).contains(&value) {
            return Err(MemoryError::invalid(
                "similarityScore",
                "must be in -1000000..=1000000",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn quantized(self) -> i32 {
        self.0
    }
}

pub fn cosine_similarity(
    query: &EmbeddingVector,
    candidate: &EmbeddingVector,
) -> Result<SimilarityScore> {
    if query.profile != candidate.profile {
        return Err(MemoryError::RetrievalUnavailable(
            "embedding profile ID or revision differs",
        ));
    }
    if query.values.len() != candidate.values.len() {
        return Err(MemoryError::RetrievalUnavailable(
            "embedding dimensions differ",
        ));
    }
    let mut dot = 0.0f64;
    let mut query_norm = 0.0f64;
    let mut candidate_norm = 0.0f64;
    for (left, right) in query.values.iter().zip(&candidate.values) {
        let left = f64::from(*left);
        let right = f64::from(*right);
        dot += left * right;
        query_norm += left * left;
        candidate_norm += right * right;
    }
    let cosine = (dot / (query_norm.sqrt() * candidate_norm.sqrt())).clamp(-1.0, 1.0);
    let quantized = (cosine * f64::from(SIMILARITY_SCALE)).round() as i32;
    SimilarityScore::from_quantized(quantized)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingArtifact {
    memory_artifact_id: MemoryArtifactId,
    memory_artifact_revision: u64,
    chat_id: ChatId,
    character_card_id: CharacterCardId,
    memory_generation: u64,
    memory_preset_id: MemoryPresetId,
    memory_preset_revision: u64,
    prompt_preset: PromptPresetRef,
    summary_model: ModelPresetRef,
    vector: EmbeddingVector,
}

impl EmbeddingArtifact {
    pub fn new(
        binding: &MemorySessionBinding,
        expected_chat_id: &ChatId,
        expected_character_card_id: &CharacterCardId,
        artifact: &SummaryArtifact,
        vector: EmbeddingVector,
    ) -> Result<Self> {
        binding.verify_scope(expected_chat_id, expected_character_card_id)?;
        if artifact.chat_id != *binding.chat_id()
            || artifact.character_card_id != *binding.character_card_id()
            || artifact.memory_generation != binding.memory_generation()
            || artifact.memory_preset_id != *binding.preset().id()
            || artifact.memory_preset_revision != binding.preset().revision()
            || artifact.prompt_preset != *binding.preset().prompt_preset()
            || artifact.summary_model != *binding.resolved_summary_model()
        {
            return Err(MemoryError::RetrievalUnavailable(
                "summary artifact is outside the active memory session",
            ));
        }
        if matches!(
            artifact.validity,
            MemoryValidity::InvalidatedByEdit | MemoryValidity::InvalidatedByDeletion
        ) {
            return Err(MemoryError::RetrievalUnavailable(
                "invalidated summary artifacts must not be embedded",
            ));
        }
        if artifact.validity == MemoryValidity::SourceCompacted
            && !binding.preset().preserve_compacted_orphans()
        {
            return Err(MemoryError::RetrievalUnavailable(
                "compacted-source artifacts are disabled by this memory preset",
            ));
        }
        let MemoryStrategy::HybridRetrieval(policy) = binding.preset().strategy() else {
            return Err(MemoryError::RetrievalUnavailable(
                "rolling-summary mode has no embedding profile",
            ));
        };
        if vector.profile != *policy.embedding_profile() {
            return Err(MemoryError::RetrievalUnavailable(
                "vector is outside the pinned embedding space",
            ));
        }
        Ok(Self {
            memory_artifact_id: artifact.id.clone(),
            memory_artifact_revision: artifact.revision,
            chat_id: artifact.chat_id.clone(),
            character_card_id: artifact.character_card_id.clone(),
            memory_generation: artifact.memory_generation,
            memory_preset_id: artifact.memory_preset_id.clone(),
            memory_preset_revision: artifact.memory_preset_revision,
            prompt_preset: artifact.prompt_preset.clone(),
            summary_model: artifact.summary_model.clone(),
            vector,
        })
    }

    #[must_use]
    pub const fn memory_artifact_id(&self) -> &MemoryArtifactId {
        &self.memory_artifact_id
    }

    #[must_use]
    pub const fn memory_artifact_revision(&self) -> u64 {
        self.memory_artifact_revision
    }

    #[must_use]
    pub const fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }

    #[must_use]
    pub const fn character_card_id(&self) -> &CharacterCardId {
        &self.character_card_id
    }

    #[must_use]
    pub const fn memory_generation(&self) -> u64 {
        self.memory_generation
    }

    #[must_use]
    pub const fn memory_preset_id(&self) -> &MemoryPresetId {
        &self.memory_preset_id
    }

    #[must_use]
    pub const fn memory_preset_revision(&self) -> u64 {
        self.memory_preset_revision
    }

    #[must_use]
    pub const fn prompt_preset(&self) -> &PromptPresetRef {
        &self.prompt_preset
    }

    #[must_use]
    pub const fn summary_model(&self) -> &ModelPresetRef {
        &self.summary_model
    }

    #[must_use]
    pub const fn vector(&self) -> &EmbeddingVector {
        &self.vector
    }
}

fn validate_artifact_content(content: &str) -> Result<()> {
    if content.trim().is_empty() {
        return Err(MemoryError::invalid(
            "summaryArtifact.content",
            "must not be empty",
        ));
    }
    if content.len() > MAX_MEMORY_ARTIFACT_BYTES {
        return Err(MemoryError::too_large(
            "summaryArtifact.content",
            MAX_MEMORY_ARTIFACT_BYTES,
        ));
    }
    if content.contains('\0') {
        return Err(MemoryError::invalid(
            "summaryArtifact.content",
            "must contain no NUL",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{EmbeddingProfileId, EmbeddingProfileRef};

    use super::*;

    #[test]
    fn cosine_is_quantized_and_embedding_spaces_do_not_mix() {
        let profile =
            EmbeddingProfileRef::new(EmbeddingProfileId::parse("embedding").unwrap(), 1).unwrap();
        let query = EmbeddingVector::new(profile.clone(), vec![1.0, 0.0]).unwrap();
        let same = EmbeddingVector::new(profile, vec![1.0, 0.0]).unwrap();
        assert_eq!(
            cosine_similarity(&query, &same).unwrap().quantized(),
            1_000_000
        );

        let other = EmbeddingVector::new(
            EmbeddingProfileRef::new(EmbeddingProfileId::parse("embedding").unwrap(), 2).unwrap(),
            vec![1.0, 0.0],
        )
        .unwrap();
        assert!(cosine_similarity(&query, &other).is_err());
    }

    #[test]
    fn invalid_vectors_fail_closed() {
        let profile =
            EmbeddingProfileRef::new(EmbeddingProfileId::parse("embedding").unwrap(), 1).unwrap();
        assert!(EmbeddingVector::new(profile.clone(), vec![0.0, 0.0]).is_err());
        assert!(EmbeddingVector::new(profile, vec![f32::NAN]).is_err());
    }
}
