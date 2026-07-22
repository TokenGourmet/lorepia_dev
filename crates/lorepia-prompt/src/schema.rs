use std::collections::BTreeMap;

use lorepia_providers::MessageRole;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptPreset {
    pub name: String,
    pub blocks: Vec<PromptBlock>,
    #[serde(default)]
    pub sampling: PromptSampling,
    #[serde(default)]
    pub advanced: AdvancedSettings,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PromptPresetWire {
    name: String,
    blocks: Vec<PromptBlock>,
    #[serde(default)]
    sampling: PromptSampling,
    #[serde(default)]
    advanced: AdvancedSettings,
}

impl<'de> Deserialize<'de> for PromptPreset {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PromptPresetWire::deserialize(deserializer)?;
        let mut preset = Self {
            name: wire.name,
            blocks: wire.blocks,
            sampling: wire.sampling,
            advanced: wire.advanced,
        };

        // Portable data cannot deserialize its own local execution authority.
        // Local approval is a separate product-policy action after validation.
        for reference in &mut preset.advanced.tools {
            reference.enabled = false;
        }
        for reference in &mut preset.advanced.modules {
            reference.enabled = false;
        }
        for rule in &mut preset.advanced.regex {
            rule.enabled = false;
        }
        Ok(preset)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptRole {
    System,
    User,
    Assistant,
}

impl From<PromptRole> for MessageRole {
    fn from(value: PromptRole) -> Self {
        match value {
            PromptRole::System => Self::System,
            PromptRole::User => Self::User,
            PromptRole::Assistant => Self::Assistant,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptBlockKind {
    Raw,
    Chat,
    Persona,
    CharacterDescription,
    AuthorNote,
    Lorebook,
    LongTermMemory,
    FinalInsertion,
    ChatMl,
    CachePoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawPromptSpecial {
    Main,
    GlobalNote,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentFormat {
    Plain,
    Custom { template: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSelection {
    pub start: usize,
    pub end: ChatEnd,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "index",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ChatEnd {
    Index(usize),
    EndOfChat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PromptBlock {
    Raw {
        name: String,
        enabled: bool,
        role: PromptRole,
        special: Option<RawPromptSpecial>,
        prompt: String,
    },
    Chat {
        name: String,
        enabled: bool,
        selection: ChatSelection,
    },
    Persona {
        name: String,
        enabled: bool,
        role: PromptRole,
        format: ContentFormat,
    },
    CharacterDescription {
        name: String,
        enabled: bool,
        role: PromptRole,
        format: ContentFormat,
    },
    AuthorNote {
        name: String,
        enabled: bool,
        role: PromptRole,
        default_prompt: Option<String>,
        format: ContentFormat,
    },
    Lorebook {
        name: String,
        enabled: bool,
        role: PromptRole,
        format: ContentFormat,
    },
    LongTermMemory {
        name: String,
        enabled: bool,
        role: PromptRole,
        format: ContentFormat,
    },
    FinalInsertion {
        name: String,
        enabled: bool,
        role: PromptRole,
        prompt: String,
    },
    ChatMl {
        name: String,
        enabled: bool,
        role: PromptRole,
        prompt: String,
    },
    CachePoint {
        name: String,
        enabled: bool,
        depth: usize,
        role: PromptRole,
    },
}

impl PromptBlock {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Raw { name, .. }
            | Self::Chat { name, .. }
            | Self::Persona { name, .. }
            | Self::CharacterDescription { name, .. }
            | Self::AuthorNote { name, .. }
            | Self::Lorebook { name, .. }
            | Self::LongTermMemory { name, .. }
            | Self::FinalInsertion { name, .. }
            | Self::ChatMl { name, .. }
            | Self::CachePoint { name, .. } => name,
        }
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        match self {
            Self::Raw { enabled, .. }
            | Self::Chat { enabled, .. }
            | Self::Persona { enabled, .. }
            | Self::CharacterDescription { enabled, .. }
            | Self::AuthorNote { enabled, .. }
            | Self::Lorebook { enabled, .. }
            | Self::LongTermMemory { enabled, .. }
            | Self::FinalInsertion { enabled, .. }
            | Self::ChatMl { enabled, .. }
            | Self::CachePoint { enabled, .. } => *enabled,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> PromptBlockKind {
        match self {
            Self::Raw { .. } => PromptBlockKind::Raw,
            Self::Chat { .. } => PromptBlockKind::Chat,
            Self::Persona { .. } => PromptBlockKind::Persona,
            Self::CharacterDescription { .. } => PromptBlockKind::CharacterDescription,
            Self::AuthorNote { .. } => PromptBlockKind::AuthorNote,
            Self::Lorebook { .. } => PromptBlockKind::Lorebook,
            Self::LongTermMemory { .. } => PromptBlockKind::LongTermMemory,
            Self::FinalInsertion { .. } => PromptBlockKind::FinalInsertion,
            Self::ChatMl { .. } => PromptBlockKind::ChatMl,
            Self::CachePoint { .. } => PromptBlockKind::CachePoint,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptSampling {
    pub temperature: Option<f64>,
    pub top_k: Option<u32>,
    pub min_p: Option<f64>,
    pub top_a: Option<f64>,
    pub repetition_penalty: Option<f64>,
    pub top_p: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub frequency_penalty: Option<f64>,
    #[serde(default)]
    pub stop_sequences: Vec<String>,
    pub seed: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdvancedSettings {
    #[serde(default)]
    pub template: PromptTemplateSettings,
    #[serde(default)]
    pub tools: Vec<ToolReference>,
    #[serde(default)]
    pub regex: Vec<RegexRule>,
    #[serde(default)]
    pub modules: Vec<ModuleReference>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptTemplateSettings {
    pub preamble: Option<SyntheticMessage>,
    pub epilogue: Option<SyntheticMessage>,
    #[serde(default)]
    pub default_variables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyntheticMessage {
    pub role: PromptRole,
    pub template: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolReference {
    pub tool_id: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleReference {
    pub module_id: String,
    pub version: String,
    pub digest_sha256: String,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformTarget {
    Input,
    Request,
    Response,
    Display,
    Translation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegexFlags {
    pub global: bool,
    pub case_insensitive: bool,
    pub multi_line: bool,
    pub unicode: bool,
    pub dot_matches_new_line: bool,
}

impl Default for RegexFlags {
    fn default() -> Self {
        Self {
            global: true,
            case_insensitive: false,
            multi_line: false,
            unicode: true,
            dot_matches_new_line: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegexRule {
    pub name: String,
    pub enabled: bool,
    pub target: TransformTarget,
    pub pattern: String,
    pub replacement: String,
    #[serde(default)]
    pub flags: RegexFlags,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachePoint {
    pub block_index: usize,
    pub message_index: usize,
    pub depth: usize,
    pub role: PromptRole,
}
