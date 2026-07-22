use std::collections::{BTreeMap, BTreeSet};

use lorepia_persona::{CharacterCardId, ChatId};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    EmbeddingArtifact, EmbeddingProfileRef, EmbeddingQueryPlan, EmbeddingVector,
    MAX_LONG_TERM_MEMORY_INPUT_BYTES, MAX_MEMORY_CANDIDATES, MAX_QUERY_MESSAGE_COUNT,
    MemoryArtifactId, MemoryBudgetInput, MemoryError, MemoryPresetId, MemorySessionBinding,
    MemorySourceMessage, MemoryStrategy, MemoryTokenBudget, MemoryValidity, MessageId, Result,
    SimilarityScore, SummaryArtifact, SummaryArtifactKind, cosine_similarity,
};

const RANDOM_ORDER_DOMAIN: &[u8] = b"lorepia-memory-random-order-v1";

#[derive(Clone, Copy)]
pub enum AuthoritativeEmbeddingQuery<'a> {
    RollingSummary,
    Hybrid {
        plan: &'a EmbeddingQueryPlan,
        messages: &'a [MemorySourceMessage],
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingMatch {
    memory_artifact_id: MemoryArtifactId,
    memory_artifact_revision: u64,
    chat_id: ChatId,
    character_card_id: CharacterCardId,
    memory_generation: u64,
    memory_preset_id: MemoryPresetId,
    memory_preset_revision: u64,
    embedding_profile: EmbeddingProfileRef,
    query_message_ids: Vec<MessageId>,
    query_snapshot_digest: [u8; 32],
    score: SimilarityScore,
}

impl EmbeddingMatch {
    pub fn new(
        binding: &MemorySessionBinding,
        expected_chat_id: &ChatId,
        expected_character_card_id: &CharacterCardId,
        query_plan: &EmbeddingQueryPlan,
        current_query_messages: &[MemorySourceMessage],
        query_vector: &EmbeddingVector,
        candidate: &EmbeddingArtifact,
    ) -> Result<Self> {
        binding.verify_scope(expected_chat_id, expected_character_card_id)?;
        query_plan.verify_authoritative_messages(binding, current_query_messages)?;
        let MemoryStrategy::HybridRetrieval(policy) = binding.preset().strategy() else {
            return Err(MemoryError::RetrievalUnavailable(
                "rolling-summary mode has no embedding match",
            ));
        };
        if query_vector.profile() != policy.embedding_profile()
            || candidate.chat_id() != binding.chat_id()
            || candidate.character_card_id() != binding.character_card_id()
            || candidate.memory_generation() != binding.memory_generation()
            || candidate.memory_preset_id() != binding.preset().id()
            || candidate.memory_preset_revision() != binding.preset().revision()
            || candidate.prompt_preset() != binding.preset().prompt_preset()
            || candidate.summary_model() != binding.resolved_summary_model()
            || candidate.vector().profile() != policy.embedding_profile()
        {
            return Err(MemoryError::RetrievalUnavailable(
                "embedding evidence is outside the active session snapshot",
            ));
        }
        let query_message_ids = query_plan
            .messages()
            .iter()
            .map(|message| message.id().clone())
            .collect();
        Ok(Self {
            memory_artifact_id: candidate.memory_artifact_id().clone(),
            memory_artifact_revision: candidate.memory_artifact_revision(),
            chat_id: binding.chat_id().clone(),
            character_card_id: binding.character_card_id().clone(),
            memory_generation: binding.memory_generation(),
            memory_preset_id: binding.preset().id().clone(),
            memory_preset_revision: binding.preset().revision(),
            embedding_profile: policy.embedding_profile().clone(),
            query_message_ids,
            query_snapshot_digest: *query_plan.message_snapshot_digest(),
            score: cosine_similarity(query_vector, candidate.vector())?,
        })
    }

    #[must_use]
    pub const fn score(&self) -> SimilarityScore {
        self.score
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryCandidate {
    artifact: SummaryArtifact,
    reserved_prompt_tokens: u32,
    embedding_match: Option<EmbeddingMatch>,
}

impl MemoryCandidate {
    /// `reserved_prompt_tokens` is a conservative reservation for this item,
    /// including any insertion framing. The final compiled request must still
    /// pass the provider-specific exact-token gate.
    pub fn new(
        artifact: SummaryArtifact,
        reserved_prompt_tokens: u32,
        embedding_match: Option<EmbeddingMatch>,
    ) -> Result<Self> {
        if reserved_prompt_tokens == 0 {
            return Err(MemoryError::invalid(
                "memoryCandidate.reservedPromptTokens",
                "must be at least 1",
            ));
        }
        if let Some(evidence) = &embedding_match
            && (evidence.memory_artifact_id != *artifact.id()
                || evidence.memory_artifact_revision != artifact.revision())
        {
            return Err(MemoryError::RetrievalUnavailable(
                "embedding evidence does not belong to the memory artifact revision",
            ));
        }
        Ok(Self {
            artifact,
            reserved_prompt_tokens,
            embedding_match,
        })
    }

    #[must_use]
    pub const fn artifact(&self) -> &SummaryArtifact {
        &self.artifact
    }

    #[must_use]
    pub const fn reserved_prompt_tokens(&self) -> u32 {
        self.reserved_prompt_tokens
    }

    #[must_use]
    pub const fn similarity(&self) -> Option<SimilarityScore> {
        match &self.embedding_match {
            Some(evidence) => Some(evidence.score),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySelectionBucket {
    Consolidated,
    Recent,
    Similar,
    Random,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedMemory {
    artifact_id: MemoryArtifactId,
    artifact_revision: u64,
    first_source_sequence: u64,
    last_source_sequence: u64,
    bucket: MemorySelectionBucket,
    reserved_prompt_tokens: u32,
}

impl SelectedMemory {
    #[must_use]
    pub const fn artifact_id(&self) -> &MemoryArtifactId {
        &self.artifact_id
    }

    #[must_use]
    pub const fn artifact_revision(&self) -> u64 {
        self.artifact_revision
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
    pub const fn bucket(&self) -> MemorySelectionBucket {
        self.bucket
    }

    #[must_use]
    pub const fn reserved_prompt_tokens(&self) -> u32 {
        self.reserved_prompt_tokens
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySelection {
    chat_id: ChatId,
    character_card_id: CharacterCardId,
    memory_generation: u64,
    memory_preset_id: MemoryPresetId,
    memory_preset_revision: u64,
    budget_input: MemoryBudgetInput,
    query_snapshot_digest: Option<[u8; 32]>,
    selected: Vec<SelectedMemory>,
    reserved_prompt_tokens: u32,
}

impl MemorySelection {
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
    pub const fn budget_input(&self) -> MemoryBudgetInput {
        self.budget_input
    }

    #[must_use]
    pub fn selected(&self) -> &[SelectedMemory] {
        &self.selected
    }

    #[must_use]
    pub const fn reserved_prompt_tokens(&self) -> u32 {
        self.reserved_prompt_tokens
    }

    /// Resolves IDs against a current authoritative artifact snapshot before
    /// copying any content. A deleted, edited, or revision-changed artifact
    /// makes the whole selection stale.
    pub fn materialize(
        &self,
        binding: &MemorySessionBinding,
        expected_chat_id: &ChatId,
        expected_character_card_id: &CharacterCardId,
        expected_budget_input: MemoryBudgetInput,
        query: AuthoritativeEmbeddingQuery<'_>,
        current_artifacts: &[SummaryArtifact],
    ) -> Result<String> {
        self.verify_application_context(
            binding,
            expected_chat_id,
            expected_character_card_id,
            expected_budget_input,
            query,
        )?;
        materialize_current_artifacts(binding, &self.selected, current_artifacts)
    }

    fn verify_application_context(
        &self,
        binding: &MemorySessionBinding,
        expected_chat_id: &ChatId,
        expected_character_card_id: &CharacterCardId,
        expected_budget_input: MemoryBudgetInput,
        query: AuthoritativeEmbeddingQuery<'_>,
    ) -> Result<()> {
        binding.verify_scope(expected_chat_id, expected_character_card_id)?;
        let (_, query_snapshot_digest) = validate_query_context(binding, query)?;
        if self.chat_id != *binding.chat_id()
            || self.character_card_id != *binding.character_card_id()
            || self.memory_generation != binding.memory_generation()
            || self.memory_preset_id != *binding.preset().id()
            || self.memory_preset_revision != binding.preset().revision()
            || self.budget_input != expected_budget_input
            || self.query_snapshot_digest != query_snapshot_digest
        {
            return Err(MemoryError::RetrievalUnavailable(
                "memory selection is outside the active session snapshot",
            ));
        }
        Ok(())
    }
}

pub fn select_memories(
    binding: &MemorySessionBinding,
    expected_chat_id: &ChatId,
    expected_character_card_id: &CharacterCardId,
    budget_input: MemoryBudgetInput,
    query: AuthoritativeEmbeddingQuery<'_>,
    candidates: &[MemoryCandidate],
) -> Result<MemorySelection> {
    binding.verify_scope(expected_chat_id, expected_character_card_id)?;
    let budget = MemoryTokenBudget::calculate(binding.preset(), budget_input);
    let (query_message_ids, query_snapshot_digest) = validate_query_context(binding, query)?;
    if candidates.len() > MAX_MEMORY_CANDIDATES {
        return Err(MemoryError::too_many(
            "memoryCandidates",
            MAX_MEMORY_CANDIDATES,
        ));
    }

    let mut candidate_ids = BTreeSet::new();
    for candidate in candidates {
        verify_candidate_scope(
            binding,
            &query_message_ids,
            query_snapshot_digest.as_ref(),
            candidate,
        )?;
        if !candidate_ids.insert(candidate.artifact.id().clone()) {
            return Err(MemoryError::DuplicateId {
                field: "memoryArtifactId",
                id: candidate.artifact.id().to_string(),
            });
        }
    }

    let eligible = candidates
        .iter()
        .filter(|candidate| candidate_is_eligible(binding, candidate))
        .collect::<Vec<_>>();
    let mut consolidated = eligible
        .iter()
        .copied()
        .filter(|candidate| {
            candidate.artifact.kind() == SummaryArtifactKind::Consolidated
                && candidate.reserved_prompt_tokens <= budget.consolidated_summary()
        })
        .collect::<Vec<_>>();
    consolidated.sort_by(|left, right| {
        right
            .artifact
            .last_source_sequence()
            .cmp(&left.artifact.last_source_sequence())
            .then_with(|| left.artifact.id().cmp(right.artifact.id()))
    });

    let mut selected = Vec::new();
    let mut selected_ids = BTreeSet::new();
    let consolidated_artifact = consolidated.first().map(|candidate| {
        push_selected(
            &mut selected,
            &mut selected_ids,
            candidate,
            MemorySelectionBucket::Consolidated,
        );
        &candidate.artifact
    });

    let leaves = eligible
        .into_iter()
        .filter(|candidate| candidate.artifact.kind() == SummaryArtifactKind::Leaf)
        .filter(|candidate| {
            consolidated_artifact
                .is_none_or(|summary| !summary.covers_leaf(candidate.artifact.id()))
        })
        .collect::<Vec<_>>();

    let mut recent = leaves.clone();
    recent.sort_by(|left, right| {
        right
            .artifact
            .last_source_sequence()
            .cmp(&left.artifact.last_source_sequence())
            .then_with(|| left.artifact.id().cmp(right.artifact.id()))
    });
    select_bucket(
        &recent,
        budget.retrieval().recent(),
        MemorySelectionBucket::Recent,
        &mut selected,
        &mut selected_ids,
    );

    if matches!(
        binding.preset().strategy(),
        MemoryStrategy::HybridRetrieval(_)
    ) {
        if budget.retrieval().similar() > 0
            && leaves
                .iter()
                .any(|candidate| candidate.embedding_match.is_none())
        {
            return Err(MemoryError::RetrievalUnavailable(
                "hybrid retrieval requires a similarity score for every eligible leaf",
            ));
        }

        if budget.retrieval().similar() > 0 {
            let mut similar = leaves.clone();
            similar.sort_by(|left, right| {
                right
                    .similarity()
                    .expect("similarity was checked")
                    .cmp(&left.similarity().expect("similarity was checked"))
                    .then_with(|| left.artifact.id().cmp(right.artifact.id()))
            });
            select_bucket(
                &similar,
                budget.retrieval().similar(),
                MemorySelectionBucket::Similar,
                &mut selected,
                &mut selected_ids,
            );
        }

        let mut random = leaves;
        random.sort_by_cached_key(|candidate| {
            deterministic_random_key(
                binding,
                query_snapshot_digest
                    .as_ref()
                    .expect("hybrid retrieval has a query snapshot"),
                candidate.artifact.id(),
            )
        });
        select_bucket(
            &random,
            budget.retrieval().random(),
            MemorySelectionBucket::Random,
            &mut selected,
            &mut selected_ids,
        );
    }

    selected.sort_by(|left, right| {
        left.first_source_sequence
            .cmp(&right.first_source_sequence)
            .then_with(|| left.last_source_sequence.cmp(&right.last_source_sequence))
            .then_with(|| left.artifact_id.cmp(&right.artifact_id))
    });
    let reserved_prompt_tokens = selected
        .iter()
        .map(SelectedMemory::reserved_prompt_tokens)
        .try_fold(0u32, u32::checked_add)
        .ok_or_else(|| MemoryError::invalid("memorySelection", "token reservation overflowed"))?;
    if reserved_prompt_tokens > budget.total_memory() {
        return Err(MemoryError::invalid(
            "memorySelection",
            "selected reservations exceed the total memory budget",
        ));
    }

    Ok(MemorySelection {
        chat_id: binding.chat_id().clone(),
        character_card_id: binding.character_card_id().clone(),
        memory_generation: binding.memory_generation(),
        memory_preset_id: binding.preset().id().clone(),
        memory_preset_revision: binding.preset().revision(),
        budget_input,
        query_snapshot_digest,
        selected,
        reserved_prompt_tokens,
    })
}

fn validate_query_context(
    binding: &MemorySessionBinding,
    query: AuthoritativeEmbeddingQuery<'_>,
) -> Result<(Vec<MessageId>, Option<[u8; 32]>)> {
    let MemoryStrategy::HybridRetrieval(_) = binding.preset().strategy() else {
        if !matches!(query, AuthoritativeEmbeddingQuery::RollingSummary) {
            return Err(MemoryError::RetrievalUnavailable(
                "rolling-summary mode must not receive an embedding query",
            ));
        }
        return Ok((Vec::new(), None));
    };
    let AuthoritativeEmbeddingQuery::Hybrid {
        plan: query_plan,
        messages: current_query_messages,
    } = query
    else {
        return Err(MemoryError::RetrievalUnavailable(
            "hybrid retrieval requires an authoritative embedding query plan",
        ));
    };
    query_plan.verify_authoritative_messages(binding, current_query_messages)?;
    let query_message_ids = query_plan
        .messages()
        .iter()
        .map(|message| message.id().clone())
        .collect::<Vec<_>>();
    if query_message_ids.len() > usize::from(MAX_QUERY_MESSAGE_COUNT) {
        return Err(MemoryError::too_many(
            "queryMessageIds",
            usize::from(MAX_QUERY_MESSAGE_COUNT),
        ));
    }
    if query_message_ids.is_empty() {
        return Err(MemoryError::RetrievalUnavailable(
            "hybrid retrieval requires a pinned query-message snapshot",
        ));
    }
    let mut ids = BTreeSet::new();
    for id in &query_message_ids {
        if !ids.insert(id) {
            return Err(MemoryError::DuplicateId {
                field: "queryMessageId",
                id: id.to_string(),
            });
        }
    }
    Ok((
        query_message_ids,
        Some(*query_plan.message_snapshot_digest()),
    ))
}

fn verify_candidate_scope(
    binding: &MemorySessionBinding,
    query_message_ids: &[MessageId],
    query_snapshot_digest: Option<&[u8; 32]>,
    candidate: &MemoryCandidate,
) -> Result<()> {
    verify_artifact_scope(binding, &candidate.artifact)?;
    if let Some(evidence) = &candidate.embedding_match {
        let MemoryStrategy::HybridRetrieval(policy) = binding.preset().strategy() else {
            return Err(MemoryError::RetrievalUnavailable(
                "rolling-summary mode must not consume embedding evidence",
            ));
        };
        if evidence.memory_artifact_id != *candidate.artifact.id()
            || evidence.memory_artifact_revision != candidate.artifact.revision()
            || evidence.chat_id != *binding.chat_id()
            || evidence.character_card_id != *binding.character_card_id()
            || evidence.memory_generation != binding.memory_generation()
            || evidence.memory_preset_id != *binding.preset().id()
            || evidence.memory_preset_revision != binding.preset().revision()
            || evidence.embedding_profile != *policy.embedding_profile()
            || evidence.query_message_ids != query_message_ids
            || Some(&evidence.query_snapshot_digest) != query_snapshot_digest
        {
            return Err(MemoryError::RetrievalUnavailable(
                "embedding evidence is stale or belongs to another query snapshot",
            ));
        }
    }
    Ok(())
}

fn verify_artifact_scope(binding: &MemorySessionBinding, artifact: &SummaryArtifact) -> Result<()> {
    if artifact.chat_id() != binding.chat_id()
        || artifact.character_card_id() != binding.character_card_id()
        || artifact.memory_generation() != binding.memory_generation()
        || artifact.memory_preset_id() != binding.preset().id()
        || artifact.memory_preset_revision() != binding.preset().revision()
        || artifact.prompt_preset() != binding.preset().prompt_preset()
        || artifact.summary_model() != binding.resolved_summary_model()
    {
        return Err(MemoryError::RetrievalUnavailable(
            "memory candidate is outside the active session snapshot",
        ));
    }
    Ok(())
}

fn candidate_is_eligible(binding: &MemorySessionBinding, candidate: &MemoryCandidate) -> bool {
    artifact_is_eligible(binding, &candidate.artifact)
}

fn artifact_is_eligible(binding: &MemorySessionBinding, artifact: &SummaryArtifact) -> bool {
    match artifact.validity() {
        MemoryValidity::Attached => true,
        MemoryValidity::SourceCompacted => binding.preset().preserve_compacted_orphans(),
        MemoryValidity::InvalidatedByEdit | MemoryValidity::InvalidatedByDeletion => false,
    }
}

fn materialize_current_artifacts(
    binding: &MemorySessionBinding,
    selected: &[SelectedMemory],
    current_artifacts: &[SummaryArtifact],
) -> Result<String> {
    if current_artifacts.len() > MAX_MEMORY_CANDIDATES {
        return Err(MemoryError::too_many(
            "currentMemoryArtifacts",
            MAX_MEMORY_CANDIDATES,
        ));
    }
    let mut by_id = BTreeMap::new();
    for artifact in current_artifacts {
        if by_id.insert(artifact.id(), artifact).is_some() {
            return Err(MemoryError::DuplicateId {
                field: "currentMemoryArtifactId",
                id: artifact.id().to_string(),
            });
        }
    }

    let mut resolved = Vec::with_capacity(selected.len());
    let mut output_bytes = 0usize;
    for (index, selected) in selected.iter().enumerate() {
        let artifact =
            by_id
                .get(&selected.artifact_id)
                .copied()
                .ok_or(MemoryError::RetrievalUnavailable(
                    "a selected artifact is absent from the current authoritative snapshot",
                ))?;
        verify_artifact_scope(binding, artifact)?;
        if artifact.revision() != selected.artifact_revision
            || artifact.first_source_sequence() != selected.first_source_sequence
            || artifact.last_source_sequence() != selected.last_source_sequence
            || !artifact_is_eligible(binding, artifact)
        {
            return Err(MemoryError::RetrievalUnavailable(
                "a selected artifact changed or was invalidated before prompt application",
            ));
        }
        if index > 0 {
            output_bytes = output_bytes.checked_add(2).ok_or_else(|| {
                MemoryError::too_large("longTermMemory", MAX_LONG_TERM_MEMORY_INPUT_BYTES)
            })?;
        }
        output_bytes = output_bytes
            .checked_add(artifact.content().len())
            .ok_or_else(|| {
                MemoryError::too_large("longTermMemory", MAX_LONG_TERM_MEMORY_INPUT_BYTES)
            })?;
        if output_bytes > MAX_LONG_TERM_MEMORY_INPUT_BYTES {
            return Err(MemoryError::too_large(
                "longTermMemory",
                MAX_LONG_TERM_MEMORY_INPUT_BYTES,
            ));
        }
        resolved.push(artifact);
    }

    let mut output = String::with_capacity(output_bytes);
    for (index, artifact) in resolved.into_iter().enumerate() {
        if index > 0 {
            output.push_str("\n\n");
        }
        output.push_str(artifact.content());
    }
    if output.contains('\0') {
        return Err(MemoryError::invalid(
            "longTermMemory",
            "must contain no NUL",
        ));
    }
    Ok(output)
}

fn select_bucket(
    ranked: &[&MemoryCandidate],
    token_budget: u32,
    bucket: MemorySelectionBucket,
    selected: &mut Vec<SelectedMemory>,
    selected_ids: &mut BTreeSet<MemoryArtifactId>,
) {
    let mut remaining = token_budget;
    for candidate in ranked {
        if selected_ids.contains(candidate.artifact.id())
            || candidate.reserved_prompt_tokens > remaining
        {
            continue;
        }
        remaining -= candidate.reserved_prompt_tokens;
        push_selected(selected, selected_ids, candidate, bucket);
    }
}

fn push_selected(
    selected: &mut Vec<SelectedMemory>,
    selected_ids: &mut BTreeSet<MemoryArtifactId>,
    candidate: &MemoryCandidate,
    bucket: MemorySelectionBucket,
) {
    let inserted = selected_ids.insert(candidate.artifact.id().clone());
    debug_assert!(inserted, "selection buckets must deduplicate candidates");
    selected.push(SelectedMemory {
        artifact_id: candidate.artifact.id().clone(),
        artifact_revision: candidate.artifact.revision(),
        first_source_sequence: candidate.artifact.first_source_sequence(),
        last_source_sequence: candidate.artifact.last_source_sequence(),
        bucket,
        reserved_prompt_tokens: candidate.reserved_prompt_tokens,
    });
}

fn deterministic_random_key(
    binding: &MemorySessionBinding,
    query_snapshot_digest: &[u8; 32],
    artifact_id: &MemoryArtifactId,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(RANDOM_ORDER_DOMAIN);
    update_digest_field(&mut digest, binding.chat_id().as_str().as_bytes());
    digest.update(binding.memory_generation().to_be_bytes());
    digest.update(query_snapshot_digest);
    update_digest_field(&mut digest, artifact_id.as_str().as_bytes());
    digest.finalize().into()
}

fn update_digest_field(digest: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("bounded memory field length fits u64");
    digest.update(length.to_be_bytes());
    digest.update(value);
}

#[cfg(test)]
mod tests {
    use crate::{
        BasisPoints, ChunkSeparatorRegex, EmbeddingProfileId, EmbeddingProfileRef,
        HybridRetrievalPolicy, InsertionPolicy, MemoryBudgetInput, MemoryMix, MemoryPreset,
        MemoryPresetDraft, MemoryPresetId, ModelPresetId, ModelPresetRef, PromptPresetId,
        PromptPresetRef, RegenerationRegexPolicy, SummaryModelRef, SummaryPolicy,
    };

    use super::*;

    fn prompt() -> lorepia_prompt::PromptPreset {
        lorepia_prompt::PromptPreset {
            name: "prompt".to_owned(),
            blocks: vec![lorepia_prompt::PromptBlock::Raw {
                name: "system".to_owned(),
                enabled: true,
                role: lorepia_prompt::PromptRole::System,
                special: None,
                prompt: "System instruction".to_owned(),
            }],
            sampling: lorepia_prompt::PromptSampling::default(),
            advanced: lorepia_prompt::AdvancedSettings::default(),
        }
    }

    fn binding(preserve_orphans: bool) -> MemorySessionBinding {
        let mix = MemoryMix::new(
            BasisPoints::new(3_000).unwrap(),
            BasisPoints::new(4_000).unwrap(),
            BasisPoints::new(3_000).unwrap(),
        )
        .unwrap();
        let preset = MemoryPreset::create(
            MemoryPresetId::parse("memory").unwrap(),
            MemoryPresetDraft {
                label: "Memory".to_owned(),
                strategy: MemoryStrategy::HybridRetrieval(
                    HybridRetrievalPolicy::new(
                        4,
                        crate::EmbeddingQueryScope::Conversation,
                        mix,
                        EmbeddingProfileRef::new(
                            EmbeddingProfileId::parse("embedding").unwrap(),
                            1,
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ),
                prompt_preset: PromptPresetRef::new(PromptPresetId::parse("prompt").unwrap(), 1)
                    .unwrap(),
                summary_model: SummaryModelRef::ActiveChatModel,
                summary: SummaryPolicy::new(
                    "Summarize.",
                    "Consolidate.",
                    16,
                    false,
                    ChunkSeparatorRegex::new(r"\n{2,}").unwrap(),
                )
                .unwrap(),
                insertion: InsertionPolicy::new(
                    BasisPoints::new(5_000).unwrap(),
                    BasisPoints::new(8_000).unwrap(),
                )
                .unwrap(),
                preserve_compacted_orphans: preserve_orphans,
                regeneration_regex: RegenerationRegexPolicy::Skip,
            },
        )
        .unwrap();
        MemorySessionBinding::new(
            ChatId::parse("chat").unwrap(),
            CharacterCardId::parse("card").unwrap(),
            7,
            &preset,
            ModelPresetRef::new(ModelPresetId::parse("chat-model").unwrap(), 1).unwrap(),
            &prompt(),
        )
        .unwrap()
    }

    fn artifact(
        binding: &MemorySessionBinding,
        id: &str,
        sequence: u64,
        content: &str,
    ) -> SummaryArtifact {
        let source = crate::MemorySourceMessage::new(
            MessageId::parse(format!("source-{id}")).unwrap(),
            binding.chat_id().clone(),
            sequence,
            lorepia_providers::MessageRole::Assistant,
            format!("source for {id}"),
        )
        .unwrap();
        let sources = vec![source];
        let job = crate::plan_initial_summary_jobs(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            &sources,
        )
        .unwrap()
        .remove(0);
        let processed = crate::process_summary_output(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            &job,
            crate::SummaryOutputCandidate {
                current_source: crate::AuthoritativeSummarySource::Messages(&sources),
                resolved_prompt: &prompt(),
                mode: crate::SummaryOutputMode::InitialGeneration,
                raw_output: content,
            },
        )
        .unwrap();
        SummaryArtifact::new(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            processed,
            MemoryArtifactId::parse(id).unwrap(),
        )
        .unwrap()
    }

    fn consolidated_artifact(
        binding: &MemorySessionBinding,
        id: &str,
        sources: &[SummaryArtifact],
        content: &str,
    ) -> SummaryArtifact {
        let job = crate::plan_resummary_job(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            sources,
        )
        .unwrap();
        let processed = crate::process_summary_output(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            &job,
            crate::SummaryOutputCandidate {
                current_source: crate::AuthoritativeSummarySource::Artifacts(sources),
                resolved_prompt: &prompt(),
                mode: crate::SummaryOutputMode::InitialGeneration,
                raw_output: content,
            },
        )
        .unwrap();
        SummaryArtifact::new(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            processed,
            MemoryArtifactId::parse(id).unwrap(),
        )
        .unwrap()
    }

    fn budget_input() -> MemoryBudgetInput {
        MemoryBudgetInput::new(1_000, 200, 300).unwrap()
    }

    fn query(
        binding: &MemorySessionBinding,
    ) -> (
        EmbeddingQueryPlan,
        EmbeddingVector,
        Vec<MemorySourceMessage>,
    ) {
        let message = crate::MemorySourceMessage::new(
            MessageId::parse("q1").unwrap(),
            binding.chat_id().clone(),
            100,
            lorepia_providers::MessageRole::Assistant,
            "current query",
        )
        .unwrap();
        let messages = vec![message];
        let plan = crate::build_embedding_query(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            &messages,
        )
        .unwrap();
        let vector =
            EmbeddingVector::new(plan.embedding_profile().clone(), vec![1.0, 0.0]).unwrap();
        (plan, vector, messages)
    }

    fn matched_candidate(
        binding: &MemorySessionBinding,
        artifact: SummaryArtifact,
        reserved_prompt_tokens: u32,
        plan: &EmbeddingQueryPlan,
        current_query_messages: &[MemorySourceMessage],
        query_vector: &EmbeddingVector,
        cosine: f32,
    ) -> MemoryCandidate {
        let perpendicular = (1.0 - cosine * cosine).sqrt();
        let vector = EmbeddingVector::new(
            plan.embedding_profile().clone(),
            vec![cosine, perpendicular],
        )
        .unwrap();
        let embedding = EmbeddingArtifact::new(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            &artifact,
            vector,
        )
        .unwrap();
        let evidence = EmbeddingMatch::new(
            binding,
            binding.chat_id(),
            binding.character_card_id(),
            plan,
            current_query_messages,
            query_vector,
            &embedding,
        )
        .unwrap();
        MemoryCandidate::new(artifact, reserved_prompt_tokens, Some(evidence)).unwrap()
    }

    #[test]
    fn hybrid_retrieval_is_deterministic_deduplicated_and_chronological() {
        let binding = binding(true);
        let (plan, query_vector, query_messages) = query(&binding);
        let candidates = vec![
            matched_candidate(
                &binding,
                artifact(&binding, "m1", 1, "one"),
                30,
                &plan,
                &query_messages,
                &query_vector,
                0.1,
            ),
            matched_candidate(
                &binding,
                artifact(&binding, "m2", 2, "two"),
                30,
                &plan,
                &query_messages,
                &query_vector,
                0.9,
            ),
            matched_candidate(
                &binding,
                artifact(&binding, "m3", 3, "three"),
                30,
                &plan,
                &query_messages,
                &query_vector,
                0.5,
            ),
            matched_candidate(
                &binding,
                artifact(&binding, "m4", 4, "four"),
                30,
                &plan,
                &query_messages,
                &query_vector,
                0.3,
            ),
        ];
        let first = select_memories(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            budget_input(),
            AuthoritativeEmbeddingQuery::Hybrid {
                plan: &plan,
                messages: &query_messages,
            },
            &candidates,
        )
        .unwrap();
        let second = select_memories(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            budget_input(),
            AuthoritativeEmbeddingQuery::Hybrid {
                plan: &plan,
                messages: &query_messages,
            },
            &candidates,
        )
        .unwrap();
        assert_eq!(first, second);
        assert!(
            first
                .selected()
                .windows(2)
                .all(|pair| { pair[0].first_source_sequence() < pair[1].first_source_sequence() })
        );
        assert_eq!(
            first
                .selected()
                .iter()
                .map(SelectedMemory::artifact_id)
                .collect::<BTreeSet<_>>()
                .len(),
            first.selected().len()
        );
    }

    #[test]
    fn consolidated_summary_hides_only_its_actual_leaf_lineage() {
        let binding = binding(true);
        let (plan, query_vector, query_messages) = query(&binding);
        let first = artifact(&binding, "leaf-1", 1, "one");
        let gap = artifact(&binding, "leaf-2", 2, "two");
        let third = artifact(&binding, "leaf-3", 3, "three");
        let consolidated = consolidated_artifact(
            &binding,
            "consolidated",
            &[first.clone(), third.clone()],
            "one and three",
        );
        assert!(consolidated.covers_leaf(first.id()));
        assert!(!consolidated.covers_leaf(gap.id()));

        let candidates = vec![
            MemoryCandidate::new(consolidated, 20, None).unwrap(),
            matched_candidate(
                &binding,
                first,
                20,
                &plan,
                &query_messages,
                &query_vector,
                0.2,
            ),
            matched_candidate(
                &binding,
                gap,
                20,
                &plan,
                &query_messages,
                &query_vector,
                0.9,
            ),
            matched_candidate(
                &binding,
                third,
                20,
                &plan,
                &query_messages,
                &query_vector,
                0.1,
            ),
        ];
        let selection = select_memories(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            budget_input(),
            AuthoritativeEmbeddingQuery::Hybrid {
                plan: &plan,
                messages: &query_messages,
            },
            &candidates,
        )
        .unwrap();
        let selected = selection
            .selected()
            .iter()
            .map(|memory| memory.artifact_id().as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(selected, BTreeSet::from(["consolidated", "leaf-2"]));
    }

    #[test]
    fn missing_similarity_fails_closed_without_bucket_fallback() {
        let binding = binding(true);
        let (plan, _, query_messages) = query(&binding);
        let candidates =
            [MemoryCandidate::new(artifact(&binding, "missing", 1, "memory"), 20, None).unwrap()];
        assert!(matches!(
            select_memories(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                budget_input(),
                AuthoritativeEmbeddingQuery::Hybrid {
                    plan: &plan,
                    messages: &query_messages,
                },
                &candidates,
            ),
            Err(MemoryError::RetrievalUnavailable(_))
        ));
    }

    #[test]
    fn embedding_evidence_is_bound_to_the_exact_query_snapshot() {
        let binding = binding(true);
        let (plan, query_vector, query_messages) = query(&binding);
        let candidate = matched_candidate(
            &binding,
            artifact(&binding, "matched", 1, "memory"),
            20,
            &plan,
            &query_messages,
            &query_vector,
            0.8,
        );
        let changed_query = vec![
            MemorySourceMessage::new(
                query_messages[0].id().clone(),
                binding.chat_id().clone(),
                query_messages[0].sequence(),
                query_messages[0].role(),
                "edited query with the same ID",
            )
            .unwrap(),
        ];
        assert!(matches!(
            select_memories(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                budget_input(),
                AuthoritativeEmbeddingQuery::Hybrid {
                    plan: &plan,
                    messages: &changed_query,
                },
                &[candidate],
            ),
            Err(MemoryError::RetrievalUnavailable(_))
        ));
    }

    #[test]
    fn compacted_artifact_cannot_be_embedded_when_orphan_preservation_is_off() {
        let binding = binding(false);
        let artifact = artifact(&binding, "compacted", 1, "memory").source_compacted();
        let profile = match binding.preset().strategy() {
            MemoryStrategy::HybridRetrieval(policy) => policy.embedding_profile().clone(),
            MemoryStrategy::RollingSummary => unreachable!("test binding is hybrid"),
        };
        assert!(
            EmbeddingArtifact::new(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                &artifact,
                EmbeddingVector::new(profile, vec![1.0, 0.0]).unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn deletion_and_edit_always_win_over_orphan_preservation() {
        let binding = binding(true);
        let (plan, query_vector, query_messages) = query(&binding);
        let candidates = vec![
            matched_candidate(
                &binding,
                artifact(&binding, "compacted", 1, "kept").source_compacted(),
                20,
                &plan,
                &query_messages,
                &query_vector,
                0.1,
            ),
            MemoryCandidate::new(
                artifact(&binding, "edited", 2, "stale")
                    .invalidated_by_edit()
                    .unwrap(),
                20,
                None,
            )
            .unwrap(),
            MemoryCandidate::new(
                artifact(&binding, "deleted", 3, "private")
                    .invalidated_by_deletion()
                    .unwrap(),
                20,
                None,
            )
            .unwrap(),
        ];
        let selection = select_memories(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            budget_input(),
            AuthoritativeEmbeddingQuery::Hybrid {
                plan: &plan,
                messages: &query_messages,
            },
            &candidates,
        )
        .unwrap();
        assert_eq!(selection.selected().len(), 1);
        let current = candidates
            .iter()
            .map(|candidate| candidate.artifact().clone())
            .collect::<Vec<_>>();
        assert_eq!(
            selection
                .materialize(
                    &binding,
                    binding.chat_id(),
                    binding.character_card_id(),
                    budget_input(),
                    AuthoritativeEmbeddingQuery::Hybrid {
                        plan: &plan,
                        messages: &query_messages,
                    },
                    &current,
                )
                .unwrap(),
            "kept"
        );
    }

    #[test]
    fn selection_revalidates_authoritative_artifact_revision_after_deletion() {
        let binding = binding(true);
        let (plan, query_vector, query_messages) = query(&binding);
        let original = artifact(&binding, "private", 1, "private content");
        let candidate = matched_candidate(
            &binding,
            original.clone(),
            20,
            &plan,
            &query_messages,
            &query_vector,
            0.8,
        );
        let selection = select_memories(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            budget_input(),
            AuthoritativeEmbeddingQuery::Hybrid {
                plan: &plan,
                messages: &query_messages,
            },
            &[candidate],
        )
        .unwrap();
        let deleted = original.invalidated_by_deletion().unwrap();
        assert!(deleted.content().is_empty());

        assert!(
            selection
                .materialize(
                    &binding,
                    binding.chat_id(),
                    binding.character_card_id(),
                    budget_input(),
                    AuthoritativeEmbeddingQuery::Hybrid {
                        plan: &plan,
                        messages: &query_messages,
                    },
                    &[deleted],
                )
                .is_err()
        );
    }

    #[test]
    fn materialization_verifies_scope_budget_and_query_snapshot() {
        let binding = binding(true);
        let (plan, _, query_messages) = query(&binding);
        let selection = select_memories(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            budget_input(),
            AuthoritativeEmbeddingQuery::Hybrid {
                plan: &plan,
                messages: &query_messages,
            },
            &[],
        )
        .unwrap();
        assert_eq!(
            selection
                .materialize(
                    &binding,
                    binding.chat_id(),
                    binding.character_card_id(),
                    budget_input(),
                    AuthoritativeEmbeddingQuery::Hybrid {
                        plan: &plan,
                        messages: &query_messages,
                    },
                    &[],
                )
                .unwrap(),
            ""
        );

        assert!(
            selection
                .materialize(
                    &binding,
                    &ChatId::parse("other").unwrap(),
                    binding.character_card_id(),
                    budget_input(),
                    AuthoritativeEmbeddingQuery::Hybrid {
                        plan: &plan,
                        messages: &query_messages,
                    },
                    &[],
                )
                .is_err()
        );

        assert!(
            selection
                .materialize(
                    &binding,
                    binding.chat_id(),
                    binding.character_card_id(),
                    MemoryBudgetInput::new(2_000, 200, 300).unwrap(),
                    AuthoritativeEmbeddingQuery::Hybrid {
                        plan: &plan,
                        messages: &query_messages,
                    },
                    &[],
                )
                .is_err()
        );

        let newer_binding = MemorySessionBinding::new(
            binding.chat_id().clone(),
            binding.character_card_id().clone(),
            binding.memory_generation() + 1,
            binding.preset(),
            binding.resolved_summary_model().clone(),
            &prompt(),
        )
        .unwrap();
        assert!(
            selection
                .materialize(
                    &newer_binding,
                    newer_binding.chat_id(),
                    newer_binding.character_card_id(),
                    budget_input(),
                    AuthoritativeEmbeddingQuery::Hybrid {
                        plan: &plan,
                        messages: &query_messages,
                    },
                    &[],
                )
                .is_err()
        );
    }
}
