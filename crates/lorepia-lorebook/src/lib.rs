#![forbid(unsafe_code)]

//! UI-independent, bounded lorebook selection.
//!
//! Lore text is opaque data. This crate does not expand variables, evaluate
//! macros, execute code, or assign provider message roles. Its output is a
//! plain string that the existing `lorepia-prompt` compiler may consume.

mod engine;
mod error;
mod import;
mod model;
mod normalize;

pub use engine::{LorebookEngine, Selection, SelectionReceipt};
pub use error::{LimitKind, LorebookError, Result};
pub use import::{
    IMPORT_FORMAT, IMPORT_SCHEMA_VERSION, ImportTrust, export_catalog, import_catalog,
    import_catalog_with_trust, import_trusted_engine,
};
pub use model::{
    Activation, ChatTurn, ConversationSnapshot, EntryId, EntrySource, LoreEntry, LorebookCatalog,
    MatchCondition, MatchConditions, MessageState, PartialMessagePolicy, SecondaryConditions,
    SelectionRequest, SelectionSettings, SummarySnapshot,
};
pub use normalize::normalize_search_text;

pub const MAX_CATALOG_ENTRIES: usize = 100_000;
pub const MAX_CATALOG_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ENTRY_CONTENT_BYTES: usize = 256 * 1024;
pub const MAX_ENTRY_CONDITIONS: usize = 64;
pub const MAX_KEY_BYTES: usize = 4 * 1024;
pub const MAX_REGEX_BYTES: usize = 4 * 1024;
pub const MAX_RECENT_TURNS: usize = 512;
pub const MAX_TURN_BYTES: usize = 256 * 1024;
pub const MAX_SEARCH_INPUT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_OUTPUT_TOKENS: u32 = 1_000_000;
pub const MAX_REGEX_SCAN_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_REGEX_MATCHES: usize = 8_192;
pub const MAX_REGEX_EVALUATIONS: usize = 1_024;
pub const MAX_LITERAL_MATCH_EVENTS: usize = 4 * 1024 * 1024;
pub const MAX_ACTIVE_REGEX_CONDITIONS: usize = 1_024;
pub const MAX_ACTIVE_REGEX_PATTERN_BYTES: usize = 1024 * 1024;
pub const MAX_IMPORT_BYTES: usize = 72 * 1024 * 1024;
