use std::collections::BTreeMap;

use lorepia_prompt::PromptCompileInput;
use serde::Serialize;

use crate::{
    CharacterCardId, ChatId, MAX_CHARACTER_POLICIES, MAX_PERSONA_LABEL_BYTES,
    MAX_PERSONA_PROMPT_BYTES, MAX_PERSONAS, MAX_TOTAL_PERSONA_PROMPT_BYTES, PersonaError,
    PersonaId, Result,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaDraft {
    label: String,
    prompt_text: String,
}

impl PersonaDraft {
    pub fn new(label: impl Into<String>, prompt_text: impl Into<String>) -> Result<Self> {
        let label = label.into();
        let prompt_text = prompt_text.into();
        validate_label(&label)?;
        validate_prompt_text(&prompt_text)?;
        Ok(Self { label, prompt_text })
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn prompt_text(&self) -> &str {
        &self.prompt_text
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonaRecord {
    pub(crate) id: PersonaId,
    pub(crate) revision: u64,
    pub(crate) label: String,
    pub(crate) prompt_text: String,
}

impl PersonaRecord {
    #[must_use]
    pub const fn id(&self) -> &PersonaId {
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
    pub fn prompt_text(&self) -> &str {
        &self.prompt_text
    }

    fn snapshot(&self) -> PersonaSnapshot {
        PersonaSnapshot {
            id: self.id.clone(),
            revision: self.revision,
            label: self.label.clone(),
            prompt_text: self.prompt_text.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaSnapshot {
    id: PersonaId,
    revision: u64,
    label: String,
    prompt_text: String,
}

impl PersonaSnapshot {
    #[must_use]
    pub const fn id(&self) -> &PersonaId {
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
    pub fn prompt_text(&self) -> &str {
        &self.prompt_text
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersonaChoice {
    UseDefault,
    Specific(PersonaId),
    NoPersona,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CharacterPersonaPolicy {
    InheritGlobal,
    Specific(PersonaId),
    NoPersona,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaSelectionSource {
    Explicit,
    CharacterDefault,
    GlobalDefault,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolvedPersona {
    source: PersonaSelectionSource,
    snapshot: PersonaSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonaPlayBinding {
    chat_id: ChatId,
    character_card_id: CharacterCardId,
    persona: Option<ResolvedPersona>,
}

impl PersonaPlayBinding {
    #[must_use]
    pub const fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }

    #[must_use]
    pub const fn character_card_id(&self) -> &CharacterCardId {
        &self.character_card_id
    }

    #[must_use]
    pub fn persona(&self) -> Option<&PersonaSnapshot> {
        self.persona.as_ref().map(|resolved| &resolved.snapshot)
    }

    #[must_use]
    pub fn source(&self) -> Option<PersonaSelectionSource> {
        self.persona.as_ref().map(|resolved| resolved.source)
    }

    #[must_use]
    pub fn prompt_text(&self) -> Option<&str> {
        self.persona().map(PersonaSnapshot::prompt_text)
    }

    /// Replaces, rather than appends to, any previous persona input, but only
    /// after both native-owned play identities match this immutable binding.
    pub fn apply_to_prompt_input(
        &self,
        expected_chat_id: &ChatId,
        expected_character_card_id: &CharacterCardId,
        input: &mut PromptCompileInput,
    ) -> Result<()> {
        if self.chat_id != *expected_chat_id {
            return Err(PersonaError::BindingMismatch {
                field: "chatId",
                expected: expected_chat_id.to_string(),
                actual: self.chat_id.to_string(),
            });
        }
        if self.character_card_id != *expected_character_card_id {
            return Err(PersonaError::BindingMismatch {
                field: "characterCardId",
                expected: expected_character_card_id.to_string(),
                actual: self.character_card_id.to_string(),
            });
        }
        input.persona.clear();
        if let Some(prompt_text) = self.prompt_text() {
            input.persona.push_str(prompt_text);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PersonaCatalog {
    pub(crate) generation: u64,
    pub(crate) personas: BTreeMap<PersonaId, PersonaRecord>,
    pub(crate) global_default: Option<PersonaId>,
    /// An absent key inherits the global default. Stored entries are explicit
    /// per-character overrides, including an explicit no-persona choice.
    pub(crate) character_policies: BTreeMap<CharacterCardId, StoredCharacterPolicy>,
    pub(crate) total_prompt_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StoredCharacterPolicy {
    Specific(PersonaId),
    NoPersona,
}

impl PersonaCatalog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.personas.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.personas.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: &PersonaId) -> Option<&PersonaRecord> {
        self.personas.get(id)
    }

    pub fn personas(&self) -> impl ExactSizeIterator<Item = &PersonaRecord> {
        self.personas.values()
    }

    #[must_use]
    pub const fn global_default(&self) -> Option<&PersonaId> {
        self.global_default.as_ref()
    }

    #[must_use]
    pub fn character_policy(&self, character_id: &CharacterCardId) -> CharacterPersonaPolicy {
        match self.character_policies.get(character_id) {
            Some(StoredCharacterPolicy::Specific(id)) => {
                CharacterPersonaPolicy::Specific(id.clone())
            }
            Some(StoredCharacterPolicy::NoPersona) => CharacterPersonaPolicy::NoPersona,
            None => CharacterPersonaPolicy::InheritGlobal,
        }
    }

    pub fn add(&mut self, id: PersonaId, draft: PersonaDraft) -> Result<PersonaRecord> {
        if self.personas.contains_key(&id) {
            return Err(PersonaError::AlreadyExists {
                kind: "persona",
                id: id.to_string(),
            });
        }
        if self.personas.len() >= MAX_PERSONAS {
            return Err(PersonaError::too_many("personas", MAX_PERSONAS));
        }
        let total_prompt_bytes =
            checked_total_after_replace(self.total_prompt_bytes, 0, draft.prompt_text.len())?;
        let generation = next_generation(self.generation)?;
        let record = PersonaRecord {
            id: id.clone(),
            revision: 1,
            label: draft.label,
            prompt_text: draft.prompt_text,
        };
        self.personas.insert(id, record.clone());
        self.total_prompt_bytes = total_prompt_bytes;
        self.generation = generation;
        Ok(record)
    }

    pub fn update(
        &mut self,
        id: &PersonaId,
        expected_revision: u64,
        draft: PersonaDraft,
    ) -> Result<PersonaRecord> {
        let current = self
            .personas
            .get(id)
            .ok_or_else(|| PersonaError::NotFound {
                kind: "persona",
                id: id.to_string(),
            })?;
        if current.revision != expected_revision {
            return Err(PersonaError::RevisionConflict {
                persona_id: id.to_string(),
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or(PersonaError::RevisionOverflow)?;
        let total_prompt_bytes = checked_total_after_replace(
            self.total_prompt_bytes,
            current.prompt_text.len(),
            draft.prompt_text.len(),
        )?;
        let generation = next_generation(self.generation)?;
        let updated = PersonaRecord {
            id: id.clone(),
            revision,
            label: draft.label,
            prompt_text: draft.prompt_text,
        };
        self.personas.insert(id.clone(), updated.clone());
        self.total_prompt_bytes = total_prompt_bytes;
        self.generation = generation;
        Ok(updated)
    }

    pub fn remove(&mut self, id: &PersonaId, expected_revision: u64) -> Result<PersonaRecord> {
        let current = self
            .personas
            .get(id)
            .ok_or_else(|| PersonaError::NotFound {
                kind: "persona",
                id: id.to_string(),
            })?;
        if current.revision != expected_revision {
            return Err(PersonaError::RevisionConflict {
                persona_id: id.to_string(),
                expected: expected_revision,
                actual: current.revision,
            });
        }
        if self.global_default.as_ref() == Some(id) {
            return Err(PersonaError::PersonaInUse {
                persona_id: id.to_string(),
                scope: "the global default".to_owned(),
            });
        }
        if let Some((character_id, _)) = self.character_policies.iter().find(|(_, policy)| {
            matches!(policy, StoredCharacterPolicy::Specific(persona_id) if persona_id == id)
        }) {
            return Err(PersonaError::PersonaInUse {
                persona_id: id.to_string(),
                scope: format!("character policy {character_id}"),
            });
        }
        let generation = next_generation(self.generation)?;
        let removed = self
            .personas
            .remove(id)
            .expect("persona existence was checked before removal");
        self.total_prompt_bytes -= removed.prompt_text.len();
        self.generation = generation;
        Ok(removed)
    }

    pub fn set_global_default(&mut self, persona_id: Option<PersonaId>) -> Result<()> {
        if let Some(id) = &persona_id {
            self.require_persona(id)?;
        }
        if self.global_default == persona_id {
            return Ok(());
        }
        let generation = next_generation(self.generation)?;
        self.global_default = persona_id;
        self.generation = generation;
        Ok(())
    }

    pub fn set_character_policy(
        &mut self,
        character_id: CharacterCardId,
        policy: CharacterPersonaPolicy,
    ) -> Result<()> {
        let stored = match policy {
            CharacterPersonaPolicy::InheritGlobal => None,
            CharacterPersonaPolicy::Specific(id) => {
                self.require_persona(&id)?;
                Some(StoredCharacterPolicy::Specific(id))
            }
            CharacterPersonaPolicy::NoPersona => Some(StoredCharacterPolicy::NoPersona),
        };
        let unchanged = match (&stored, self.character_policies.get(&character_id)) {
            (None, None) => true,
            (Some(next), Some(current)) => next == current,
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        if stored.is_some()
            && !self.character_policies.contains_key(&character_id)
            && self.character_policies.len() >= MAX_CHARACTER_POLICIES
        {
            return Err(PersonaError::too_many(
                "character policies",
                MAX_CHARACTER_POLICIES,
            ));
        }
        let generation = next_generation(self.generation)?;
        if let Some(stored) = stored {
            self.character_policies.insert(character_id, stored);
        } else {
            self.character_policies.remove(&character_id);
        }
        self.generation = generation;
        Ok(())
    }

    pub fn resolve_for_play(
        &self,
        chat_id: ChatId,
        character_card_id: CharacterCardId,
        choice: PersonaChoice,
    ) -> Result<PersonaPlayBinding> {
        let resolved = match choice {
            PersonaChoice::Specific(id) => {
                Some(self.resolve(&id, PersonaSelectionSource::Explicit)?)
            }
            PersonaChoice::NoPersona => None,
            PersonaChoice::UseDefault => match self.character_policies.get(&character_card_id) {
                Some(StoredCharacterPolicy::Specific(id)) => {
                    Some(self.resolve(id, PersonaSelectionSource::CharacterDefault)?)
                }
                Some(StoredCharacterPolicy::NoPersona) => None,
                None => self
                    .global_default
                    .as_ref()
                    .map(|id| self.resolve(id, PersonaSelectionSource::GlobalDefault))
                    .transpose()?,
            },
        };
        Ok(PersonaPlayBinding {
            chat_id,
            character_card_id,
            persona: resolved,
        })
    }

    fn require_persona(&self, id: &PersonaId) -> Result<&PersonaRecord> {
        self.personas.get(id).ok_or_else(|| PersonaError::NotFound {
            kind: "persona",
            id: id.to_string(),
        })
    }

    fn resolve(&self, id: &PersonaId, source: PersonaSelectionSource) -> Result<ResolvedPersona> {
        Ok(ResolvedPersona {
            source,
            snapshot: self.require_persona(id)?.snapshot(),
        })
    }
}

pub(crate) fn validate_label(label: &str) -> Result<()> {
    if label.is_empty() || label.len() > MAX_PERSONA_LABEL_BYTES {
        return Err(PersonaError::invalid(
            "label",
            format!("must be 1-{MAX_PERSONA_LABEL_BYTES} bytes"),
        ));
    }
    if label.trim() != label {
        return Err(PersonaError::invalid(
            "label",
            "must not start or end with whitespace",
        ));
    }
    if label.chars().any(char::is_control) {
        return Err(PersonaError::invalid(
            "label",
            "must contain no control characters",
        ));
    }
    Ok(())
}

pub(crate) fn validate_prompt_text(prompt_text: &str) -> Result<()> {
    if prompt_text.trim().is_empty() {
        return Err(PersonaError::invalid(
            "promptText",
            "must not be empty or whitespace-only",
        ));
    }
    if prompt_text.len() > MAX_PERSONA_PROMPT_BYTES {
        return Err(PersonaError::too_large(
            "promptText",
            MAX_PERSONA_PROMPT_BYTES,
        ));
    }
    if prompt_text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(PersonaError::invalid(
            "promptText",
            "must contain no control characters other than newline or tab",
        ));
    }
    Ok(())
}

fn checked_total_after_replace(current: usize, old: usize, new: usize) -> Result<usize> {
    let total = current
        .checked_sub(old)
        .and_then(|remaining| remaining.checked_add(new))
        .ok_or_else(|| {
            PersonaError::too_large("total persona prompt text", MAX_TOTAL_PERSONA_PROMPT_BYTES)
        })?;
    if total > MAX_TOTAL_PERSONA_PROMPT_BYTES {
        return Err(PersonaError::too_large(
            "total persona prompt text",
            MAX_TOTAL_PERSONA_PROMPT_BYTES,
        ));
    }
    Ok(total)
}

fn next_generation(current: u64) -> Result<u64> {
    current.checked_add(1).ok_or(PersonaError::RevisionOverflow)
}

#[cfg(test)]
mod tests {
    use lorepia_prompt::{ContentFormat, PromptBlock, PromptPreset, PromptRole, compile_prompt};

    use super::*;

    fn persona_id(value: &str) -> PersonaId {
        PersonaId::parse(value).unwrap()
    }

    fn character_id(value: &str) -> CharacterCardId {
        CharacterCardId::parse(value).unwrap()
    }

    fn chat_id(value: &str) -> ChatId {
        ChatId::parse(value).unwrap()
    }

    fn draft(label: &str, prompt: &str) -> PersonaDraft {
        PersonaDraft::new(label, prompt).unwrap()
    }

    #[test]
    fn selection_priority_and_explicit_no_persona_are_distinct() {
        let global = persona_id("global");
        let character = persona_id("character");
        let explicit = persona_id("explicit");
        let card = character_id("card");
        let mut catalog = PersonaCatalog::new();
        catalog
            .add(global.clone(), draft("Global", "global self"))
            .unwrap();
        catalog
            .add(character.clone(), draft("Character", "character self"))
            .unwrap();
        catalog
            .add(explicit.clone(), draft("Explicit", "explicit self"))
            .unwrap();
        catalog.set_global_default(Some(global)).unwrap();
        catalog
            .set_character_policy(card.clone(), CharacterPersonaPolicy::Specific(character))
            .unwrap();

        let automatic = catalog
            .resolve_for_play(
                chat_id("chat-auto"),
                card.clone(),
                PersonaChoice::UseDefault,
            )
            .unwrap();
        assert_eq!(automatic.prompt_text(), Some("character self"));
        assert_eq!(
            automatic.source(),
            Some(PersonaSelectionSource::CharacterDefault)
        );

        let selected = catalog
            .resolve_for_play(
                chat_id("chat-selected"),
                card.clone(),
                PersonaChoice::Specific(explicit),
            )
            .unwrap();
        assert_eq!(selected.prompt_text(), Some("explicit self"));
        assert_eq!(selected.source(), Some(PersonaSelectionSource::Explicit));

        let none = catalog
            .resolve_for_play(chat_id("chat-none"), card, PersonaChoice::NoPersona)
            .unwrap();
        assert_eq!(none.prompt_text(), None);
    }

    #[test]
    fn character_no_persona_overrides_global_default() {
        let global = persona_id("global");
        let card = character_id("card");
        let mut catalog = PersonaCatalog::new();
        catalog
            .add(global.clone(), draft("Global", "global self"))
            .unwrap();
        catalog.set_global_default(Some(global)).unwrap();
        catalog
            .set_character_policy(card.clone(), CharacterPersonaPolicy::NoPersona)
            .unwrap();

        let binding = catalog
            .resolve_for_play(chat_id("chat"), card, PersonaChoice::UseDefault)
            .unwrap();
        assert!(binding.persona().is_none());
    }

    #[test]
    fn play_binding_is_an_immutable_snapshot_across_edits_and_deletion() {
        let id = persona_id("player");
        let mut catalog = PersonaCatalog::new();
        catalog
            .add(id.clone(), draft("Player", "first self"))
            .unwrap();
        let binding = catalog
            .resolve_for_play(
                chat_id("chat"),
                character_id("card"),
                PersonaChoice::Specific(id.clone()),
            )
            .unwrap();

        let updated = catalog
            .update(&id, 1, draft("Player", "second self"))
            .unwrap();
        catalog.remove(&id, updated.revision()).unwrap();

        assert_eq!(binding.prompt_text(), Some("first self"));
        assert_eq!(binding.persona().unwrap().revision(), 1);
    }

    #[test]
    fn referenced_personas_must_be_unbound_before_deletion() {
        let id = persona_id("player");
        let mut catalog = PersonaCatalog::new();
        catalog.add(id.clone(), draft("Player", "self")).unwrap();
        catalog.set_global_default(Some(id.clone())).unwrap();
        assert!(matches!(
            catalog.remove(&id, 1),
            Err(PersonaError::PersonaInUse { .. })
        ));

        catalog.set_global_default(None).unwrap();
        catalog
            .set_character_policy(
                character_id("card"),
                CharacterPersonaPolicy::Specific(id.clone()),
            )
            .unwrap();
        assert!(matches!(
            catalog.remove(&id, 1),
            Err(PersonaError::PersonaInUse { .. })
        ));
    }

    #[test]
    fn stale_edits_are_rejected_without_mutation() {
        let id = persona_id("player");
        let mut catalog = PersonaCatalog::new();
        catalog.add(id.clone(), draft("Player", "first")).unwrap();
        let generation = catalog.generation();

        assert!(matches!(
            catalog.update(&id, 9, draft("Player", "stale")),
            Err(PersonaError::RevisionConflict { .. })
        ));
        assert_eq!(catalog.generation(), generation);
        assert_eq!(catalog.get(&id).unwrap().prompt_text(), "first");
    }

    #[test]
    fn applying_no_persona_clears_stale_prompt_input() {
        let catalog = PersonaCatalog::new();
        let binding = catalog
            .resolve_for_play(
                chat_id("chat"),
                character_id("card"),
                PersonaChoice::NoPersona,
            )
            .unwrap();
        let mut input = PromptCompileInput {
            persona: "stale persona".to_owned(),
            ..PromptCompileInput::default()
        };

        binding
            .apply_to_prompt_input(&chat_id("chat"), &character_id("card"), &mut input)
            .unwrap();
        assert!(input.persona.is_empty());
    }

    #[test]
    fn binding_cannot_be_applied_to_another_chat_or_character() {
        let id = persona_id("player");
        let mut catalog = PersonaCatalog::new();
        catalog
            .add(id.clone(), draft("Player", "private self"))
            .unwrap();
        let binding = catalog
            .resolve_for_play(
                chat_id("chat-a"),
                character_id("card-a"),
                PersonaChoice::Specific(id),
            )
            .unwrap();
        let mut input = PromptCompileInput {
            persona: "unchanged".to_owned(),
            ..PromptCompileInput::default()
        };

        assert!(matches!(
            binding.apply_to_prompt_input(&chat_id("chat-b"), &character_id("card-a"), &mut input),
            Err(PersonaError::BindingMismatch {
                field: "chatId",
                ..
            })
        ));
        assert_eq!(input.persona, "unchanged");
        assert!(matches!(
            binding.apply_to_prompt_input(&chat_id("chat-a"), &character_id("card-b"), &mut input),
            Err(PersonaError::BindingMismatch {
                field: "characterCardId",
                ..
            })
        ));
        assert_eq!(input.persona, "unchanged");
    }

    #[test]
    fn persona_runtime_text_is_not_reexpanded_as_a_template() {
        let id = persona_id("player");
        let mut catalog = PersonaCatalog::new();
        catalog
            .add(id.clone(), draft("Player", "literal ${secret}"))
            .unwrap();
        let binding = catalog
            .resolve_for_play(
                chat_id("chat"),
                character_id("card"),
                PersonaChoice::Specific(id),
            )
            .unwrap();
        let mut input = PromptCompileInput::default();
        input
            .variables
            .insert("secret".to_owned(), "expanded".to_owned());
        binding
            .apply_to_prompt_input(&chat_id("chat"), &character_id("card"), &mut input)
            .unwrap();
        let preset = PromptPreset {
            name: "persona".to_owned(),
            blocks: vec![PromptBlock::Persona {
                name: "persona".to_owned(),
                enabled: true,
                role: PromptRole::System,
                format: ContentFormat::Custom {
                    template: "Self: ${value}".to_owned(),
                },
            }],
            sampling: Default::default(),
            advanced: Default::default(),
        };

        let compiled = compile_prompt(&preset, &input).unwrap();
        assert_eq!(compiled.messages[0].content, "Self: literal ${secret}");
    }

    #[test]
    fn labels_and_prompt_text_have_separate_validation() {
        assert!(PersonaDraft::new("\n", "valid").is_err());
        assert!(PersonaDraft::new("valid", "\0").is_err());
        assert!(PersonaDraft::new("same", "first").is_ok());
        assert!(PersonaDraft::new("same", "second").is_ok());
    }
}
