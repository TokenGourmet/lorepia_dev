use std::io::{self, Write};

use lorepia_persona::{CharacterCardId, ChatId};
use lorepia_prompt::{PromptPreset, validate_preset};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{MemoryError, MemoryPreset, ModelPresetRef, Result, SummaryModelRef};

/// Native-owned immutable snapshot for one chat memory generation.
///
/// This type is serialize-only: portable input cannot construct a binding or
/// choose its own model/preset revisions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySessionBinding {
    chat_id: ChatId,
    character_card_id: CharacterCardId,
    memory_generation: u64,
    preset: MemoryPreset,
    resolved_summary_model: ModelPresetRef,
    #[serde(skip)]
    prompt_preset_digest: [u8; 32],
}

impl MemorySessionBinding {
    pub fn new(
        chat_id: ChatId,
        character_card_id: CharacterCardId,
        memory_generation: u64,
        preset: &MemoryPreset,
        resolved_summary_model: ModelPresetRef,
        resolved_prompt_preset: &PromptPreset,
    ) -> Result<Self> {
        if memory_generation == 0 {
            return Err(MemoryError::invalid(
                "memoryGeneration",
                "must be at least 1",
            ));
        }
        if let SummaryModelRef::ModelPreset(expected) = preset.summary_model()
            && expected != &resolved_summary_model
        {
            return Err(MemoryError::invalid(
                "resolvedSummaryModel",
                "must match the model preset pinned by the memory preset",
            ));
        }
        validate_preset(resolved_prompt_preset)?;
        let prompt_preset_digest = digest_prompt_preset(resolved_prompt_preset)?;
        Ok(Self {
            chat_id,
            character_card_id,
            memory_generation,
            preset: preset.clone(),
            resolved_summary_model,
            prompt_preset_digest,
        })
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
    pub const fn preset(&self) -> &MemoryPreset {
        &self.preset
    }

    #[must_use]
    pub const fn resolved_summary_model(&self) -> &ModelPresetRef {
        &self.resolved_summary_model
    }

    pub(crate) fn verify_prompt_preset(&self, preset: &PromptPreset) -> Result<()> {
        validate_preset(preset)?;
        if digest_prompt_preset(preset)? != self.prompt_preset_digest {
            return Err(MemoryError::RetrievalUnavailable(
                "resolved prompt preset content differs from the session snapshot",
            ));
        }
        Ok(())
    }

    pub(crate) fn verify_scope(
        &self,
        expected_chat_id: &ChatId,
        expected_character_card_id: &CharacterCardId,
    ) -> Result<()> {
        if self.chat_id != *expected_chat_id {
            return Err(MemoryError::BindingMismatch {
                field: "chatId",
                expected: expected_chat_id.to_string(),
                actual: self.chat_id.to_string(),
            });
        }
        if self.character_card_id != *expected_character_card_id {
            return Err(MemoryError::BindingMismatch {
                field: "characterCardId",
                expected: expected_character_card_id.to_string(),
                actual: self.character_card_id.to_string(),
            });
        }
        Ok(())
    }
}

fn digest_prompt_preset(preset: &PromptPreset) -> Result<[u8; 32]> {
    let mut writer = DigestWriter(Sha256::new());
    serde_json::to_writer(&mut writer, preset)?;
    Ok(writer.0.finalize().into())
}

struct DigestWriter(Sha256);

impl Write for DigestWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
