use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{
    LorebookError, MAX_CATALOG_BYTES, MAX_CATALOG_ENTRIES, MAX_ENTRY_CONDITIONS,
    MAX_ENTRY_CONTENT_BYTES, MAX_KEY_BYTES, MAX_LITERAL_MATCH_EVENTS, MAX_OUTPUT_BYTES,
    MAX_OUTPUT_TOKENS, MAX_RECENT_TURNS, MAX_REGEX_BYTES, MAX_REGEX_EVALUATIONS, MAX_REGEX_MATCHES,
    MAX_REGEX_SCAN_BYTES, MAX_SEARCH_INPUT_BYTES, MAX_TURN_BYTES, Result,
};

const MAX_ENTRY_ID_BYTES: usize = 128;
const MAX_TOKENIZER_ID_BYTES: usize = 256;
const MAX_TOKENIZER_REVISION_BYTES: usize = 256;

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EntryId(String);

impl EntryId {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_ENTRY_ID_BYTES {
            return Err(LorebookError::invalid(
                "entry.id",
                "must be a bounded non-empty identifier",
            ));
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        {
            return Err(LorebookError::invalid(
                "entry.id",
                "contains unsupported characters",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntryId([redacted])")
    }
}

impl<'de> Deserialize<'de> for EntryId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrySource {
    Global,
    CharacterCard,
    Chat,
}

impl EntrySource {
    pub(crate) const fn precedence(self) -> u8 {
        match self {
            Self::Global => 0,
            Self::CharacterCard => 1,
            Self::Chat => 2,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MatchCondition {
    Literal {
        value: String,
        #[serde(default)]
        case_sensitive: bool,
    },
    Regex {
        pattern: String,
        required_literal: String,
        #[serde(default)]
        case_insensitive: bool,
    },
}

impl fmt::Debug for MatchCondition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal { case_sensitive, .. } => formatter
                .debug_struct("Literal")
                .field("case_sensitive", case_sensitive)
                .field("value", &"[redacted]")
                .finish(),
            Self::Regex {
                case_insensitive, ..
            } => formatter
                .debug_struct("Regex")
                .field("case_insensitive", case_insensitive)
                .field("pattern", &"[redacted]")
                .field("required_literal", &"[redacted]")
                .finish(),
        }
    }
}

impl MatchCondition {
    pub fn literal(value: impl Into<String>) -> Self {
        Self::Literal {
            value: value.into(),
            case_sensitive: false,
        }
    }

    pub fn regex(pattern: impl Into<String>, required_literal: impl Into<String>) -> Self {
        Self::Regex {
            pattern: pattern.into(),
            required_literal: required_literal.into(),
            case_insensitive: false,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let (primary, secondary) = match self {
            Self::Literal { value, .. } => (value.as_str(), None),
            Self::Regex {
                pattern,
                required_literal,
                ..
            } => (required_literal.as_str(), Some(pattern.as_str())),
        };
        if primary.is_empty() || primary.len() > MAX_KEY_BYTES {
            return Err(LorebookError::invalid(
                "entry.condition.key",
                "must be a bounded non-empty string",
            ));
        }
        if let Some(pattern) = secondary
            && (pattern.is_empty() || pattern.len() > MAX_REGEX_BYTES)
        {
            return Err(LorebookError::invalid(
                "entry.condition.regex",
                "must be a bounded non-empty pattern",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "conditions",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SecondaryConditions {
    None,
    Any(Vec<MatchCondition>),
    All(Vec<MatchCondition>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatchConditions {
    pub primary: Vec<MatchCondition>,
    #[serde(default = "default_secondary")]
    pub secondary: SecondaryConditions,
}

fn default_secondary() -> SecondaryConditions {
    SecondaryConditions::None
}

impl MatchConditions {
    pub fn any(primary: Vec<MatchCondition>) -> Self {
        Self {
            primary,
            secondary: SecondaryConditions::None,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.primary.is_empty() {
            return Err(LorebookError::invalid(
                "entry.conditions.primary",
                "must contain at least one condition",
            ));
        }
        let secondary = match &self.secondary {
            SecondaryConditions::None => &[][..],
            SecondaryConditions::Any(values) | SecondaryConditions::All(values) => values,
        };
        if matches!(
            self.secondary,
            SecondaryConditions::Any(_) | SecondaryConditions::All(_)
        ) && secondary.is_empty()
        {
            return Err(LorebookError::invalid(
                "entry.conditions.secondary",
                "enabled secondary conditions cannot be empty",
            ));
        }
        let count = self
            .primary
            .len()
            .checked_add(secondary.len())
            .ok_or_else(|| LorebookError::too_many("entry.conditions", MAX_ENTRY_CONDITIONS))?;
        if count > MAX_ENTRY_CONDITIONS {
            return Err(LorebookError::too_many(
                "entry.conditions",
                MAX_ENTRY_CONDITIONS,
            ));
        }
        for condition in self.primary.iter().chain(secondary) {
            condition.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Activation {
    Constant,
    Selective {
        conditions: MatchConditions,
    },
    Probability {
        basis_points: u16,
        conditions: Option<MatchConditions>,
    },
}

impl Activation {
    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::Constant => Ok(()),
            Self::Selective { conditions } => conditions.validate(),
            Self::Probability {
                basis_points,
                conditions,
            } => {
                if *basis_points > 10_000 {
                    return Err(LorebookError::invalid(
                        "entry.activation.basisPoints",
                        "must be in 0..=10000",
                    ));
                }
                if let Some(conditions) = conditions {
                    conditions.validate()?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoreEntry {
    pub(crate) id: EntryId,
    pub(crate) source: EntrySource,
    pub(crate) priority: i32,
    pub(crate) order: i32,
    pub(crate) enabled: bool,
    pub(crate) activation: Activation,
    pub(crate) content: String,
    pub(crate) reserved_tokens: u32,
}

impl fmt::Debug for LoreEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoreEntry")
            .field("id", &self.id)
            .field("source", &self.source)
            .field("priority", &self.priority)
            .field("order", &self.order)
            .field("enabled", &self.enabled)
            .field("activation", &self.activation)
            .field("content_bytes", &self.content.len())
            .field("reserved_tokens", &self.reserved_tokens)
            .finish()
    }
}

impl LoreEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: EntryId,
        source: EntrySource,
        priority: i32,
        order: i32,
        enabled: bool,
        activation: Activation,
        content: impl Into<String>,
        reserved_tokens: u32,
    ) -> Result<Self> {
        let value = Self {
            id,
            source,
            priority,
            order,
            enabled,
            activation,
            content: content.into(),
            reserved_tokens,
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.content.is_empty() {
            return Err(LorebookError::invalid("entry.content", "must not be empty"));
        }
        if self.content.len() > MAX_ENTRY_CONTENT_BYTES {
            return Err(LorebookError::too_large(
                "entry.content",
                MAX_ENTRY_CONTENT_BYTES,
            ));
        }
        if self.reserved_tokens == 0 || self.reserved_tokens > MAX_OUTPUT_TOKENS {
            return Err(LorebookError::invalid(
                "entry.reservedTokens",
                "must be within the output token ceiling",
            ));
        }
        self.activation.validate()
    }

    #[must_use]
    pub const fn id(&self) -> &EntryId {
        &self.id
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LorebookCatalog {
    revision: u64,
    entries: Vec<LoreEntry>,
}

impl fmt::Debug for LorebookCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LorebookCatalog")
            .field("revision", &self.revision)
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

impl LorebookCatalog {
    pub fn new(revision: u64, entries: Vec<LoreEntry>) -> Result<Self> {
        let value = Self { revision, entries };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.revision == 0 {
            return Err(LorebookError::invalid(
                "catalog.revision",
                "must be at least one",
            ));
        }
        if self.entries.len() > MAX_CATALOG_ENTRIES {
            return Err(LorebookError::too_many(
                "catalog.entries",
                MAX_CATALOG_ENTRIES,
            ));
        }
        let mut ids = BTreeSet::new();
        let mut bytes = 0usize;
        for entry in &self.entries {
            entry.validate()?;
            if !ids.insert(entry.id.as_str()) {
                return Err(LorebookError::DuplicateEntryId);
            }
            bytes = bytes
                .checked_add(entry.id.as_str().len())
                .and_then(|value| value.checked_add(entry.content.len()))
                .ok_or_else(|| LorebookError::too_large("catalog", MAX_CATALOG_BYTES))?;
            let conditions = match &entry.activation {
                Activation::Constant => None,
                Activation::Selective { conditions } => Some(conditions),
                Activation::Probability { conditions, .. } => conditions.as_ref(),
            };
            if let Some(conditions) = conditions {
                for condition in conditions
                    .primary
                    .iter()
                    .chain(match &conditions.secondary {
                        SecondaryConditions::None => &[][..],
                        SecondaryConditions::Any(values) | SecondaryConditions::All(values) => {
                            values
                        }
                    })
                {
                    bytes = bytes
                        .checked_add(match condition {
                            MatchCondition::Literal { value, .. } => value.len(),
                            MatchCondition::Regex {
                                pattern,
                                required_literal,
                                ..
                            } => pattern.len() + required_literal.len(),
                        })
                        .ok_or_else(|| LorebookError::too_large("catalog", MAX_CATALOG_BYTES))?;
                }
            }
            if bytes > MAX_CATALOG_BYTES {
                return Err(LorebookError::too_large("catalog", MAX_CATALOG_BYTES));
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn entries(&self) -> &[LoreEntry] {
        &self.entries
    }

    pub(crate) fn entries_mut(&mut self) -> &mut [LoreEntry] {
        &mut self.entries
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageState {
    Complete,
    Partial,
    Failed,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatTurn {
    pub revision: u64,
    pub state: MessageState,
    pub content: String,
}

impl fmt::Debug for ChatTurn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatTurn")
            .field("revision", &self.revision)
            .field("state", &self.state)
            .field("content_bytes", &self.content.len())
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SummarySnapshot {
    Missing,
    Corrupt { revision: u64 },
    Available { revision: u64, content: String },
}

impl fmt::Debug for SummarySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("Missing"),
            Self::Corrupt { revision } => formatter
                .debug_struct("Corrupt")
                .field("revision", revision)
                .finish(),
            Self::Available { revision, content } => formatter
                .debug_struct("Available")
                .field("revision", revision)
                .field("content_bytes", &content.len())
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationSnapshot {
    pub chat_revision: u64,
    pub branch_revision: u64,
    pub total_turns: u64,
    /// A storage adapter must provide only a bounded recent tail here. The
    /// engine never asks for or walks earlier history.
    pub recent_turns: Vec<ChatTurn>,
    pub summary: SummarySnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialMessagePolicy {
    Exclude,
    Include,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionSettings {
    pub search_depth: usize,
    pub partial_messages: PartialMessagePolicy,
    pub max_output_tokens: u32,
    pub max_output_bytes: usize,
    pub max_literal_match_events: usize,
    pub max_regex_evaluations: usize,
    pub max_regex_scan_bytes: usize,
    pub max_regex_matches: usize,
    pub tokenizer_id: String,
    pub tokenizer_revision: String,
    pub separator: String,
}

impl fmt::Debug for SelectionSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectionSettings")
            .field("search_depth", &self.search_depth)
            .field("partial_messages", &self.partial_messages)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("max_literal_match_events", &self.max_literal_match_events)
            .field("max_regex_evaluations", &self.max_regex_evaluations)
            .field("max_regex_scan_bytes", &self.max_regex_scan_bytes)
            .field("max_regex_matches", &self.max_regex_matches)
            .field("tokenizer_id_bytes", &self.tokenizer_id.len())
            .field("tokenizer_revision_bytes", &self.tokenizer_revision.len())
            .field("separator_bytes", &self.separator.len())
            .finish()
    }
}

impl SelectionSettings {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.search_depth > MAX_RECENT_TURNS {
            return Err(LorebookError::too_many("searchDepth", MAX_RECENT_TURNS));
        }
        if self.max_output_tokens > MAX_OUTPUT_TOKENS {
            return Err(LorebookError::invalid(
                "maxOutputTokens",
                "exceeds the product ceiling",
            ));
        }
        if self.max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(LorebookError::too_large("output", MAX_OUTPUT_BYTES));
        }
        if self.max_literal_match_events > MAX_LITERAL_MATCH_EVENTS {
            return Err(LorebookError::too_many(
                "literalMatchEvents",
                MAX_LITERAL_MATCH_EVENTS,
            ));
        }
        if self.max_regex_evaluations > MAX_REGEX_EVALUATIONS {
            return Err(LorebookError::too_many(
                "regexEvaluations",
                MAX_REGEX_EVALUATIONS,
            ));
        }
        if self.max_regex_scan_bytes > MAX_REGEX_SCAN_BYTES {
            return Err(LorebookError::too_large("regexScan", MAX_REGEX_SCAN_BYTES));
        }
        if self.max_regex_matches > MAX_REGEX_MATCHES {
            return Err(LorebookError::too_many("regexMatches", MAX_REGEX_MATCHES));
        }
        if self.tokenizer_id.is_empty() || self.tokenizer_id.len() > MAX_TOKENIZER_ID_BYTES {
            return Err(LorebookError::invalid(
                "tokenizerId",
                "must be a bounded non-empty identifier",
            ));
        }
        if self.tokenizer_revision.is_empty()
            || self.tokenizer_revision.len() > MAX_TOKENIZER_REVISION_BYTES
        {
            return Err(LorebookError::invalid(
                "tokenizerRevision",
                "must be a bounded non-empty identifier",
            ));
        }
        if self.separator.len() > MAX_KEY_BYTES {
            return Err(LorebookError::too_large("separator", MAX_KEY_BYTES));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SelectionRequest {
    pub conversation: ConversationSnapshot,
    pub settings: SelectionSettings,
    pub seed: u64,
}

impl SelectionRequest {
    pub(crate) fn validate(&self) -> Result<()> {
        self.settings.validate()?;
        if self.conversation.recent_turns.len() > MAX_RECENT_TURNS {
            return Err(LorebookError::too_many("recentTurns", MAX_RECENT_TURNS));
        }
        let recent_count = u64::try_from(self.conversation.recent_turns.len())
            .map_err(|_| LorebookError::invalid("recentTurns", "length cannot be represented"))?;
        if recent_count > self.conversation.total_turns {
            return Err(LorebookError::invalid(
                "totalTurns",
                "cannot be smaller than the supplied recent tail",
            ));
        }
        let mut bytes = 0usize;
        for turn in &self.conversation.recent_turns {
            if turn.content.len() > MAX_TURN_BYTES {
                return Err(LorebookError::too_large("turn.content", MAX_TURN_BYTES));
            }
            bytes = bytes
                .checked_add(turn.content.len())
                .ok_or_else(|| LorebookError::too_large("searchInput", MAX_SEARCH_INPUT_BYTES))?;
        }
        if let SummarySnapshot::Available { content, .. } = &self.conversation.summary {
            if content.len() > MAX_TURN_BYTES {
                return Err(LorebookError::too_large("summary.content", MAX_TURN_BYTES));
            }
            bytes = bytes
                .checked_add(content.len())
                .ok_or_else(|| LorebookError::too_large("searchInput", MAX_SEARCH_INPUT_BYTES))?;
        }
        if bytes > MAX_SEARCH_INPUT_BYTES {
            return Err(LorebookError::too_large(
                "searchInput",
                MAX_SEARCH_INPUT_BYTES,
            ));
        }
        Ok(())
    }
}
