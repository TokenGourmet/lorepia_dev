#![forbid(unsafe_code)]

mod compile;
mod error;
mod import;
mod regex_pipeline;
mod schema;
mod template;
mod validate;

pub use compile::{
    CompiledPrompt, CompiledPromptRequest, ExactTokenCounter, ExactTokenInput,
    MAX_LONG_TERM_MEMORY_INPUT_BYTES, MAX_PERSONA_INPUT_BYTES, MessageSource, ModelCapacity,
    ModelPresetSnapshot, PromptBindingMode, PromptCompileInput, TokenCount, compile_prompt,
    compile_prompt_request,
};
pub use error::{
    ExactTokenCountError, ExactTokenCountErrorKind, ExactTokenResult, PromptError, Result,
};
pub use import::{IMPORT_FORMAT, IMPORT_SCHEMA_VERSION, export_preset, import_preset};
pub use regex_pipeline::apply_text_transforms;
pub use schema::{
    AdvancedSettings, CachePoint, ChatEnd, ChatSelection, ContentFormat, ModuleReference,
    PromptBlock, PromptBlockKind, PromptPreset, PromptRole, PromptSampling, PromptTemplateSettings,
    RawPromptSpecial, RegexFlags, RegexRule, SyntheticMessage, ToolReference, TransformTarget,
};
pub use validate::validate_preset;
