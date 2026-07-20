use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{PersonaError, Result};

const MAX_PERSONA_ID_BYTES: usize = 64;
const MAX_CHARACTER_CARD_ID_BYTES: usize = 128;
const MAX_CHAT_ID_BYTES: usize = 64;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PersonaId(String);

impl PersonaId {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_id("personaId", &value, MAX_PERSONA_ID_BYTES)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PersonaId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for PersonaId {
    type Err = PersonaError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for PersonaId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CharacterCardId(String);

impl CharacterCardId {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_id("characterCardId", &value, MAX_CHARACTER_CARD_ID_BYTES)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CharacterCardId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for CharacterCardId {
    type Err = PersonaError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for CharacterCardId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ChatId(String);

impl ChatId {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_id("chatId", &value, MAX_CHAT_ID_BYTES)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ChatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ChatId {
    type Err = PersonaError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ChatId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

fn validate_id(field: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.is_empty() || value.len() > max_bytes {
        return Err(PersonaError::invalid(
            field,
            format!("must be 1-{max_bytes} bytes"),
        ));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(PersonaError::invalid(
            field,
            "must contain only ASCII letters, digits, hyphen, underscore, or period",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_closed_opaque_identifiers() {
        assert!(PersonaId::parse("persona_01.a-b").is_ok());
        assert!(CharacterCardId::parse("card_01").is_ok());
        assert!(ChatId::parse("chat_01").is_ok());
        for invalid in ["", "has space", "../card", "카드", "a/b"] {
            assert!(PersonaId::parse(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn deserialization_cannot_bypass_id_validation() {
        let error = serde_json::from_str::<PersonaId>(r#""../persona""#).unwrap_err();
        assert!(error.to_string().contains("personaId"));
    }
}
