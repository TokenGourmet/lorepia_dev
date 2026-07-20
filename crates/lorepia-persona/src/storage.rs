use std::collections::BTreeMap;

use std::{fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, IgnoredAny, SeqAccess, Visitor},
};
use serde_json::value::RawValue;

use crate::{
    CharacterCardId, MAX_CHARACTER_POLICIES, MAX_PERSONA_STATE_BYTES, MAX_PERSONAS,
    MAX_TOTAL_PERSONA_PROMPT_BYTES, PersonaCatalog, PersonaDraft, PersonaError, PersonaId,
    PersonaRecord, Result,
    catalog::{StoredCharacterPolicy, validate_label, validate_prompt_text},
};

pub const PERSONA_STATE_FORMAT: &str = "lorepia.persona-state";
pub const PERSONA_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateEnvelopeOut {
    format: &'static str,
    schema_version: u32,
    state: CatalogWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateEnvelopeIn<'a> {
    format: String,
    schema_version: u32,
    #[serde(borrow)]
    state: &'a RawValue,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogWire {
    generation: u64,
    personas: BoundedVec<PersonaWire, MAX_PERSONAS, MAX_TOTAL_PERSONA_PROMPT_BYTES>,
    global_default_persona_id: Option<PersonaId>,
    character_policies: BoundedVec<CharacterPolicyWire, MAX_CHARACTER_POLICIES, { usize::MAX }>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersonaWire {
    id: PersonaId,
    revision: u64,
    label: String,
    prompt_text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CharacterPolicyWire {
    character_card_id: CharacterCardId,
    policy: StoredPolicyWire,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum StoredPolicyWire {
    Specific { persona_id: PersonaId },
    NoPersona,
}

trait WireItem {
    fn validate(&self) -> Result<()>;
    fn budget_bytes(&self) -> usize;
}

impl WireItem for PersonaWire {
    fn validate(&self) -> Result<()> {
        if self.revision == 0 {
            return Err(PersonaError::state("persona revision must be at least 1"));
        }
        validate_label(&self.label)?;
        validate_prompt_text(&self.prompt_text)
    }

    fn budget_bytes(&self) -> usize {
        self.prompt_text.len()
    }
}

impl WireItem for CharacterPolicyWire {
    fn validate(&self) -> Result<()> {
        Ok(())
    }

    fn budget_bytes(&self) -> usize {
        0
    }
}

struct BoundedVec<T, const MAX: usize, const MAX_BUDGET_BYTES: usize>(Vec<T>);

impl<T: Serialize, const MAX: usize, const MAX_BUDGET_BYTES: usize> Serialize
    for BoundedVec<T, MAX, MAX_BUDGET_BYTES>
{
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de> + WireItem, const MAX: usize, const MAX_BUDGET_BYTES: usize>
    Deserialize<'de> for BoundedVec<T, MAX, MAX_BUDGET_BYTES>
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(BoundedVecVisitor::<T, MAX, MAX_BUDGET_BYTES>(PhantomData))
    }
}

struct BoundedVecVisitor<T, const MAX: usize, const MAX_BUDGET_BYTES: usize>(PhantomData<T>);

impl<'de, T: Deserialize<'de> + WireItem, const MAX: usize, const MAX_BUDGET_BYTES: usize>
    Visitor<'de> for BoundedVecVisitor<T, MAX, MAX_BUDGET_BYTES>
{
    type Value = BoundedVec<T, MAX, MAX_BUDGET_BYTES>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "an array containing at most {MAX} entries")
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if sequence.size_hint().is_some_and(|size| size > MAX) {
            return Err(de::Error::custom(format!("array exceeds {MAX} entries")));
        }
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(MAX));
        let mut budget_bytes = 0usize;
        while values.len() < MAX {
            let Some(value) = sequence.next_element::<T>()? else {
                return Ok(BoundedVec(values));
            };
            value.validate().map_err(de::Error::custom)?;
            budget_bytes = budget_bytes
                .checked_add(value.budget_bytes())
                .ok_or_else(|| de::Error::custom("array byte budget overflowed"))?;
            if budget_bytes > MAX_BUDGET_BYTES {
                return Err(de::Error::custom(format!(
                    "array exceeds {MAX_BUDGET_BYTES} budget bytes"
                )));
            }
            values.push(value);
        }
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(format!("array exceeds {MAX} entries")));
        }
        Ok(BoundedVec(values))
    }
}

impl<T, const MAX: usize, const MAX_BUDGET_BYTES: usize> BoundedVec<T, MAX, MAX_BUDGET_BYTES> {
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<T, const MAX: usize, const MAX_BUDGET_BYTES: usize> IntoIterator
    for BoundedVec<T, MAX, MAX_BUDGET_BYTES>
{
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

pub fn serialize_state(catalog: &PersonaCatalog) -> Result<Vec<u8>> {
    let personas = catalog
        .personas
        .values()
        .map(|record| PersonaWire {
            id: record.id.clone(),
            revision: record.revision,
            label: record.label.clone(),
            prompt_text: record.prompt_text.clone(),
        })
        .collect();
    let character_policies = catalog
        .character_policies
        .iter()
        .map(|(character_card_id, policy)| CharacterPolicyWire {
            character_card_id: character_card_id.clone(),
            policy: match policy {
                StoredCharacterPolicy::Specific(persona_id) => StoredPolicyWire::Specific {
                    persona_id: persona_id.clone(),
                },
                StoredCharacterPolicy::NoPersona => StoredPolicyWire::NoPersona,
            },
        })
        .collect();
    let bytes = serde_json::to_vec_pretty(&StateEnvelopeOut {
        format: PERSONA_STATE_FORMAT,
        schema_version: PERSONA_STATE_SCHEMA_VERSION,
        state: CatalogWire {
            generation: catalog.generation,
            personas: BoundedVec(personas),
            global_default_persona_id: catalog.global_default.clone(),
            character_policies: BoundedVec(character_policies),
        },
    })?;
    if bytes.len() > MAX_PERSONA_STATE_BYTES {
        return Err(PersonaError::too_large(
            "persona state",
            MAX_PERSONA_STATE_BYTES,
        ));
    }
    Ok(bytes)
}

pub fn deserialize_state(bytes: &[u8]) -> Result<PersonaCatalog> {
    if bytes.len() > MAX_PERSONA_STATE_BYTES {
        return Err(PersonaError::too_large(
            "persona state",
            MAX_PERSONA_STATE_BYTES,
        ));
    }
    let envelope: StateEnvelopeIn<'_> = serde_json::from_slice(bytes)?;
    if envelope.format != PERSONA_STATE_FORMAT {
        return Err(PersonaError::state("unsupported persona state format"));
    }
    match envelope.schema_version {
        1 => restore_v1(serde_json::from_str(envelope.state.get())?),
        version => Err(PersonaError::state(format!(
            "unsupported schema version {version}; expected {PERSONA_STATE_SCHEMA_VERSION}"
        ))),
    }
}

fn restore_v1(wire: CatalogWire) -> Result<PersonaCatalog> {
    if wire.personas.len() > MAX_PERSONAS {
        return Err(PersonaError::too_many("personas", MAX_PERSONAS));
    }
    if wire.character_policies.len() > MAX_CHARACTER_POLICIES {
        return Err(PersonaError::too_many(
            "character policies",
            MAX_CHARACTER_POLICIES,
        ));
    }

    let mut personas = BTreeMap::new();
    let mut total_prompt_bytes = 0usize;
    for persona in wire.personas {
        if persona.revision == 0 {
            return Err(PersonaError::state("persona revision must be at least 1"));
        }
        let draft = PersonaDraft::new(persona.label, persona.prompt_text)?;
        total_prompt_bytes = total_prompt_bytes
            .checked_add(draft.prompt_text().len())
            .ok_or_else(|| {
                PersonaError::too_large("total persona prompt text", MAX_TOTAL_PERSONA_PROMPT_BYTES)
            })?;
        if total_prompt_bytes > MAX_TOTAL_PERSONA_PROMPT_BYTES {
            return Err(PersonaError::too_large(
                "total persona prompt text",
                MAX_TOTAL_PERSONA_PROMPT_BYTES,
            ));
        }
        let id = persona.id;
        let record = PersonaRecord {
            id: id.clone(),
            revision: persona.revision,
            label: draft.label().to_owned(),
            prompt_text: draft.prompt_text().to_owned(),
        };
        if personas.insert(id.clone(), record).is_some() {
            return Err(PersonaError::AlreadyExists {
                kind: "persona",
                id: id.to_string(),
            });
        }
    }

    if let Some(id) = &wire.global_default_persona_id
        && !personas.contains_key(id)
    {
        return Err(PersonaError::NotFound {
            kind: "global default persona",
            id: id.to_string(),
        });
    }

    let mut character_policies = BTreeMap::new();
    for entry in wire.character_policies {
        let stored = match entry.policy {
            StoredPolicyWire::Specific { persona_id } => {
                if !personas.contains_key(&persona_id) {
                    return Err(PersonaError::NotFound {
                        kind: "character policy persona",
                        id: persona_id.to_string(),
                    });
                }
                StoredCharacterPolicy::Specific(persona_id)
            }
            StoredPolicyWire::NoPersona => StoredCharacterPolicy::NoPersona,
        };
        let character_id = entry.character_card_id;
        if character_policies
            .insert(character_id.clone(), stored)
            .is_some()
        {
            return Err(PersonaError::AlreadyExists {
                kind: "character policy",
                id: character_id.to_string(),
            });
        }
    }

    Ok(PersonaCatalog {
        generation: wire.generation,
        personas,
        global_default: wire.global_default_persona_id,
        character_policies,
        total_prompt_bytes,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::{CharacterPersonaPolicy, ChatId, PersonaChoice};

    use super::*;

    fn catalog() -> PersonaCatalog {
        let id = PersonaId::parse("player").unwrap();
        let card = CharacterCardId::parse("card").unwrap();
        let mut catalog = PersonaCatalog::new();
        catalog
            .add(
                id.clone(),
                PersonaDraft::new("Player", "A curious explorer.").unwrap(),
            )
            .unwrap();
        catalog.set_global_default(Some(id.clone())).unwrap();
        catalog
            .set_character_policy(card, CharacterPersonaPolicy::NoPersona)
            .unwrap();
        catalog
    }

    #[test]
    fn local_state_round_trips_with_defaults_and_revisions() {
        let expected = catalog();
        let bytes = serialize_state(&expected).unwrap();
        let actual = deserialize_state(&bytes).unwrap();
        assert_eq!(actual, expected);
        let binding = actual
            .resolve_for_play(
                ChatId::parse("chat").unwrap(),
                CharacterCardId::parse("other-card").unwrap(),
                PersonaChoice::UseDefault,
            )
            .unwrap();
        assert_eq!(binding.prompt_text(), Some("A curious explorer."));
    }

    #[test]
    fn version_is_dispatched_before_current_state_shape() {
        let value = json!({
            "format": PERSONA_STATE_FORMAT,
            "schemaVersion": 2,
            "state": {"future": true}
        });
        let error = deserialize_state(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.to_string().contains("unsupported schema version 2"));
    }

    #[test]
    fn unknown_and_duplicate_fields_are_rejected() {
        let mut value: Value =
            serde_json::from_slice(&serialize_state(&catalog()).unwrap()).unwrap();
        value["state"]["personas"][0]["foreign"] = Value::Bool(true);
        let error = deserialize_state(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let duplicate = br#"{
          "format":"lorepia.persona-state",
          "schemaVersion":1,
          "schemaVersion":1,
          "state":{"generation":0,"personas":[],"globalDefaultPersonaId":null,"characterPolicies":[]}
        }"#;
        let error = deserialize_state(duplicate).unwrap_err();
        assert!(error.to_string().contains("duplicate field"));
    }

    #[test]
    fn duplicate_ids_and_dangling_references_are_rejected() {
        let mut value: Value =
            serde_json::from_slice(&serialize_state(&catalog()).unwrap()).unwrap();
        let persona = value["state"]["personas"][0].clone();
        value["state"]["personas"]
            .as_array_mut()
            .unwrap()
            .push(persona);
        assert!(matches!(
            deserialize_state(&serde_json::to_vec(&value).unwrap()),
            Err(PersonaError::AlreadyExists { .. })
        ));

        let mut value: Value =
            serde_json::from_slice(&serialize_state(&catalog()).unwrap()).unwrap();
        value["state"]["globalDefaultPersonaId"] = Value::String("missing".to_owned());
        assert!(matches!(
            deserialize_state(&serde_json::to_vec(&value).unwrap()),
            Err(PersonaError::NotFound { .. })
        ));
    }

    #[test]
    fn sequence_limits_are_enforced_during_deserialization() {
        let mut value: Value =
            serde_json::from_slice(&serialize_state(&PersonaCatalog::new()).unwrap()).unwrap();
        value["state"]["personas"] = Value::Array(
            (0..=MAX_PERSONAS)
                .map(|index| {
                    json!({
                        "id": format!("p{index}"),
                        "revision": 1,
                        "label": "Player",
                        "promptText": "self"
                    })
                })
                .collect(),
        );
        let error = deserialize_state(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.to_string().contains("array exceeds 256 entries"));
    }

    #[test]
    fn cumulative_prompt_budget_is_enforced_while_reading_personas() {
        let mut value: Value =
            serde_json::from_slice(&serialize_state(&PersonaCatalog::new()).unwrap()).unwrap();
        value["state"]["personas"] = Value::Array(
            (0..=(MAX_TOTAL_PERSONA_PROMPT_BYTES / crate::MAX_PERSONA_PROMPT_BYTES))
                .map(|index| {
                    json!({
                        "id": format!("p{index}"),
                        "revision": 1,
                        "label": "Player",
                        "promptText": "x".repeat(crate::MAX_PERSONA_PROMPT_BYTES)
                    })
                })
                .collect(),
        );

        let error = deserialize_state(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.to_string().contains("budget bytes"));
    }
}
