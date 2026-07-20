use serde::Serialize;

use crate::{BASIS_POINTS_SCALE, MemoryError, MemoryPreset, MemoryStrategy, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBudgetInput {
    max_context_tokens: u32,
    max_output_tokens: u32,
    exact_non_memory_tokens: u32,
}

impl MemoryBudgetInput {
    pub fn new(
        max_context_tokens: u32,
        max_output_tokens: u32,
        exact_non_memory_tokens: u32,
    ) -> Result<Self> {
        if max_context_tokens == 0
            || max_output_tokens == 0
            || max_output_tokens > max_context_tokens
        {
            return Err(MemoryError::invalid(
                "memoryBudgetInput",
                "context and output limits must be positive and output must fit context",
            ));
        }
        let input_capacity = max_context_tokens - max_output_tokens;
        if exact_non_memory_tokens > input_capacity {
            return Err(MemoryError::invalid(
                "exactNonMemoryTokens",
                "must fit the context remaining after max output",
            ));
        }
        Ok(Self {
            max_context_tokens,
            max_output_tokens,
            exact_non_memory_tokens,
        })
    }

    #[must_use]
    pub const fn max_context_tokens(self) -> u32 {
        self.max_context_tokens
    }

    #[must_use]
    pub const fn max_output_tokens(self) -> u32 {
        self.max_output_tokens
    }

    #[must_use]
    pub const fn exact_non_memory_tokens(self) -> u32 {
        self.exact_non_memory_tokens
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetrievalTokenBudget {
    recent: u32,
    similar: u32,
    random: u32,
}

impl RetrievalTokenBudget {
    #[must_use]
    pub const fn recent(self) -> u32 {
        self.recent
    }

    #[must_use]
    pub const fn similar(self) -> u32 {
        self.similar
    }

    #[must_use]
    pub const fn random(self) -> u32 {
        self.random
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryTokenBudget {
    input_capacity: u32,
    remaining_after_non_memory: u32,
    total_memory: u32,
    consolidated_summary: u32,
    additional_detail: u32,
    retrieval: RetrievalTokenBudget,
}

impl MemoryTokenBudget {
    pub fn calculate(preset: &MemoryPreset, input: MemoryBudgetInput) -> Self {
        let input_capacity = input.max_context_tokens - input.max_output_tokens;
        let remaining_after_non_memory = input_capacity - input.exact_non_memory_tokens;
        let ratio_cap = scale_tokens(input_capacity, preset.insertion().memory_budget().get());
        let total_memory = ratio_cap.min(remaining_after_non_memory);
        let additional_detail = scale_tokens(
            total_memory,
            preset.insertion().additional_detail_budget().get(),
        );
        let consolidated_summary = total_memory - additional_detail;
        let retrieval = match preset.strategy() {
            MemoryStrategy::RollingSummary => RetrievalTokenBudget {
                recent: additional_detail,
                similar: 0,
                random: 0,
            },
            MemoryStrategy::HybridRetrieval(policy) => {
                let mix = policy.selection_mix();
                let shares = apportion(
                    additional_detail,
                    [mix.recent().get(), mix.similar().get(), mix.random().get()],
                );
                RetrievalTokenBudget {
                    recent: shares[0],
                    similar: shares[1],
                    random: shares[2],
                }
            }
        };
        Self {
            input_capacity,
            remaining_after_non_memory,
            total_memory,
            consolidated_summary,
            additional_detail,
            retrieval,
        }
    }

    #[must_use]
    pub const fn input_capacity(self) -> u32 {
        self.input_capacity
    }

    #[must_use]
    pub const fn remaining_after_non_memory(self) -> u32 {
        self.remaining_after_non_memory
    }

    #[must_use]
    pub const fn total_memory(self) -> u32 {
        self.total_memory
    }

    #[must_use]
    pub const fn consolidated_summary(self) -> u32 {
        self.consolidated_summary
    }

    #[must_use]
    pub const fn additional_detail(self) -> u32 {
        self.additional_detail
    }

    #[must_use]
    pub const fn retrieval(self) -> RetrievalTokenBudget {
        self.retrieval
    }
}

fn scale_tokens(tokens: u32, basis_points: u16) -> u32 {
    let scaled = u64::from(tokens) * u64::from(basis_points);
    u32::try_from(scaled / u64::from(BASIS_POINTS_SCALE))
        .expect("basis-point scaling of a u32 fits u32")
}

fn apportion(total: u32, weights: [u16; 3]) -> [u32; 3] {
    let scale = u64::from(BASIS_POINTS_SCALE);
    let mut shares = [0u32; 3];
    let mut remainders = [(0u64, 0usize); 3];
    let mut assigned = 0u32;
    for (index, weight) in weights.into_iter().enumerate() {
        let product = u64::from(total) * u64::from(weight);
        shares[index] = u32::try_from(product / scale).expect("share fits total");
        assigned += shares[index];
        remainders[index] = (product % scale, index);
    }
    remainders.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    for (_, index) in remainders
        .into_iter()
        .take(usize::try_from(total - assigned).expect("remainder count fits usize"))
    {
        shares[index] += 1;
    }
    shares
}

#[cfg(test)]
mod tests {
    use crate::{
        BasisPoints, ChunkSeparatorRegex, EmbeddingProfileId, EmbeddingProfileRef,
        EmbeddingQueryScope, HybridRetrievalPolicy, InsertionPolicy, MemoryMix, MemoryPreset,
        MemoryPresetDraft, MemoryPresetId, ModelPresetId, ModelPresetRef, PromptPresetId,
        PromptPresetRef, RegenerationRegexPolicy, SummaryModelRef, SummaryPolicy,
    };

    use super::*;

    fn preset() -> MemoryPreset {
        let mix = MemoryMix::new(
            BasisPoints::new(3_333).unwrap(),
            BasisPoints::new(3_333).unwrap(),
            BasisPoints::new(3_334).unwrap(),
        )
        .unwrap();
        MemoryPreset::create(
            MemoryPresetId::parse("memory").unwrap(),
            MemoryPresetDraft {
                label: "Memory".to_owned(),
                strategy: MemoryStrategy::HybridRetrieval(
                    HybridRetrievalPolicy::new(
                        8,
                        EmbeddingQueryScope::Conversation,
                        mix,
                        EmbeddingProfileRef::new(
                            EmbeddingProfileId::parse("embedding").unwrap(),
                            2,
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ),
                prompt_preset: PromptPresetRef::new(PromptPresetId::parse("prompt").unwrap(), 1)
                    .unwrap(),
                summary_model: SummaryModelRef::ModelPreset(
                    ModelPresetRef::new(ModelPresetId::parse("helper").unwrap(), 3).unwrap(),
                ),
                summary: SummaryPolicy::new(
                    "Summarize.",
                    "Consolidate.",
                    32,
                    false,
                    ChunkSeparatorRegex::new(r"\n{2,}").unwrap(),
                )
                .unwrap(),
                insertion: InsertionPolicy::new(
                    BasisPoints::new(2_500).unwrap(),
                    BasisPoints::new(4_000).unwrap(),
                )
                .unwrap(),
                preserve_compacted_orphans: true,
                regeneration_regex: RegenerationRegexPolicy::Skip,
            },
        )
        .unwrap()
    }

    #[test]
    fn budget_uses_context_minus_output_and_non_memory_exact_count() {
        let budget = MemoryTokenBudget::calculate(
            &preset(),
            MemoryBudgetInput::new(10_000, 2_000, 7_000).unwrap(),
        );
        assert_eq!(budget.input_capacity, 8_000);
        assert_eq!(budget.remaining_after_non_memory, 1_000);
        assert_eq!(budget.total_memory, 1_000);
        assert_eq!(budget.consolidated_summary, 600);
        assert_eq!(budget.additional_detail, 400);
        assert_eq!(
            budget.retrieval.recent + budget.retrieval.similar + budget.retrieval.random,
            400
        );
    }

    #[test]
    fn invalid_non_memory_count_is_rejected_before_subtraction() {
        assert!(MemoryBudgetInput::new(100, 20, 81).is_err());
    }
}
