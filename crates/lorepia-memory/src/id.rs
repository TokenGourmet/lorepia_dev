use std::{fmt, marker::PhantomData, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{MemoryError, Result};

const MAX_OPAQUE_ID_BYTES: usize = 128;

macro_rules! opaque_id {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                validate_id($field, &value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = MemoryError;

            fn from_str(value: &str) -> Result<Self> {
                Self::parse(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(de::Error::custom)
            }
        }
    };
}

opaque_id!(MemoryPresetId, "memoryPresetId");
opaque_id!(PromptPresetId, "promptPresetId");
opaque_id!(ModelPresetId, "modelPresetId");
opaque_id!(EmbeddingProfileId, "embeddingProfileId");
opaque_id!(MemoryArtifactId, "memoryArtifactId");
opaque_id!(MessageId, "messageId");

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionedRef<I> {
    id: I,
    revision: u64,
    #[serde(skip)]
    marker: PhantomData<fn() -> I>,
}

impl<I> VersionedRef<I> {
    pub fn new(id: I, revision: u64) -> Result<Self> {
        if revision == 0 {
            return Err(MemoryError::invalid("revision", "must be at least 1"));
        }
        Ok(Self {
            id,
            revision,
            marker: PhantomData,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &I {
        &self.id
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

pub type PromptPresetRef = VersionedRef<PromptPresetId>;
pub type ModelPresetRef = VersionedRef<ModelPresetId>;
pub type EmbeddingProfileRef = VersionedRef<EmbeddingProfileId>;

fn validate_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_OPAQUE_ID_BYTES {
        return Err(MemoryError::invalid(
            field,
            format!("must be 1-{MAX_OPAQUE_ID_BYTES} bytes"),
        ));
    }
    if value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(MemoryError::invalid(
            field,
            "must be a safe opaque ASCII identifier",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_revisions_are_closed() {
        assert!(MemoryPresetId::parse("memory.main-v1").is_ok());
        assert!(MemoryPresetId::parse("../memory").is_err());
        assert!(ModelPresetRef::new(ModelPresetId::parse("helper").unwrap(), 0).is_err());
    }
}
