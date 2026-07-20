#![forbid(unsafe_code)]

mod catalog;
mod error;
mod id;
mod storage;

pub use catalog::{
    CharacterPersonaPolicy, PersonaCatalog, PersonaChoice, PersonaDraft, PersonaPlayBinding,
    PersonaRecord, PersonaSelectionSource, PersonaSnapshot,
};
pub use error::{PersonaError, Result};
pub use id::{CharacterCardId, ChatId, PersonaId};
pub use storage::{
    PERSONA_STATE_FORMAT, PERSONA_STATE_SCHEMA_VERSION, deserialize_state, serialize_state,
};

pub const MAX_PERSONAS: usize = 256;
pub const MAX_CHARACTER_POLICIES: usize = 4_096;
pub const MAX_PERSONA_LABEL_BYTES: usize = 256;
pub const MAX_PERSONA_PROMPT_BYTES: usize = lorepia_prompt::MAX_PERSONA_INPUT_BYTES;
pub const MAX_TOTAL_PERSONA_PROMPT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PERSONA_STATE_BYTES: usize = 16 * 1024 * 1024;
