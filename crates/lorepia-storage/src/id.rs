use std::{fmt, str::FromStr};

use serde::Serialize;
use uuid::Uuid;

use crate::{Result, StorageError};

pub const MAX_ID_BYTES: usize = 128;
pub const MAX_MODEL_ID_BYTES: usize = 256;

fn validate_id(value: &str, field: &'static str) -> Result<()> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(StorageError::InvalidInput {
            field,
            reason: "must be exactly 32 lowercase hexadecimal characters",
        });
    }
    Ok(())
}

macro_rules! define_id {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4().simple().to_string())
            }

            pub fn parse(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                validate_id(&value, $field)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = StorageError;

            fn from_str(value: &str) -> Result<Self> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = StorageError;

            fn try_from(value: String) -> Result<Self> {
                Self::parse(value)
            }
        }
    };
}

define_id!(ChatId, "chat ID");
define_id!(MessageId, "message ID");
define_id!(RequestStateId, "request state ID");
define_id!(StreamGeneration, "stream generation");

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CharacterId(String);

impl CharacterId {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_ID_BYTES {
            return Err(StorageError::InvalidInput {
                field: "character ID",
                reason: "must contain between 1 and 128 bytes",
            });
        }
        if value.contains('\0') {
            return Err(StorageError::InvalidInput {
                field: "character ID",
                reason: "contains a null character",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CharacterId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for CharacterId {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl TryFrom<String> for CharacterId {
    type Error = StorageError;

    fn try_from(value: String) -> Result<Self> {
        Self::parse(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(StorageError::InvalidInput {
                field: "model ID",
                reason: "must not be empty",
            });
        }
        if value.len() > MAX_MODEL_ID_BYTES {
            return Err(StorageError::InvalidInput {
                field: "model ID",
                reason: "exceeds the byte limit",
            });
        }
        if value.chars().any(char::is_control) {
            return Err(StorageError::InvalidInput {
                field: "model ID",
                reason: "contains a control character",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ModelId {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl TryFrom<String> for ModelId {
    type Error = StorageError;

    fn try_from(value: String) -> Result<Self> {
        Self::parse(value)
    }
}
