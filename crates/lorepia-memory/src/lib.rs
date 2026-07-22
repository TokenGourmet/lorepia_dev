#![forbid(unsafe_code)]

mod artifact;
mod budget;
mod chunk;
mod config;
mod error;
mod id;
mod retrieval;
mod session;
mod storage;
mod summary;

pub use artifact::{
    EmbeddingArtifact, EmbeddingVector, MemoryValidity, SimilarityScore, SummaryArtifact,
    SummaryArtifactKind, cosine_similarity,
};
pub use budget::{MemoryBudgetInput, MemoryTokenBudget, RetrievalTokenBudget};
pub use chunk::{MemoryChunk, split_memory_chunks};
pub use config::{
    BasisPoints, ChunkSeparatorRegex, EmbeddingQueryScope, HybridRetrievalPolicy, InsertionPolicy,
    MemoryMix, MemoryPreset, MemoryPresetDraft, MemoryStrategy, RegenerationRegexPolicy,
    SummaryModelRef, SummaryPolicy,
};
pub use error::{MemoryError, Result};
pub use id::{
    EmbeddingProfileId, EmbeddingProfileRef, MemoryArtifactId, MemoryPresetId, MessageId,
    ModelPresetId, ModelPresetRef, PromptPresetId, PromptPresetRef, VersionedRef,
};
pub use retrieval::{
    AuthoritativeEmbeddingQuery, EmbeddingMatch, MemoryCandidate, MemorySelection,
    MemorySelectionBucket, SelectedMemory, select_memories,
};
pub use session::MemorySessionBinding;
pub use storage::{
    MEMORY_PRESET_STATE_FORMAT, MEMORY_PRESET_STATE_SCHEMA_VERSION, deserialize_preset_state,
    serialize_preset_state,
};
pub use summary::{
    AuthoritativeSummarySource, EmbeddingQueryPlan, MemorySourceMessage, ProcessedSummary,
    SummaryJob, SummaryJobKind, SummaryJobSource, SummaryOutputCandidate, SummaryOutputMode,
    build_embedding_query, plan_initial_summary_jobs, plan_resummary_job,
    process_regenerated_summary, process_summary_output,
};

pub const BASIS_POINTS_SCALE: u16 = 10_000;
pub const MAX_MEMORY_PRESET_STATE_BYTES: usize = 512 * 1024;
pub const MAX_MEMORY_PRESETS: usize = 256;
pub const MAX_MEMORY_LABEL_BYTES: usize = 256;
pub const MAX_SUMMARY_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_CHUNK_SEPARATOR_REGEX_BYTES: usize = 4 * 1024;
pub const MAX_MESSAGES_PER_SUMMARY: u16 = 256;
pub const MAX_QUERY_MESSAGE_COUNT: u16 = 128;
pub const MAX_SOURCE_MESSAGES: usize = 4_096;
pub const MAX_SOURCE_MESSAGE_BYTES: usize = 256 * 1024;
pub const MAX_SOURCE_BATCH_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SUMMARY_PLAN_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MEMORY_ARTIFACT_BYTES: usize = 256 * 1024;
pub const MAX_MEMORY_CANDIDATES: usize = 4_096;
pub const MAX_MEMORY_CHUNKS: usize = 2_048;
pub const MAX_EMBEDDING_DIMENSIONS: usize = 8_192;
pub const MAX_LONG_TERM_MEMORY_INPUT_BYTES: usize =
    lorepia_prompt::MAX_LONG_TERM_MEMORY_INPUT_BYTES;
