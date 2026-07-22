use std::collections::BTreeSet;

use lorepia_persona::{CharacterCardId, ChatId};
use lorepia_prompt::{PromptPreset, TransformTarget, apply_text_transforms};
use lorepia_providers::MessageRole;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    EmbeddingProfileRef, MAX_MEMORY_ARTIFACT_BYTES, MAX_SOURCE_BATCH_BYTES,
    MAX_SOURCE_MESSAGE_BYTES, MAX_SOURCE_MESSAGES, MAX_SUMMARY_PLAN_BYTES, MemoryError,
    MemoryPresetId, MemorySessionBinding, MemoryStrategy, MemoryValidity, MessageId,
    ModelPresetRef, PromptPresetRef, RegenerationRegexPolicy, Result, SummaryArtifact,
};

const MESSAGE_SNAPSHOT_DOMAIN: &[u8] = b"lorepia-memory-message-snapshot-v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySourceMessage {
    id: MessageId,
    chat_id: ChatId,
    sequence: u64,
    role: MessageRole,
    content: String,
}

impl MemorySourceMessage {
    pub fn new(
        id: MessageId,
        chat_id: ChatId,
        sequence: u64,
        role: MessageRole,
        content: impl Into<String>,
    ) -> Result<Self> {
        if sequence == 0 {
            return Err(MemoryError::invalid(
                "sourceMessage.sequence",
                "must be at least 1",
            ));
        }
        if role == MessageRole::System {
            return Err(MemoryError::invalid(
                "sourceMessage.role",
                "memory source messages must be user or assistant messages",
            ));
        }
        let content = content.into();
        validate_source_content(&content)?;
        Ok(Self {
            id,
            chat_id,
            sequence,
            role,
            content,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &MessageId {
        &self.id
    }

    #[must_use]
    pub const fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn role(&self) -> MessageRole {
        self.role
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryJobKind {
    Initial,
    Resummary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SummaryOutputMode {
    InitialGeneration,
    Regeneration,
}

pub struct SummaryOutputCandidate<'a> {
    pub current_source: AuthoritativeSummarySource<'a>,
    pub resolved_prompt: &'a PromptPreset,
    pub mode: SummaryOutputMode,
    pub raw_output: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "items", rename_all = "snake_case")]
pub enum SummaryJobSource {
    Messages(Vec<MemorySourceMessage>),
    Artifacts(Vec<SummaryArtifact>),
}

impl SummaryJobSource {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Messages(messages) => messages.is_empty(),
            Self::Artifacts(artifacts) => artifacts.is_empty(),
        }
    }
}

/// The exact current repository slice used to revalidate an asynchronous
/// summary job before accepting its result.
pub enum AuthoritativeSummarySource<'a> {
    Messages(&'a [MemorySourceMessage]),
    Artifacts(&'a [SummaryArtifact]),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MessageSnapshotPin {
    id: MessageId,
    sequence: u64,
    role: MessageRole,
    content_digest: [u8; 32],
}

impl MessageSnapshotPin {
    fn from_message(message: &MemorySourceMessage) -> Self {
        Self {
            id: message.id.clone(),
            sequence: message.sequence,
            role: message.role,
            content_digest: digest_message(message),
        }
    }

    fn matches(&self, message: &MemorySourceMessage) -> bool {
        self.id == message.id
            && self.sequence == message.sequence
            && self.role == message.role
            && self.content_digest == digest_message(message)
    }
}

/// A credential-free native execution plan.
///
/// `instruction` and `source` stay in separate fields so untrusted chat text is
/// never interpolated into an authored system instruction by this crate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryJob {
    chat_id: ChatId,
    character_card_id: CharacterCardId,
    memory_generation: u64,
    memory_preset_id: MemoryPresetId,
    memory_preset_revision: u64,
    prompt_preset: PromptPresetRef,
    summary_model: ModelPresetRef,
    kind: SummaryJobKind,
    instruction: String,
    first_source_sequence: u64,
    last_source_sequence: u64,
    source: SummaryJobSource,
    #[serde(skip)]
    message_snapshot: Vec<MessageSnapshotPin>,
}

impl SummaryJob {
    pub(crate) fn verify_binding(&self, binding: &MemorySessionBinding) -> Result<()> {
        if self.chat_id != *binding.chat_id()
            || self.character_card_id != *binding.character_card_id()
            || self.memory_generation != binding.memory_generation()
            || self.memory_preset_id != *binding.preset().id()
            || self.memory_preset_revision != binding.preset().revision()
            || self.prompt_preset != *binding.preset().prompt_preset()
            || self.summary_model != *binding.resolved_summary_model()
        {
            return Err(MemoryError::RetrievalUnavailable(
                "summary job is outside the active memory session snapshot",
            ));
        }
        Ok(())
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
    pub const fn kind(&self) -> SummaryJobKind {
        self.kind
    }

    #[must_use]
    pub fn instruction(&self) -> &str {
        &self.instruction
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
    pub const fn source(&self) -> &SummaryJobSource {
        &self.source
    }

    /// False means the filtered batch intentionally advances its source range
    /// without sending an empty request to a helper model.
    #[must_use]
    pub fn requires_generation(&self) -> bool {
        !self.source.is_empty()
    }

    /// Must be called against a repository-loaded source slice immediately
    /// before provider send and again before accepting the result.
    pub fn verify_authoritative_source(
        &self,
        binding: &MemorySessionBinding,
        current: AuthoritativeSummarySource<'_>,
    ) -> Result<()> {
        match (&self.source, current) {
            (SummaryJobSource::Messages(_), AuthoritativeSummarySource::Messages(messages)) => {
                validate_message_snapshot(binding, messages)?;
                if messages.len() != self.message_snapshot.len()
                    || !self
                        .message_snapshot
                        .iter()
                        .zip(messages)
                        .all(|(pin, message)| pin.matches(message))
                {
                    return Err(MemoryError::RetrievalUnavailable(
                        "summary source messages changed after the job was planned",
                    ));
                }
            }
            (
                SummaryJobSource::Artifacts(planned),
                AuthoritativeSummarySource::Artifacts(artifacts),
            ) => {
                for artifact in artifacts {
                    verify_artifact_scope(binding, artifact)?;
                }
                if planned != artifacts {
                    return Err(MemoryError::RetrievalUnavailable(
                        "summary source artifacts changed after the job was planned",
                    ));
                }
            }
            _ => {
                return Err(MemoryError::RetrievalUnavailable(
                    "authoritative summary source type differs from the planned job",
                ));
            }
        }
        Ok(())
    }
}

/// Validated helper-model output that retains the exact job provenance.
///
/// It is intentionally non-serializable and non-cloneable. A summary artifact
/// can only be created by consuming this value.
#[derive(Debug)]
pub struct ProcessedSummary {
    job: SummaryJob,
    content: String,
}

impl ProcessedSummary {
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    pub(crate) fn into_parts(self) -> (SummaryJob, String) {
        (self.job, self.content)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingQueryPlan {
    chat_id: ChatId,
    character_card_id: CharacterCardId,
    memory_generation: u64,
    memory_preset_id: MemoryPresetId,
    memory_preset_revision: u64,
    prompt_preset: PromptPresetRef,
    summary_model: ModelPresetRef,
    embedding_profile: EmbeddingProfileRef,
    messages: Vec<MemorySourceMessage>,
    #[serde(skip)]
    message_snapshot_digest: [u8; 32],
}

impl EmbeddingQueryPlan {
    pub(crate) fn verify_binding(&self, binding: &MemorySessionBinding) -> Result<()> {
        let MemoryStrategy::HybridRetrieval(policy) = binding.preset().strategy() else {
            return Err(MemoryError::RetrievalUnavailable(
                "rolling-summary mode has no embedding query",
            ));
        };
        if self.chat_id != *binding.chat_id()
            || self.character_card_id != *binding.character_card_id()
            || self.memory_generation != binding.memory_generation()
            || self.memory_preset_id != *binding.preset().id()
            || self.memory_preset_revision != binding.preset().revision()
            || self.prompt_preset != *binding.preset().prompt_preset()
            || self.summary_model != *binding.resolved_summary_model()
            || self.embedding_profile != *policy.embedding_profile()
        {
            return Err(MemoryError::RetrievalUnavailable(
                "embedding query plan is outside the active memory session snapshot",
            ));
        }
        Ok(())
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
    pub const fn embedding_profile(&self) -> &EmbeddingProfileRef {
        &self.embedding_profile
    }

    #[must_use]
    pub fn messages(&self) -> &[MemorySourceMessage] {
        &self.messages
    }

    /// Revalidates the exact selected query slice at the provider-send and
    /// result-acceptance boundaries. The caller must load this slice from the
    /// authoritative chat repository, not from the queued plan.
    pub fn verify_authoritative_messages(
        &self,
        binding: &MemorySessionBinding,
        messages: &[MemorySourceMessage],
    ) -> Result<()> {
        self.verify_binding(binding)?;
        validate_message_snapshot(binding, messages)?;
        if self.messages != messages
            || self.message_snapshot_digest != digest_message_snapshot(messages)
        {
            return Err(MemoryError::RetrievalUnavailable(
                "embedding query messages changed after the plan was created",
            ));
        }
        Ok(())
    }

    pub(crate) const fn message_snapshot_digest(&self) -> &[u8; 32] {
        &self.message_snapshot_digest
    }
}

pub fn plan_initial_summary_jobs(
    binding: &MemorySessionBinding,
    expected_chat_id: &ChatId,
    expected_character_card_id: &CharacterCardId,
    messages: &[MemorySourceMessage],
) -> Result<Vec<SummaryJob>> {
    binding.verify_scope(expected_chat_id, expected_character_card_id)?;
    validate_message_snapshot(binding, messages)?;

    let batch_size = usize::from(binding.preset().summary().max_messages_per_summary());
    let mut jobs = Vec::with_capacity(messages.len().div_ceil(batch_size));
    let mut planned_bytes = 0usize;
    let skip_user_messages = binding.preset().summary().skip_user_messages();
    let summary_prompt = binding.preset().summary().summary_prompt();
    for batch in messages.chunks(batch_size) {
        let first_source_sequence = batch
            .first()
            .expect("chunks of a non-zero size are non-empty")
            .sequence;
        let last_source_sequence = batch
            .last()
            .expect("chunks of a non-zero size are non-empty")
            .sequence;
        let included = batch
            .iter()
            .filter(|message| !skip_user_messages || message.role != MessageRole::User)
            .collect::<Vec<_>>();
        let batch_bytes = validate_batch_bytes(
            "summaryJob.messages",
            included.iter().map(|message| message.content.len()),
        )?;
        let instruction_bytes = if included.is_empty() {
            0
        } else {
            summary_prompt.len()
        };
        let job_bytes = batch_bytes
            .checked_add(instruction_bytes)
            .ok_or_else(|| MemoryError::too_large("summaryPlan", MAX_SUMMARY_PLAN_BYTES))?;
        planned_bytes = planned_bytes
            .checked_add(job_bytes)
            .ok_or_else(|| MemoryError::too_large("summaryPlan", MAX_SUMMARY_PLAN_BYTES))?;
        if planned_bytes > MAX_SUMMARY_PLAN_BYTES {
            return Err(MemoryError::too_large(
                "summaryPlan",
                MAX_SUMMARY_PLAN_BYTES,
            ));
        }
        let source = included.into_iter().cloned().collect::<Vec<_>>();
        let message_snapshot = batch.iter().map(MessageSnapshotPin::from_message).collect();
        jobs.push(job_from_binding(
            binding,
            SummaryJobKind::Initial,
            if source.is_empty() {
                String::new()
            } else {
                summary_prompt.to_owned()
            },
            first_source_sequence,
            last_source_sequence,
            SummaryJobSource::Messages(source),
            message_snapshot,
        ));
    }
    Ok(jobs)
}

pub fn plan_resummary_job(
    binding: &MemorySessionBinding,
    expected_chat_id: &ChatId,
    expected_character_card_id: &CharacterCardId,
    artifacts: &[SummaryArtifact],
) -> Result<SummaryJob> {
    binding.verify_scope(expected_chat_id, expected_character_card_id)?;
    if artifacts.is_empty() {
        return Err(MemoryError::invalid(
            "resummaryArtifacts",
            "must contain at least one artifact",
        ));
    }
    if artifacts.len() > MAX_SOURCE_MESSAGES {
        return Err(MemoryError::too_many(
            "resummaryArtifacts",
            MAX_SOURCE_MESSAGES,
        ));
    }

    let mut ids = BTreeSet::new();
    let mut previous_last = None;
    for artifact in artifacts {
        verify_artifact_scope(binding, artifact)?;
        match artifact.validity() {
            MemoryValidity::InvalidatedByEdit | MemoryValidity::InvalidatedByDeletion => {
                return Err(MemoryError::invalid(
                    "resummaryArtifacts.validity",
                    "invalidated artifacts must never be summarized again",
                ));
            }
            MemoryValidity::SourceCompacted if !binding.preset().preserve_compacted_orphans() => {
                return Err(MemoryError::invalid(
                    "resummaryArtifacts.validity",
                    "compacted-source artifacts are disabled by this memory preset",
                ));
            }
            MemoryValidity::Attached | MemoryValidity::SourceCompacted => {}
        }
        if !ids.insert(artifact.id().clone()) {
            return Err(MemoryError::DuplicateId {
                field: "memoryArtifactId",
                id: artifact.id().to_string(),
            });
        }
        if previous_last.is_some_and(|previous| previous >= artifact.first_source_sequence()) {
            return Err(MemoryError::invalid(
                "resummaryArtifacts",
                "artifacts must be strictly chronological and non-overlapping",
            ));
        }
        previous_last = Some(artifact.last_source_sequence());
    }
    let _ = validate_batch_bytes(
        "resummaryArtifacts",
        artifacts.iter().map(|artifact| artifact.content().len()),
    )?;

    Ok(job_from_binding(
        binding,
        SummaryJobKind::Resummary,
        binding.preset().summary().resummary_prompt().to_owned(),
        artifacts
            .first()
            .expect("non-empty artifacts were checked")
            .first_source_sequence(),
        artifacts
            .last()
            .expect("non-empty artifacts were checked")
            .last_source_sequence(),
        SummaryJobSource::Artifacts(artifacts.to_vec()),
        Vec::new(),
    ))
}

pub fn build_embedding_query(
    binding: &MemorySessionBinding,
    expected_chat_id: &ChatId,
    expected_character_card_id: &CharacterCardId,
    messages: &[MemorySourceMessage],
) -> Result<EmbeddingQueryPlan> {
    binding.verify_scope(expected_chat_id, expected_character_card_id)?;
    validate_message_snapshot(binding, messages)?;
    let MemoryStrategy::HybridRetrieval(policy) = binding.preset().strategy() else {
        return Err(MemoryError::RetrievalUnavailable(
            "rolling-summary mode has no embedding query",
        ));
    };

    let eligible = messages
        .iter()
        .filter(|message| match policy.query_scope() {
            crate::EmbeddingQueryScope::Conversation => true,
            crate::EmbeddingQueryScope::AssistantOnly => message.role == MessageRole::Assistant,
        });
    let mut selected = eligible
        .rev()
        .take(usize::from(policy.query_message_count()))
        .collect::<Vec<_>>();
    selected.reverse();
    if selected.is_empty() {
        return Err(MemoryError::RetrievalUnavailable(
            "the configured embedding query scope selected no messages",
        ));
    }
    let _ = validate_batch_bytes(
        "embeddingQuery.messages",
        selected.iter().map(|message| message.content.len()),
    )?;
    let messages = selected.into_iter().cloned().collect::<Vec<_>>();
    let message_snapshot_digest = digest_message_snapshot(&messages);
    Ok(EmbeddingQueryPlan {
        chat_id: binding.chat_id().clone(),
        character_card_id: binding.character_card_id().clone(),
        memory_generation: binding.memory_generation(),
        memory_preset_id: binding.preset().id().clone(),
        memory_preset_revision: binding.preset().revision(),
        prompt_preset: binding.preset().prompt_preset().clone(),
        summary_model: binding.resolved_summary_model().clone(),
        embedding_profile: policy.embedding_profile().clone(),
        messages,
        message_snapshot_digest,
    })
}

pub fn process_summary_output(
    binding: &MemorySessionBinding,
    expected_chat_id: &ChatId,
    expected_character_card_id: &CharacterCardId,
    job: &SummaryJob,
    candidate: SummaryOutputCandidate<'_>,
) -> Result<ProcessedSummary> {
    binding.verify_scope(expected_chat_id, expected_character_card_id)?;
    job.verify_binding(binding)?;
    job.verify_authoritative_source(binding, candidate.current_source)?;
    binding.verify_prompt_preset(candidate.resolved_prompt)?;
    validate_generated_content(candidate.raw_output)?;
    let output = match candidate.mode {
        SummaryOutputMode::InitialGeneration => candidate.raw_output.to_owned(),
        SummaryOutputMode::Regeneration => match binding.preset().regeneration_regex() {
            RegenerationRegexPolicy::Skip => candidate.raw_output.to_owned(),
            RegenerationRegexPolicy::ApplyPinnedPromptResponseRules => apply_text_transforms(
                candidate.resolved_prompt,
                TransformTarget::Response,
                candidate.raw_output,
            )?,
        },
    };
    validate_generated_content(&output)?;
    Ok(ProcessedSummary {
        job: job.clone(),
        content: output,
    })
}

pub fn process_regenerated_summary(
    binding: &MemorySessionBinding,
    expected_chat_id: &ChatId,
    expected_character_card_id: &CharacterCardId,
    job: &SummaryJob,
    current_source: AuthoritativeSummarySource<'_>,
    resolved_prompt: &PromptPreset,
    raw_output: &str,
) -> Result<ProcessedSummary> {
    process_summary_output(
        binding,
        expected_chat_id,
        expected_character_card_id,
        job,
        SummaryOutputCandidate {
            current_source,
            resolved_prompt,
            mode: SummaryOutputMode::Regeneration,
            raw_output,
        },
    )
}

fn job_from_binding(
    binding: &MemorySessionBinding,
    kind: SummaryJobKind,
    instruction: String,
    first_source_sequence: u64,
    last_source_sequence: u64,
    source: SummaryJobSource,
    message_snapshot: Vec<MessageSnapshotPin>,
) -> SummaryJob {
    SummaryJob {
        chat_id: binding.chat_id().clone(),
        character_card_id: binding.character_card_id().clone(),
        memory_generation: binding.memory_generation(),
        memory_preset_id: binding.preset().id().clone(),
        memory_preset_revision: binding.preset().revision(),
        prompt_preset: binding.preset().prompt_preset().clone(),
        summary_model: binding.resolved_summary_model().clone(),
        kind,
        instruction,
        first_source_sequence,
        last_source_sequence,
        source,
        message_snapshot,
    }
}

fn validate_message_snapshot(
    binding: &MemorySessionBinding,
    messages: &[MemorySourceMessage],
) -> Result<()> {
    if messages.len() > MAX_SOURCE_MESSAGES {
        return Err(MemoryError::too_many("sourceMessages", MAX_SOURCE_MESSAGES));
    }
    let mut ids = BTreeSet::new();
    let mut previous_sequence = None;
    for message in messages {
        if message.chat_id != *binding.chat_id() {
            return Err(MemoryError::BindingMismatch {
                field: "sourceMessage.chatId",
                expected: binding.chat_id().to_string(),
                actual: message.chat_id.to_string(),
            });
        }
        if !ids.insert(message.id.clone()) {
            return Err(MemoryError::DuplicateId {
                field: "messageId",
                id: message.id.to_string(),
            });
        }
        if previous_sequence.is_some_and(|previous| previous >= message.sequence) {
            return Err(MemoryError::invalid(
                "sourceMessages.sequence",
                "messages must be strictly chronological",
            ));
        }
        previous_sequence = Some(message.sequence);
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
            "summary artifact is outside the active memory session",
        ));
    }
    Ok(())
}

fn validate_source_content(content: &str) -> Result<()> {
    if content.len() > MAX_SOURCE_MESSAGE_BYTES {
        return Err(MemoryError::too_large(
            "sourceMessage.content",
            MAX_SOURCE_MESSAGE_BYTES,
        ));
    }
    if content.contains('\0') {
        return Err(MemoryError::invalid(
            "sourceMessage.content",
            "must contain no NUL",
        ));
    }
    Ok(())
}

fn digest_message_snapshot(messages: &[MemorySourceMessage]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(MESSAGE_SNAPSHOT_DOMAIN);
    digest.update(
        u64::try_from(messages.len())
            .expect("bounded message count fits u64")
            .to_be_bytes(),
    );
    for message in messages {
        digest.update(digest_message(message));
    }
    digest.finalize().into()
}

fn digest_message(message: &MemorySourceMessage) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(MESSAGE_SNAPSHOT_DOMAIN);
    update_digest_field(&mut digest, message.id.as_str().as_bytes());
    digest.update(message.sequence.to_be_bytes());
    digest.update([match message.role {
        MessageRole::System => 0,
        MessageRole::User => 1,
        MessageRole::Assistant => 2,
    }]);
    update_digest_field(&mut digest, message.content.as_bytes());
    digest.finalize().into()
}

fn update_digest_field(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded memory field length fits u64")
            .to_be_bytes(),
    );
    digest.update(value);
}

fn validate_generated_content(content: &str) -> Result<()> {
    if content.trim().is_empty() {
        return Err(MemoryError::invalid(
            "generatedSummary",
            "must not be empty",
        ));
    }
    if content.len() > MAX_MEMORY_ARTIFACT_BYTES {
        return Err(MemoryError::too_large(
            "generatedSummary",
            MAX_MEMORY_ARTIFACT_BYTES,
        ));
    }
    if content.contains('\0') {
        return Err(MemoryError::invalid(
            "generatedSummary",
            "must contain no NUL",
        ));
    }
    Ok(())
}

fn validate_batch_bytes(
    field: &'static str,
    sizes: impl IntoIterator<Item = usize>,
) -> Result<usize> {
    let total = sizes.into_iter().try_fold(0usize, |total, size| {
        let next = total
            .checked_add(size)
            .ok_or_else(|| MemoryError::too_large(field, MAX_SOURCE_BATCH_BYTES))?;
        if next > MAX_SOURCE_BATCH_BYTES {
            return Err(MemoryError::too_large(field, MAX_SOURCE_BATCH_BYTES));
        }
        Ok(next)
    })?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use lorepia_prompt::{AdvancedSettings, PromptBlock, PromptRole, PromptSampling};

    use crate::{
        BasisPoints, ChunkSeparatorRegex, EmbeddingProfileId, EmbeddingProfileRef,
        EmbeddingQueryScope, HybridRetrievalPolicy, InsertionPolicy, MAX_SUMMARY_PROMPT_BYTES,
        MemoryMix, MemoryPreset, MemoryPresetDraft, MemoryPresetId, ModelPresetId, PromptPresetId,
        RegenerationRegexPolicy, SummaryModelRef, SummaryPolicy,
    };

    use super::*;

    fn prompt() -> PromptPreset {
        PromptPreset {
            name: "prompt".to_owned(),
            blocks: vec![PromptBlock::Raw {
                name: "system".to_owned(),
                enabled: true,
                role: PromptRole::System,
                special: None,
                prompt: "System instruction".to_owned(),
            }],
            sampling: PromptSampling::default(),
            advanced: AdvancedSettings::default(),
        }
    }

    fn binding(skip_user_messages: bool) -> MemorySessionBinding {
        binding_with_summary(skip_user_messages, "Summarize the source.", 2)
    }

    fn binding_with_summary(
        skip_user_messages: bool,
        summary_prompt: &str,
        max_messages_per_summary: u16,
    ) -> MemorySessionBinding {
        let mix = MemoryMix::new(
            BasisPoints::new(4_000).unwrap(),
            BasisPoints::new(4_000).unwrap(),
            BasisPoints::new(2_000).unwrap(),
        )
        .unwrap();
        let prompt_ref = PromptPresetRef::new(PromptPresetId::parse("prompt").unwrap(), 3).unwrap();
        let preset = MemoryPreset::create(
            MemoryPresetId::parse("memory").unwrap(),
            MemoryPresetDraft {
                label: "Memory".to_owned(),
                strategy: MemoryStrategy::HybridRetrieval(
                    HybridRetrievalPolicy::new(
                        2,
                        EmbeddingQueryScope::Conversation,
                        mix,
                        EmbeddingProfileRef::new(
                            EmbeddingProfileId::parse("embedding").unwrap(),
                            5,
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ),
                prompt_preset: prompt_ref,
                summary_model: SummaryModelRef::ActiveChatModel,
                summary: SummaryPolicy::new(
                    summary_prompt,
                    "Consolidate the summaries.",
                    max_messages_per_summary,
                    skip_user_messages,
                    ChunkSeparatorRegex::new(r"\n{2,}").unwrap(),
                )
                .unwrap(),
                insertion: InsertionPolicy::new(
                    BasisPoints::new(2_000).unwrap(),
                    BasisPoints::new(5_000).unwrap(),
                )
                .unwrap(),
                preserve_compacted_orphans: true,
                regeneration_regex: RegenerationRegexPolicy::Skip,
            },
        )
        .unwrap();
        MemorySessionBinding::new(
            ChatId::parse("chat").unwrap(),
            CharacterCardId::parse("card").unwrap(),
            1,
            &preset,
            ModelPresetRef::new(ModelPresetId::parse("chat-model").unwrap(), 1).unwrap(),
            &prompt(),
        )
        .unwrap()
    }

    fn message(sequence: u64, role: MessageRole, content: &str) -> MemorySourceMessage {
        MemorySourceMessage::new(
            MessageId::parse(format!("message-{sequence}")).unwrap(),
            ChatId::parse("chat").unwrap(),
            sequence,
            role,
            content,
        )
        .unwrap()
    }

    #[test]
    fn user_filter_uses_original_batch_boundaries_and_advances_empty_ranges() {
        let binding = binding(true);
        let jobs = plan_initial_summary_jobs(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            &[
                message(1, MessageRole::User, "one"),
                message(2, MessageRole::User, "two"),
                message(3, MessageRole::Assistant, "three"),
            ],
        )
        .unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(
            (
                jobs[0].first_source_sequence(),
                jobs[0].last_source_sequence()
            ),
            (1, 2)
        );
        assert!(!jobs[0].requires_generation());
        assert_eq!(
            (
                jobs[1].first_source_sequence(),
                jobs[1].last_source_sequence()
            ),
            (3, 3)
        );
        assert!(jobs[1].requires_generation());
    }

    #[test]
    fn embedding_query_scope_is_independent_from_summary_filter() {
        let binding = binding(true);
        let messages = vec![
            message(1, MessageRole::Assistant, "old"),
            message(2, MessageRole::User, "private user query"),
            message(3, MessageRole::Assistant, "latest"),
        ];
        let mut query = build_embedding_query(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            &messages,
        )
        .unwrap();
        assert_eq!(query.messages().len(), 2);
        assert_eq!(query.messages()[0].role(), MessageRole::User);

        query.memory_preset_revision += 1;
        assert!(query.verify_binding(&binding).is_err());
    }

    #[test]
    fn message_snapshot_rejects_cross_chat_and_reordered_sources() {
        let binding = binding(false);
        let cross_chat = MemorySourceMessage::new(
            MessageId::parse("foreign").unwrap(),
            ChatId::parse("other").unwrap(),
            1,
            MessageRole::User,
            "text",
        )
        .unwrap();
        assert!(
            plan_initial_summary_jobs(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                &[cross_chat],
            )
            .is_err()
        );
        assert!(
            plan_initial_summary_jobs(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                &[
                    message(2, MessageRole::User, "two"),
                    message(1, MessageRole::Assistant, "one"),
                ],
            )
            .is_err()
        );
    }

    #[test]
    fn initial_summary_plan_has_an_aggregate_allocation_cap() {
        let binding = binding(false);
        let message_count = MAX_SUMMARY_PLAN_BYTES / MAX_SOURCE_MESSAGE_BYTES + 1;
        let messages = (1..=message_count)
            .map(|sequence| {
                MemorySourceMessage::new(
                    MessageId::parse(format!("large-{sequence}")).unwrap(),
                    binding.chat_id().clone(),
                    u64::try_from(sequence).unwrap(),
                    MessageRole::Assistant,
                    "x".repeat(MAX_SOURCE_MESSAGE_BYTES),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            plan_initial_summary_jobs(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                &messages,
            ),
            Err(MemoryError::PayloadTooLarge { field, max_bytes })
                if field == "summaryPlan" && max_bytes == MAX_SUMMARY_PLAN_BYTES
        ));
    }

    #[test]
    fn repeated_summary_instructions_share_the_aggregate_plan_cap() {
        let binding = binding_with_summary(false, &"p".repeat(MAX_SUMMARY_PROMPT_BYTES), 1);
        let message_count = MAX_SUMMARY_PLAN_BYTES / MAX_SUMMARY_PROMPT_BYTES + 1;
        let messages = (1..=message_count)
            .map(|sequence| {
                message(
                    u64::try_from(sequence).unwrap(),
                    MessageRole::Assistant,
                    "x",
                )
            })
            .collect::<Vec<_>>();

        assert!(matches!(
            plan_initial_summary_jobs(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                &messages,
            ),
            Err(MemoryError::PayloadTooLarge { field, max_bytes })
                if field == "summaryPlan" && max_bytes == MAX_SUMMARY_PLAN_BYTES
        ));
    }

    #[test]
    fn regenerated_output_requires_the_pinned_prompt_and_job_snapshot() {
        let binding = binding(false);
        let sources = vec![message(1, MessageRole::Assistant, "source")];
        let job = plan_initial_summary_jobs(
            &binding,
            binding.chat_id(),
            binding.character_card_id(),
            &sources,
        )
        .unwrap()
        .remove(0);
        let resolved_prompt = prompt();
        assert_eq!(
            process_regenerated_summary(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                &job,
                AuthoritativeSummarySource::Messages(&sources),
                &resolved_prompt,
                "summary",
            )
            .unwrap()
            .content(),
            "summary"
        );

        let mut changed_prompt = prompt();
        let PromptBlock::Raw {
            prompt: instruction,
            ..
        } = &mut changed_prompt.blocks[0]
        else {
            unreachable!("test prompt contains one raw block")
        };
        *instruction = "Changed instruction".to_owned();
        assert!(
            process_regenerated_summary(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                &job,
                AuthoritativeSummarySource::Messages(&sources),
                &changed_prompt,
                "summary",
            )
            .is_err()
        );

        let newer_binding = MemorySessionBinding::new(
            binding.chat_id().clone(),
            binding.character_card_id().clone(),
            binding.memory_generation() + 1,
            binding.preset(),
            binding.resolved_summary_model().clone(),
            &resolved_prompt,
        )
        .unwrap();
        assert!(
            process_regenerated_summary(
                &newer_binding,
                newer_binding.chat_id(),
                newer_binding.character_card_id(),
                &job,
                AuthoritativeSummarySource::Messages(&sources),
                &resolved_prompt,
                "summary",
            )
            .is_err()
        );

        let edited_sources = vec![message(1, MessageRole::Assistant, "edited source")];
        assert!(
            process_regenerated_summary(
                &binding,
                binding.chat_id(),
                binding.character_card_id(),
                &job,
                AuthoritativeSummarySource::Messages(&edited_sources),
                &resolved_prompt,
                "summary",
            )
            .is_err()
        );
    }
}
