use std::{
    collections::BTreeSet,
    io::{self, Write},
};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::{
    BasisPoints, ChunkSeparatorRegex, EmbeddingProfileId, EmbeddingProfileRef, EmbeddingQueryScope,
    HybridRetrievalPolicy, InsertionPolicy, MAX_MEMORY_PRESET_STATE_BYTES, MAX_MEMORY_PRESETS,
    MemoryError, MemoryMix, MemoryPreset, MemoryPresetDraft, MemoryPresetId, MemoryStrategy,
    ModelPresetId, ModelPresetRef, PromptPresetId, PromptPresetRef, RegenerationRegexPolicy,
    Result, SummaryModelRef, SummaryPolicy,
};

pub const MEMORY_PRESET_STATE_FORMAT: &str = "lorepia.memory-preset-state";
pub const MEMORY_PRESET_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateEnvelopeOut {
    format: &'static str,
    schema_version: u32,
    state: PresetStateV1,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateEnvelopeIn<'a> {
    format: String,
    schema_version: u32,
    #[serde(borrow)]
    state: &'a RawValue,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PresetStateV1 {
    presets: Vec<MemoryPresetV1>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PresetStateIn<'a> {
    #[serde(borrow)]
    presets: Vec<&'a RawValue>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemoryPresetV1 {
    id: MemoryPresetId,
    revision: u64,
    label: String,
    strategy: MemoryStrategyV1,
    prompt_preset: PromptPresetRefV1,
    summary_model: SummaryModelRefV1,
    summary: SummaryPolicyV1,
    insertion: InsertionPolicyV1,
    preserve_compacted_orphans: bool,
    regeneration_regex: RegenerationRegexPolicyV1,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "retrieval",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum MemoryStrategyV1 {
    RollingSummary,
    HybridRetrieval(HybridRetrievalPolicyV1),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HybridRetrievalPolicyV1 {
    query_message_count: u16,
    query_scope: EmbeddingQueryScopeV1,
    selection_mix: MemoryMixV1,
    embedding_profile: EmbeddingProfileRefV1,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EmbeddingQueryScopeV1 {
    Conversation,
    AssistantOnly,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemoryMixV1 {
    recent: u16,
    similar: u16,
    random: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PromptPresetRefV1 {
    id: PromptPresetId,
    revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "preset",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum SummaryModelRefV1 {
    ActiveChatModel,
    ModelPreset(ModelPresetRefV1),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelPresetRefV1 {
    id: ModelPresetId,
    revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmbeddingProfileRefV1 {
    id: EmbeddingProfileId,
    revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SummaryPolicyV1 {
    summary_prompt: String,
    resummary_prompt: String,
    max_messages_per_summary: u16,
    skip_user_messages: bool,
    chunk_separator: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InsertionPolicyV1 {
    memory_budget: u16,
    additional_detail_budget: u16,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RegenerationRegexPolicyV1 {
    Skip,
    ApplyPinnedPromptResponseRules,
}

pub fn serialize_preset_state(presets: &[MemoryPreset]) -> Result<Vec<u8>> {
    ensure_preset_count(presets.len())?;
    reject_duplicate_ids(presets.iter().map(MemoryPreset::id))?;
    let state = PresetStateV1 {
        presets: presets.iter().map(MemoryPresetV1::from_domain).collect(),
    };
    let envelope = StateEnvelopeOut {
        format: MEMORY_PRESET_STATE_FORMAT,
        schema_version: MEMORY_PRESET_STATE_SCHEMA_VERSION,
        state,
    };
    let mut writer = BoundedJsonWriter::new(MAX_MEMORY_PRESET_STATE_BYTES);
    if let Err(error) = serde_json::to_writer(&mut writer, &envelope) {
        if writer.exceeded() {
            return Err(MemoryError::too_large(
                "memory preset state",
                MAX_MEMORY_PRESET_STATE_BYTES,
            ));
        }
        return Err(MemoryError::Json(error));
    }
    Ok(writer.into_inner())
}

pub fn deserialize_preset_state(bytes: &[u8]) -> Result<Vec<MemoryPreset>> {
    ensure_state_size(bytes.len())?;
    let envelope: StateEnvelopeIn<'_> = serde_json::from_slice(bytes)?;
    if envelope.format != MEMORY_PRESET_STATE_FORMAT {
        return Err(MemoryError::state("unsupported memory preset state format"));
    }

    match envelope.schema_version {
        1 => decode_v1(envelope.state),
        version => Err(MemoryError::state(format!(
            "unsupported schema version {version}; expected {MEMORY_PRESET_STATE_SCHEMA_VERSION}"
        ))),
    }
}

fn decode_v1(raw: &RawValue) -> Result<Vec<MemoryPreset>> {
    let state: PresetStateIn<'_> = serde_json::from_str(raw.get())?;
    ensure_preset_count(state.presets.len())?;
    let mut ids = BTreeSet::new();
    let mut presets = Vec::with_capacity(state.presets.len());
    for raw_preset in state.presets {
        let wire: MemoryPresetV1 = serde_json::from_str(raw_preset.get())?;
        let id = wire.id.clone();
        if !ids.insert(id.clone()) {
            return Err(MemoryError::DuplicateId {
                field: "memory preset ID",
                id: id.to_string(),
            });
        }
        presets.push(wire.into_domain()?);
    }
    Ok(presets)
}

fn ensure_preset_count(count: usize) -> Result<()> {
    if count > MAX_MEMORY_PRESETS {
        return Err(MemoryError::too_many("memory presets", MAX_MEMORY_PRESETS));
    }
    Ok(())
}

fn reject_duplicate_ids<'a>(ids: impl IntoIterator<Item = &'a MemoryPresetId>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(MemoryError::DuplicateId {
                field: "memory preset ID",
                id: id.to_string(),
            });
        }
    }
    Ok(())
}

fn ensure_state_size(bytes: usize) -> Result<()> {
    if bytes > MAX_MEMORY_PRESET_STATE_BYTES {
        return Err(MemoryError::too_large(
            "memory preset state",
            MAX_MEMORY_PRESET_STATE_BYTES,
        ));
    }
    Ok(())
}

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
    exceeded: bool,
}

impl BoundedJsonWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_bytes,
            exceeded: false,
        }
    }

    const fn exceeded(&self) -> bool {
        self.exceeded
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(new_length) = self.bytes.len().checked_add(buffer.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("memory preset state exceeds its limit"));
        };
        if new_length > self.max_bytes {
            self.exceeded = true;
            return Err(io::Error::other("memory preset state exceeds its limit"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MemoryPresetV1 {
    fn from_domain(preset: &MemoryPreset) -> Self {
        Self {
            id: preset.id().clone(),
            revision: preset.revision(),
            label: preset.label().to_owned(),
            strategy: MemoryStrategyV1::from_domain(preset.strategy()),
            prompt_preset: PromptPresetRefV1::from_domain(preset.prompt_preset()),
            summary_model: SummaryModelRefV1::from_domain(preset.summary_model()),
            summary: SummaryPolicyV1::from_domain(preset.summary()),
            insertion: InsertionPolicyV1::from_domain(preset.insertion()),
            preserve_compacted_orphans: preset.preserve_compacted_orphans(),
            regeneration_regex: RegenerationRegexPolicyV1::from_domain(preset.regeneration_regex()),
        }
    }

    fn into_domain(self) -> Result<MemoryPreset> {
        MemoryPreset::from_parts(
            self.id,
            self.revision,
            MemoryPresetDraft {
                label: self.label,
                strategy: self.strategy.into_domain()?,
                prompt_preset: self.prompt_preset.into_domain()?,
                summary_model: self.summary_model.into_domain()?,
                summary: self.summary.into_domain()?,
                insertion: self.insertion.into_domain()?,
                preserve_compacted_orphans: self.preserve_compacted_orphans,
                regeneration_regex: self.regeneration_regex.into_domain(),
            },
        )
    }
}

impl MemoryStrategyV1 {
    fn from_domain(strategy: &MemoryStrategy) -> Self {
        match strategy {
            MemoryStrategy::RollingSummary => Self::RollingSummary,
            MemoryStrategy::HybridRetrieval(policy) => {
                Self::HybridRetrieval(HybridRetrievalPolicyV1::from_domain(policy))
            }
        }
    }

    fn into_domain(self) -> Result<MemoryStrategy> {
        match self {
            Self::RollingSummary => Ok(MemoryStrategy::RollingSummary),
            Self::HybridRetrieval(policy) => {
                Ok(MemoryStrategy::HybridRetrieval(policy.into_domain()?))
            }
        }
    }
}

impl HybridRetrievalPolicyV1 {
    fn from_domain(policy: &HybridRetrievalPolicy) -> Self {
        Self {
            query_message_count: policy.query_message_count(),
            query_scope: EmbeddingQueryScopeV1::from_domain(policy.query_scope()),
            selection_mix: MemoryMixV1::from_domain(policy.selection_mix()),
            embedding_profile: EmbeddingProfileRefV1::from_domain(policy.embedding_profile()),
        }
    }

    fn into_domain(self) -> Result<HybridRetrievalPolicy> {
        HybridRetrievalPolicy::new(
            self.query_message_count,
            self.query_scope.into_domain(),
            self.selection_mix.into_domain()?,
            self.embedding_profile.into_domain()?,
        )
    }
}

impl EmbeddingQueryScopeV1 {
    const fn from_domain(scope: EmbeddingQueryScope) -> Self {
        match scope {
            EmbeddingQueryScope::Conversation => Self::Conversation,
            EmbeddingQueryScope::AssistantOnly => Self::AssistantOnly,
        }
    }

    const fn into_domain(self) -> EmbeddingQueryScope {
        match self {
            Self::Conversation => EmbeddingQueryScope::Conversation,
            Self::AssistantOnly => EmbeddingQueryScope::AssistantOnly,
        }
    }
}

impl MemoryMixV1 {
    fn from_domain(mix: MemoryMix) -> Self {
        Self {
            recent: mix.recent().get(),
            similar: mix.similar().get(),
            random: mix.random().get(),
        }
    }

    fn into_domain(self) -> Result<MemoryMix> {
        MemoryMix::new(
            BasisPoints::new(self.recent)?,
            BasisPoints::new(self.similar)?,
            BasisPoints::new(self.random)?,
        )
    }
}

impl PromptPresetRefV1 {
    fn from_domain(reference: &PromptPresetRef) -> Self {
        Self {
            id: reference.id().clone(),
            revision: reference.revision(),
        }
    }

    fn into_domain(self) -> Result<PromptPresetRef> {
        PromptPresetRef::new(self.id, self.revision)
    }
}

impl SummaryModelRefV1 {
    fn from_domain(reference: &SummaryModelRef) -> Self {
        match reference {
            SummaryModelRef::ActiveChatModel => Self::ActiveChatModel,
            SummaryModelRef::ModelPreset(preset) => {
                Self::ModelPreset(ModelPresetRefV1::from_domain(preset))
            }
        }
    }

    fn into_domain(self) -> Result<SummaryModelRef> {
        match self {
            Self::ActiveChatModel => Ok(SummaryModelRef::ActiveChatModel),
            Self::ModelPreset(preset) => Ok(SummaryModelRef::ModelPreset(preset.into_domain()?)),
        }
    }
}

impl ModelPresetRefV1 {
    fn from_domain(reference: &ModelPresetRef) -> Self {
        Self {
            id: reference.id().clone(),
            revision: reference.revision(),
        }
    }

    fn into_domain(self) -> Result<ModelPresetRef> {
        ModelPresetRef::new(self.id, self.revision)
    }
}

impl EmbeddingProfileRefV1 {
    fn from_domain(reference: &EmbeddingProfileRef) -> Self {
        Self {
            id: reference.id().clone(),
            revision: reference.revision(),
        }
    }

    fn into_domain(self) -> Result<EmbeddingProfileRef> {
        EmbeddingProfileRef::new(self.id, self.revision)
    }
}

impl SummaryPolicyV1 {
    fn from_domain(policy: &SummaryPolicy) -> Self {
        Self {
            summary_prompt: policy.summary_prompt().to_owned(),
            resummary_prompt: policy.resummary_prompt().to_owned(),
            max_messages_per_summary: policy.max_messages_per_summary(),
            skip_user_messages: policy.skip_user_messages(),
            chunk_separator: policy.chunk_separator().pattern().to_owned(),
        }
    }

    fn into_domain(self) -> Result<SummaryPolicy> {
        SummaryPolicy::new(
            self.summary_prompt,
            self.resummary_prompt,
            self.max_messages_per_summary,
            self.skip_user_messages,
            ChunkSeparatorRegex::new(self.chunk_separator)?,
        )
    }
}

impl InsertionPolicyV1 {
    fn from_domain(policy: InsertionPolicy) -> Self {
        Self {
            memory_budget: policy.memory_budget().get(),
            additional_detail_budget: policy.additional_detail_budget().get(),
        }
    }

    fn into_domain(self) -> Result<InsertionPolicy> {
        InsertionPolicy::new(
            BasisPoints::new(self.memory_budget)?,
            BasisPoints::new(self.additional_detail_budget)?,
        )
    }
}

impl RegenerationRegexPolicyV1 {
    const fn from_domain(policy: RegenerationRegexPolicy) -> Self {
        match policy {
            RegenerationRegexPolicy::Skip => Self::Skip,
            RegenerationRegexPolicy::ApplyPinnedPromptResponseRules => {
                Self::ApplyPinnedPromptResponseRules
            }
        }
    }

    const fn into_domain(self) -> RegenerationRegexPolicy {
        match self {
            Self::Skip => RegenerationRegexPolicy::Skip,
            Self::ApplyPinnedPromptResponseRules => {
                RegenerationRegexPolicy::ApplyPinnedPromptResponseRules
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{Value, json};

    use super::*;

    fn rolling_preset() -> MemoryPreset {
        MemoryPreset::create(
            MemoryPresetId::parse("memory.rolling").unwrap(),
            MemoryPresetDraft {
                label: "Rolling memory".to_owned(),
                strategy: MemoryStrategy::RollingSummary,
                prompt_preset: PromptPresetRef::new(
                    PromptPresetId::parse("prompt.main").unwrap(),
                    3,
                )
                .unwrap(),
                summary_model: SummaryModelRef::ActiveChatModel,
                summary: SummaryPolicy::new(
                    "Summarize the supplied messages.",
                    "Merge the prior summary with the supplied messages.",
                    32,
                    false,
                    ChunkSeparatorRegex::new(r"\n{2,}").unwrap(),
                )
                .unwrap(),
                insertion: InsertionPolicy::new(
                    BasisPoints::new(2_500).unwrap(),
                    BasisPoints::new(2_000).unwrap(),
                )
                .unwrap(),
                preserve_compacted_orphans: true,
                regeneration_regex: RegenerationRegexPolicy::Skip,
            },
        )
        .unwrap()
    }

    fn hybrid_preset() -> MemoryPreset {
        let mix = MemoryMix::new(
            BasisPoints::new(4_000).unwrap(),
            BasisPoints::new(4_000).unwrap(),
            BasisPoints::new(2_000).unwrap(),
        )
        .unwrap();
        let retrieval = HybridRetrievalPolicy::new(
            12,
            EmbeddingQueryScope::AssistantOnly,
            mix,
            EmbeddingProfileRef::new(EmbeddingProfileId::parse("embedding.local").unwrap(), 2)
                .unwrap(),
        )
        .unwrap();
        let draft = MemoryPresetDraft {
            label: "Hybrid memory".to_owned(),
            strategy: MemoryStrategy::HybridRetrieval(retrieval),
            prompt_preset: PromptPresetRef::new(PromptPresetId::parse("prompt.hybrid").unwrap(), 4)
                .unwrap(),
            summary_model: SummaryModelRef::ModelPreset(
                ModelPresetRef::new(ModelPresetId::parse("model.helper").unwrap(), 7).unwrap(),
            ),
            summary: SummaryPolicy::new(
                "Create a bounded memory record.",
                "Consolidate the existing record with new messages.",
                16,
                true,
                ChunkSeparatorRegex::new(r"\n-{3,}\n").unwrap(),
            )
            .unwrap(),
            insertion: InsertionPolicy::new(
                BasisPoints::new(3_000).unwrap(),
                BasisPoints::new(3_500).unwrap(),
            )
            .unwrap(),
            preserve_compacted_orphans: false,
            regeneration_regex: RegenerationRegexPolicy::ApplyPinnedPromptResponseRules,
        };
        MemoryPreset::create(MemoryPresetId::parse("memory.hybrid").unwrap(), draft)
            .unwrap()
            .updated(
                1,
                MemoryPresetDraft {
                    label: "Hybrid memory v2".to_owned(),
                    ..hybrid_preset_draft_for_update()
                },
            )
            .unwrap()
    }

    fn hybrid_preset_draft_for_update() -> MemoryPresetDraft {
        let mix = MemoryMix::new(
            BasisPoints::new(4_000).unwrap(),
            BasisPoints::new(4_000).unwrap(),
            BasisPoints::new(2_000).unwrap(),
        )
        .unwrap();
        MemoryPresetDraft {
            label: "Hybrid memory".to_owned(),
            strategy: MemoryStrategy::HybridRetrieval(
                HybridRetrievalPolicy::new(
                    12,
                    EmbeddingQueryScope::AssistantOnly,
                    mix,
                    EmbeddingProfileRef::new(
                        EmbeddingProfileId::parse("embedding.local").unwrap(),
                        2,
                    )
                    .unwrap(),
                )
                .unwrap(),
            ),
            prompt_preset: PromptPresetRef::new(PromptPresetId::parse("prompt.hybrid").unwrap(), 4)
                .unwrap(),
            summary_model: SummaryModelRef::ModelPreset(
                ModelPresetRef::new(ModelPresetId::parse("model.helper").unwrap(), 7).unwrap(),
            ),
            summary: SummaryPolicy::new(
                "Create a bounded memory record.",
                "Consolidate the existing record with new messages.",
                16,
                true,
                ChunkSeparatorRegex::new(r"\n-{3,}\n").unwrap(),
            )
            .unwrap(),
            insertion: InsertionPolicy::new(
                BasisPoints::new(3_000).unwrap(),
                BasisPoints::new(3_500).unwrap(),
            )
            .unwrap(),
            preserve_compacted_orphans: false,
            regeneration_regex: RegenerationRegexPolicy::ApplyPinnedPromptResponseRules,
        }
    }

    #[test]
    fn v1_round_trip_reconstructs_validated_presets() {
        let expected = vec![rolling_preset(), hybrid_preset()];
        let bytes = serialize_preset_state(&expected).unwrap();
        let actual = deserialize_preset_state(&bytes).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual[1].revision(), 2);
    }

    #[test]
    fn serialized_state_contains_only_preset_configuration_keys() {
        let value: Value =
            serde_json::from_slice(&serialize_preset_state(&[hybrid_preset()]).unwrap()).unwrap();
        let mut keys = BTreeSet::new();
        collect_object_keys(&value, &mut keys);

        for forbidden in [
            "apiKey",
            "credential",
            "credentialRef",
            "binding",
            "artifacts",
            "embeddingVector",
        ] {
            assert!(
                !keys.contains(forbidden),
                "serialized forbidden key {forbidden}"
            );
        }
    }

    #[test]
    fn api_key_and_other_unknown_fields_are_rejected() {
        let mut value: Value =
            serde_json::from_slice(&serialize_preset_state(&[rolling_preset()]).unwrap()).unwrap();
        value["state"]["presets"][0]["apiKey"] = Value::String("must-not-load".to_owned());
        let error = deserialize_preset_state(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn future_version_is_rejected_before_decoding_its_state() {
        let value = json!({
            "format": MEMORY_PRESET_STATE_FORMAT,
            "schemaVersion": 2,
            "state": {"apiKey": "must-not-be-decoded"}
        });
        let error = deserialize_preset_state(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.to_string().contains("unsupported schema version 2"));
    }

    #[test]
    fn duplicate_envelope_fields_are_rejected() {
        let bytes = br#"{
          "format":"lorepia.memory-preset-state",
          "schemaVersion":1,
          "schemaVersion":1,
          "state":{"presets":[]}
        }"#;
        let error = deserialize_preset_state(bytes).unwrap_err();
        assert!(error.to_string().contains("duplicate field"));
    }

    #[test]
    fn oversized_state_is_rejected_before_json_parsing() {
        let bytes = vec![b' '; MAX_MEMORY_PRESET_STATE_BYTES + 1];
        assert!(matches!(
            deserialize_preset_state(&bytes),
            Err(MemoryError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn oversized_serialized_state_stops_at_the_output_bound() {
        let presets = (0..4)
            .map(|index| {
                MemoryPreset::create(
                    MemoryPresetId::parse(format!("large-memory-{index}")).unwrap(),
                    MemoryPresetDraft {
                        label: format!("Large memory {index}"),
                        strategy: MemoryStrategy::RollingSummary,
                        prompt_preset: PromptPresetRef::new(
                            PromptPresetId::parse("prompt.main").unwrap(),
                            1,
                        )
                        .unwrap(),
                        summary_model: SummaryModelRef::ActiveChatModel,
                        summary: SummaryPolicy::new(
                            "s".repeat(crate::MAX_SUMMARY_PROMPT_BYTES),
                            "r".repeat(crate::MAX_SUMMARY_PROMPT_BYTES),
                            1,
                            false,
                            ChunkSeparatorRegex::new(r"\n{2,}").unwrap(),
                        )
                        .unwrap(),
                        insertion: InsertionPolicy::new(
                            BasisPoints::new(1).unwrap(),
                            BasisPoints::new(0).unwrap(),
                        )
                        .unwrap(),
                        preserve_compacted_orphans: false,
                        regeneration_regex: RegenerationRegexPolicy::Skip,
                    },
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            serialize_preset_state(&presets),
            Err(MemoryError::PayloadTooLarge { field, max_bytes })
                if field == "memory preset state"
                    && max_bytes == MAX_MEMORY_PRESET_STATE_BYTES
        ));
    }

    #[test]
    fn near_limit_compact_state_round_trips_without_pretty_print_expansion() {
        let presets = (0..4)
            .map(|index| {
                MemoryPreset::create(
                    MemoryPresetId::parse(format!("boundary-memory-{index}")).unwrap(),
                    MemoryPresetDraft {
                        label: format!("Boundary memory {index}"),
                        strategy: MemoryStrategy::RollingSummary,
                        prompt_preset: PromptPresetRef::new(
                            PromptPresetId::parse("prompt.main").unwrap(),
                            1,
                        )
                        .unwrap(),
                        summary_model: SummaryModelRef::ActiveChatModel,
                        summary: SummaryPolicy::new(
                            "s".repeat(64_000),
                            "r".repeat(64_000),
                            1,
                            false,
                            ChunkSeparatorRegex::new(r"\n{2,}").unwrap(),
                        )
                        .unwrap(),
                        insertion: InsertionPolicy::new(
                            BasisPoints::new(1).unwrap(),
                            BasisPoints::new(0).unwrap(),
                        )
                        .unwrap(),
                        preserve_compacted_orphans: false,
                        regeneration_regex: RegenerationRegexPolicy::Skip,
                    },
                )
                .unwrap()
            })
            .collect::<Vec<_>>();

        let first = serialize_preset_state(&presets).unwrap();
        assert!(first.len() > 500_000);
        let decoded = deserialize_preset_state(&first).unwrap();
        assert_eq!(decoded, presets);
        assert_eq!(serialize_preset_state(&decoded).unwrap(), first);
    }

    #[test]
    fn preset_count_is_bounded_on_write_and_read() {
        let presets = (0..=MAX_MEMORY_PRESETS)
            .map(|index| {
                let mut preset = rolling_preset();
                preset = MemoryPreset::create(
                    MemoryPresetId::parse(format!("memory-{index}")).unwrap(),
                    MemoryPresetDraft {
                        label: format!("Memory {index}"),
                        strategy: preset.strategy().clone(),
                        prompt_preset: preset.prompt_preset().clone(),
                        summary_model: preset.summary_model().clone(),
                        summary: preset.summary().clone(),
                        insertion: preset.insertion(),
                        preserve_compacted_orphans: preset.preserve_compacted_orphans(),
                        regeneration_regex: preset.regeneration_regex(),
                    },
                )
                .unwrap();
                preset
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            serialize_preset_state(&presets),
            Err(MemoryError::TooManyItems { field, max })
                if field == "memory presets" && max == MAX_MEMORY_PRESETS
        ));

        let encoded = serialize_preset_state(&[rolling_preset()]).unwrap();
        let encoded: Value = serde_json::from_slice(&encoded).unwrap();
        let wire_preset = encoded["state"]["presets"][0].clone();
        let state = json!({
            "format": MEMORY_PRESET_STATE_FORMAT,
            "schemaVersion": 1,
            "state": {
                "presets": (0..=MAX_MEMORY_PRESETS)
                    .map(|_| wire_preset.clone())
                    .collect::<Vec<_>>()
            }
        });
        let error = deserialize_preset_state(&serde_json::to_vec(&state).unwrap()).unwrap_err();
        assert!(matches!(
            error,
            MemoryError::TooManyItems { field, max }
                if field == "memory presets" && max == MAX_MEMORY_PRESETS
        ));
    }

    fn collect_object_keys<'a>(value: &'a Value, keys: &mut BTreeSet<&'a str>) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    keys.insert(key);
                    collect_object_keys(value, keys);
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect_object_keys(value, keys);
                }
            }
            _ => {}
        }
    }
}
