use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
};

use lorepia_providers::{
    ChatMessage, CompiledProviderRequest, GenerationOptions, ProviderId, ProviderOptions,
    ProviderRequest, TokenizerOverride, compile_request,
};
use serde_json::Value;

use crate::{
    CachePoint, ChatEnd, ContentFormat, ExactTokenResult, ModuleReference, PromptBlock,
    PromptBlockKind, PromptError, PromptPreset, PromptRole, PromptSampling, Result, ToolReference,
    TransformTarget,
    regex_pipeline::CompiledTransformPipeline,
    template::{render_content, render_template, validate_variable_name},
    validate::{MAX_AUTHORED_TEXT_BYTES, MAX_VARIABLE_VALUE_BYTES, MAX_VARIABLES},
    validate_preset,
};

const MAX_COMPILED_MESSAGES: usize = 10_000;
const MAX_COMPILED_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_AVAILABLE_REFERENCES: usize = 256;
pub const MAX_PERSONA_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_LONG_TERM_MEMORY_INPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PromptBindingMode {
    #[default]
    Classic,
    ModelPreset {
        use_prompt_parameters: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromptCompileInput {
    pub history: Vec<ChatMessage>,
    pub persona: String,
    pub character_description: String,
    pub author_note: Option<String>,
    pub lorebook: String,
    pub long_term_memory: String,
    pub final_insertion: String,
    pub variables: BTreeMap<String, String>,
    pub available_tool_ids: BTreeSet<String>,
    pub available_modules: Vec<ModuleReference>,
    pub binding_mode: PromptBindingMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelCapacity {
    pub max_context_tokens: u32,
    pub max_output_tokens: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelPresetSnapshot {
    pub provider: ProviderId,
    pub model_id: String,
    pub capacity: ModelCapacity,
    pub provider_options: ProviderOptions,
    pub tokenizer_override: Option<TokenizerOverride>,
    pub additional_parameters: BTreeMap<String, Value>,
    /// Model-owned baseline. Prompt-owned sampling may selectively overlay
    /// sampling fields, but capacity always replaces `max_output_tokens`.
    pub generation: GenerationOptions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenCount {
    /// A deterministic local estimate. It is not a model-tokenizer result.
    pub approximate: usize,
    /// Reserved for an exact model-tokenizer result supplied by a later layer.
    pub exact: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageSource {
    Preamble,
    Block {
        block_index: usize,
        kind: PromptBlockKind,
    },
    History {
        block_index: usize,
        history_index: usize,
    },
    Epilogue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledPrompt {
    pub messages: Vec<ChatMessage>,
    /// One entry per message, in the same order as `messages`.
    pub source_map: Vec<MessageSource>,
    pub cache_points: Vec<CachePoint>,
    pub active_tools: Vec<ToolReference>,
    pub active_modules: Vec<ModuleReference>,
    /// `None` means that the active model-preset binding declined prompt-owned
    /// sampling parameters.
    pub sampling: Option<PromptSampling>,
    pub token_count: TokenCount,
}

#[derive(Debug, PartialEq)]
pub struct CompiledPromptRequest {
    provider_request: ProviderRequest,
    compiled_provider_request: CompiledProviderRequest,
    /// Kept outside `ProviderRequest`: context capacity is model metadata, not
    /// a prompt sampling parameter.
    max_context_tokens: u32,
    /// Exact input count returned by the model-specific tokenizer gate.
    input_tokens: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ExactTokenInput<'a> {
    request: &'a ProviderRequest,
    compiled: &'a CompiledProviderRequest,
}

impl<'a> ExactTokenInput<'a> {
    #[must_use]
    pub const fn new(request: &'a ProviderRequest, compiled: &'a CompiledProviderRequest) -> Self {
        Self { request, compiled }
    }

    #[must_use]
    pub const fn request(self) -> &'a ProviderRequest {
        self.request
    }

    #[must_use]
    pub const fn compiled(self) -> &'a CompiledProviderRequest {
        self.compiled
    }
}

impl CompiledPromptRequest {
    #[must_use]
    pub fn provider_request(&self) -> &ProviderRequest {
        &self.provider_request
    }

    #[must_use]
    pub fn compiled_provider_request(&self) -> &CompiledProviderRequest {
        &self.compiled_provider_request
    }

    #[must_use]
    pub const fn max_context_tokens(&self) -> u32 {
        self.max_context_tokens
    }

    #[must_use]
    pub const fn input_tokens(&self) -> usize {
        self.input_tokens
    }
}

/// Model-specific tokenizer boundary required before a prompt becomes a
/// sendable provider request.
pub trait ExactTokenCounter: Send + Sync {
    fn count_input_tokens<'a>(
        &'a self,
        input: ExactTokenInput<'a>,
    ) -> Pin<Box<dyn Future<Output = ExactTokenResult<usize>> + Send + 'a>>;
}

pub fn compile_prompt(preset: &PromptPreset, input: &PromptCompileInput) -> Result<CompiledPrompt> {
    validate_preset(preset)?;
    validate_runtime_input(input)?;
    validate_runtime_references(preset, input)?;

    let variables = merged_variables(preset, input)?;
    let mut state = CompileState::default();

    if let Some(preamble) = &preset.advanced.template.preamble {
        let content = render_template(&preamble.template, &variables)?;
        state.push(
            ChatMessage::new(preamble.role.into(), content),
            MessageSource::Preamble,
        )?;
    }

    let mut main_seen = false;
    let mut global_note_seen = false;
    for (block_index, block) in preset.blocks.iter().enumerate() {
        if !block.enabled() {
            continue;
        }
        let source = MessageSource::Block {
            block_index,
            kind: block.kind(),
        };

        match block {
            PromptBlock::Raw {
                role,
                special,
                prompt,
                ..
            } => {
                if let Some(special) = special {
                    if *role != PromptRole::System {
                        return Err(PromptError::invalid(
                            format!("blocks[{block_index}].role"),
                            "special raw prompts must use the system role",
                        ));
                    }
                    let already_seen = match special {
                        crate::RawPromptSpecial::Main => &mut main_seen,
                        crate::RawPromptSpecial::GlobalNote => &mut global_note_seen,
                    };
                    if *already_seen {
                        return Err(PromptError::invalid(
                            format!("blocks[{block_index}].special"),
                            "an enabled raw special may appear only once",
                        ));
                    }
                    *already_seen = true;
                }
                let content = render_template(prompt, &variables)?;
                state.push(ChatMessage::new((*role).into(), content), source)?;
            }
            PromptBlock::Chat { selection, .. } => {
                let end = match selection.end {
                    ChatEnd::Index(end) => end,
                    ChatEnd::EndOfChat => input.history.len(),
                };
                if selection.start >= end || end > input.history.len() {
                    return Err(PromptError::invalid(
                        format!("blocks[{block_index}].selection"),
                        format!(
                            "history range must be a non-empty half-open interval within 0..{}",
                            input.history.len()
                        ),
                    ));
                }
                for history_index in selection.start..end {
                    state.push(
                        input.history[history_index].clone(),
                        MessageSource::History {
                            block_index,
                            history_index,
                        },
                    )?;
                }
            }
            PromptBlock::Persona { role, format, .. } => push_dynamic_content(
                &mut state,
                *role,
                format,
                &input.persona,
                &variables,
                source,
            )?,
            PromptBlock::CharacterDescription { role, format, .. } => push_dynamic_content(
                &mut state,
                *role,
                format,
                &input.character_description,
                &variables,
                source,
            )?,
            PromptBlock::AuthorNote {
                role,
                default_prompt,
                format,
                ..
            } => {
                let authored_fallback;
                let value = if let Some(value) = input.author_note.as_deref() {
                    value
                } else if let Some(default_prompt) = default_prompt {
                    authored_fallback = render_template(default_prompt, &variables)?;
                    &authored_fallback
                } else {
                    ""
                };
                push_dynamic_content(&mut state, *role, format, value, &variables, source)?;
            }
            PromptBlock::Lorebook { role, format, .. } => push_dynamic_content(
                &mut state,
                *role,
                format,
                &input.lorebook,
                &variables,
                source,
            )?,
            PromptBlock::LongTermMemory { role, format, .. } => push_dynamic_content(
                &mut state,
                *role,
                format,
                &input.long_term_memory,
                &variables,
                source,
            )?,
            PromptBlock::FinalInsertion { role, prompt, .. } => {
                // Runtime content is data, not another authored template. It
                // must never gain variable expansion by containing `${...}`.
                let content = if input.final_insertion.is_empty() {
                    render_template(prompt, &variables)?
                } else {
                    input.final_insertion.clone()
                };
                state.push(ChatMessage::new((*role).into(), content), source)?;
            }
            PromptBlock::ChatMl { role, prompt, .. } => {
                // ChatML is represented as typed message content here. This
                // compiler never reparses it into provider roles or wire data.
                let content = render_template(prompt, &variables)?;
                state.push(ChatMessage::new((*role).into(), content), source)?;
            }
            PromptBlock::CachePoint { depth, role, .. } => {
                state.pending_cache_points.push(PendingCachePoint {
                    block_index,
                    depth: *depth,
                    role: *role,
                });
            }
        }
    }

    if let Some(epilogue) = &preset.advanced.template.epilogue {
        let content = render_template(&epilogue.template, &variables)?;
        state.push(
            ChatMessage::new(epilogue.role.into(), content),
            MessageSource::Epilogue,
        )?;
    }
    if state.messages.is_empty() {
        return Err(PromptError::invalid(
            "compiled messages",
            "at least one message is required",
        ));
    }
    state.resolve_cache_points()?;

    let mut request_transforms = CompiledTransformPipeline::new(preset, TransformTarget::Request)?;
    let mut transformed_bytes = 0usize;
    for message in &mut state.messages {
        message.content = request_transforms.apply(&message.content)?;
        validate_message_content(&message.content)?;
        transformed_bytes = transformed_bytes
            .checked_add(message.content.len())
            .ok_or_else(|| {
                PromptError::too_large("compiled messages", MAX_COMPILED_MESSAGE_BYTES)
            })?;
        if transformed_bytes > MAX_COMPILED_MESSAGE_BYTES {
            return Err(PromptError::too_large(
                "compiled messages",
                MAX_COMPILED_MESSAGE_BYTES,
            ));
        }
    }

    let sampling = match input.binding_mode {
        PromptBindingMode::Classic => Some(preset.sampling.clone()),
        PromptBindingMode::ModelPreset {
            use_prompt_parameters: true,
        } => Some(preset.sampling.clone()),
        PromptBindingMode::ModelPreset {
            use_prompt_parameters: false,
        } => None,
    };
    let token_count = approximate_tokens(&state.messages);

    debug_assert_eq!(state.messages.len(), state.source_map.len());
    Ok(CompiledPrompt {
        messages: state.messages,
        source_map: state.source_map,
        cache_points: state.cache_points,
        active_tools: preset
            .advanced
            .tools
            .iter()
            .filter(|reference| reference.enabled)
            .cloned()
            .collect(),
        active_modules: preset
            .advanced
            .modules
            .iter()
            .filter(|reference| reference.enabled)
            .cloned()
            .collect(),
        sampling,
        token_count,
    })
}

pub async fn compile_prompt_request(
    prompt: &CompiledPrompt,
    model: &ModelPresetSnapshot,
    token_counter: &impl ExactTokenCounter,
) -> Result<CompiledPromptRequest> {
    validate_capacity(model.capacity)?;
    if !prompt.cache_points.is_empty() {
        return Err(unsupported(
            "cache_points",
            "the provider request contract has no reviewed cache-boundary mapping",
        ));
    }
    if !prompt.active_tools.is_empty() {
        return Err(unsupported(
            "tools",
            "tool references require the separate approved tool-call loop",
        ));
    }
    if !prompt.active_modules.is_empty() {
        return Err(unsupported(
            "modules",
            "module references are metadata only until a module runtime is reviewed",
        ));
    }

    let sampling = prompt.sampling.as_ref();
    if sampling.and_then(|value| value.min_p).is_some() {
        return Err(unsupported(
            "min_p",
            "the provider-neutral request contract does not expose min-p",
        ));
    }
    if sampling.and_then(|value| value.top_a).is_some() {
        return Err(unsupported(
            "top_a",
            "the provider-neutral request contract does not expose top-a",
        ));
    }
    if sampling
        .and_then(|value| value.repetition_penalty)
        .is_some()
    {
        return Err(unsupported(
            "repetition_penalty",
            "the provider-neutral request contract does not expose repetition penalty",
        ));
    }

    let mut generation = model.generation.clone();
    generation.max_output_tokens = Some(model.capacity.max_output_tokens);
    if let Some(sampling) = sampling {
        overlay_sampling(&mut generation, sampling);
    }
    let provider_request = ProviderRequest {
        provider: model.provider,
        model_id: model.model_id.clone(),
        messages: prompt.messages.clone(),
        generation,
        provider_options: model.provider_options.clone(),
        tokenizer_override: model.tokenizer_override.clone(),
        additional_parameters: model.additional_parameters.clone(),
    };

    // The adapter does not return an unchecked request. Provider capability,
    // message-role, model, and option validation all run before hand-off.
    let compiled_provider_request = compile_request(&provider_request)?;
    let input_tokens = token_counter
        .count_input_tokens(ExactTokenInput::new(
            &provider_request,
            &compiled_provider_request,
        ))
        .await?;
    validate_context_capacity(input_tokens, model.capacity)?;
    Ok(CompiledPromptRequest {
        provider_request,
        compiled_provider_request,
        max_context_tokens: model.capacity.max_context_tokens,
        input_tokens,
    })
}

#[derive(Default)]
struct CompileState {
    messages: Vec<ChatMessage>,
    source_map: Vec<MessageSource>,
    cache_points: Vec<CachePoint>,
    pending_cache_points: Vec<PendingCachePoint>,
    message_bytes: usize,
    conversation_started: bool,
}

#[derive(Clone, Copy, Debug)]
struct PendingCachePoint {
    block_index: usize,
    depth: usize,
    role: PromptRole,
}

impl CompileState {
    fn push(&mut self, message: ChatMessage, source: MessageSource) -> Result<()> {
        validate_message_content(&message.content)?;
        if self.messages.len() >= MAX_COMPILED_MESSAGES {
            return Err(PromptError::too_many(
                "compiled messages",
                MAX_COMPILED_MESSAGES,
            ));
        }
        if message.role == lorepia_providers::MessageRole::System && self.conversation_started {
            return Err(PromptError::invalid(
                "compiled messages.role",
                "system messages must precede the conversation",
            ));
        }
        if message.role != lorepia_providers::MessageRole::System {
            self.conversation_started = true;
        }
        self.message_bytes = self
            .message_bytes
            .checked_add(message.content.len())
            .ok_or_else(|| {
                PromptError::too_large("compiled messages", MAX_COMPILED_MESSAGE_BYTES)
            })?;
        if self.message_bytes > MAX_COMPILED_MESSAGE_BYTES {
            return Err(PromptError::too_large(
                "compiled messages",
                MAX_COMPILED_MESSAGE_BYTES,
            ));
        }
        self.messages.push(message);
        self.source_map.push(source);
        Ok(())
    }

    fn resolve_cache_points(&mut self) -> Result<()> {
        let mut targets = BTreeSet::new();
        for pending in &self.pending_cache_points {
            if pending.depth == 0 || pending.depth > 16 || pending.depth > self.messages.len() {
                return Err(PromptError::invalid(
                    format!("blocks[{}].depth", pending.block_index),
                    format!(
                        "cache depth must be in 1..=16 and address one of the {} final messages",
                        self.messages.len()
                    ),
                ));
            }
            let message_index = self.messages.len() - pending.depth;
            if !targets.insert(message_index) {
                return Err(PromptError::invalid(
                    format!("blocks[{}].depth", pending.block_index),
                    "multiple cache points must not address the same final message",
                ));
            }
            if self.messages[message_index].role != pending.role.into() {
                return Err(PromptError::invalid(
                    format!("blocks[{}].role", pending.block_index),
                    "cache-point role must match the addressed final message",
                ));
            }
            self.cache_points.push(CachePoint {
                block_index: pending.block_index,
                message_index,
                depth: pending.depth,
                role: pending.role,
            });
        }
        Ok(())
    }
}

fn push_dynamic_content(
    state: &mut CompileState,
    role: PromptRole,
    format: &ContentFormat,
    source_value: &str,
    variables: &BTreeMap<String, String>,
    source: MessageSource,
) -> Result<()> {
    if source_value.is_empty() {
        return Ok(());
    }
    let content = render_content(format, source_value, variables)?;
    state.push(ChatMessage::new(role.into(), content), source)
}

fn merged_variables(
    preset: &PromptPreset,
    input: &PromptCompileInput,
) -> Result<BTreeMap<String, String>> {
    let mut variables = preset.advanced.template.default_variables.clone();
    variables.extend(input.variables.clone());
    if variables.len() > MAX_VARIABLES {
        return Err(PromptError::too_many("merged variables", MAX_VARIABLES));
    }
    let total_bytes = variables.values().try_fold(0usize, |total, value| {
        total
            .checked_add(value.len())
            .ok_or_else(|| PromptError::too_large("merged variables", MAX_AUTHORED_TEXT_BYTES))
    })?;
    if total_bytes > MAX_AUTHORED_TEXT_BYTES {
        return Err(PromptError::too_large(
            "merged variables",
            MAX_AUTHORED_TEXT_BYTES,
        ));
    }
    Ok(variables)
}

fn validate_runtime_input(input: &PromptCompileInput) -> Result<()> {
    if input.persona.len() > MAX_PERSONA_INPUT_BYTES {
        return Err(PromptError::too_large("persona", MAX_PERSONA_INPUT_BYTES));
    }
    if input.persona.contains('\0') {
        return Err(PromptError::invalid("persona", "must contain no NUL"));
    }
    if input.long_term_memory.len() > MAX_LONG_TERM_MEMORY_INPUT_BYTES {
        return Err(PromptError::too_large(
            "longTermMemory",
            MAX_LONG_TERM_MEMORY_INPUT_BYTES,
        ));
    }
    if input.long_term_memory.contains('\0') {
        return Err(PromptError::invalid(
            "longTermMemory",
            "must contain no NUL",
        ));
    }
    if input.variables.len() > MAX_VARIABLES {
        return Err(PromptError::too_many("runtime variables", MAX_VARIABLES));
    }
    let mut variable_bytes = 0usize;
    for (name, value) in &input.variables {
        if !validate_variable_name(name) || name == "value" {
            return Err(PromptError::invalid(
                "runtime variables",
                "invalid or reserved variable name",
            ));
        }
        if value.contains('\0') {
            return Err(PromptError::invalid(
                format!("runtime variables.{name}"),
                "must contain no NUL",
            ));
        }
        if value.len() > MAX_VARIABLE_VALUE_BYTES {
            return Err(PromptError::too_large(
                format!("runtime variables.{name}"),
                MAX_VARIABLE_VALUE_BYTES,
            ));
        }
        variable_bytes = variable_bytes
            .checked_add(value.len())
            .ok_or_else(|| PromptError::too_large("runtime variables", MAX_AUTHORED_TEXT_BYTES))?;
        if variable_bytes > MAX_AUTHORED_TEXT_BYTES {
            return Err(PromptError::too_large(
                "runtime variables",
                MAX_AUTHORED_TEXT_BYTES,
            ));
        }
    }
    if input.available_tool_ids.len() > MAX_AVAILABLE_REFERENCES {
        return Err(PromptError::too_many(
            "available tool references",
            MAX_AVAILABLE_REFERENCES,
        ));
    }
    if input.available_modules.len() > MAX_AVAILABLE_REFERENCES {
        return Err(PromptError::too_many(
            "available module references",
            MAX_AVAILABLE_REFERENCES,
        ));
    }
    Ok(())
}

fn validate_runtime_references(preset: &PromptPreset, input: &PromptCompileInput) -> Result<()> {
    for reference in preset
        .advanced
        .tools
        .iter()
        .filter(|reference| reference.enabled)
    {
        if !input.available_tool_ids.contains(&reference.tool_id) {
            return Err(PromptError::invalid(
                "advanced.tools",
                format!("active tool is unavailable: {}", reference.tool_id),
            ));
        }
    }
    for reference in preset
        .advanced
        .modules
        .iter()
        .filter(|reference| reference.enabled)
    {
        if !input.available_modules.iter().any(|available| {
            available.module_id == reference.module_id
                && available.version == reference.version
                && available.digest_sha256 == reference.digest_sha256
                && available.enabled
        }) {
            return Err(PromptError::invalid(
                "advanced.modules",
                format!(
                    "active pinned module is unavailable: {}@{}",
                    reference.module_id, reference.version
                ),
            ));
        }
    }
    Ok(())
}

fn validate_capacity(capacity: ModelCapacity) -> Result<()> {
    if capacity.max_context_tokens == 0 {
        return Err(PromptError::invalid(
            "model.capacity.maxContextTokens",
            "must be greater than zero",
        ));
    }
    if capacity.max_output_tokens == 0 || capacity.max_output_tokens > capacity.max_context_tokens {
        return Err(PromptError::invalid(
            "model.capacity.maxOutputTokens",
            "must be greater than zero and no larger than max context tokens",
        ));
    }
    Ok(())
}

fn validate_context_capacity(input_tokens: usize, capacity: ModelCapacity) -> Result<()> {
    let required = input_tokens
        .checked_add(capacity.max_output_tokens as usize)
        .ok_or_else(|| {
            PromptError::invalid("model.capacity", "input plus output token count overflowed")
        })?;
    if required > capacity.max_context_tokens as usize {
        return Err(PromptError::invalid(
            "model.capacity.maxContextTokens",
            format!(
                "exact input ({input_tokens}) plus max output ({}) exceeds context capacity ({})",
                capacity.max_output_tokens, capacity.max_context_tokens
            ),
        ));
    }
    Ok(())
}

fn overlay_sampling(generation: &mut GenerationOptions, sampling: &PromptSampling) {
    if let Some(value) = sampling.temperature {
        generation.temperature = Some(value);
    }
    if let Some(value) = sampling.top_k {
        generation.top_k = Some(value);
    }
    if let Some(value) = sampling.top_p {
        generation.top_p = Some(value);
    }
    if let Some(value) = sampling.presence_penalty {
        generation.presence_penalty = Some(value);
    }
    if let Some(value) = sampling.frequency_penalty {
        generation.frequency_penalty = Some(value);
    }
    if !sampling.stop_sequences.is_empty() {
        generation
            .stop_sequences
            .clone_from(&sampling.stop_sequences);
    }
    if let Some(value) = sampling.seed {
        generation.seed = Some(value);
    }
}

fn validate_message_content(content: &str) -> Result<()> {
    if content.trim().is_empty() {
        return Err(PromptError::invalid(
            "compiled message.content",
            "must not be empty",
        ));
    }
    if content.contains('\0') {
        return Err(PromptError::invalid(
            "compiled message.content",
            "must not contain NUL",
        ));
    }
    if content.len() > MAX_COMPILED_MESSAGE_BYTES {
        return Err(PromptError::too_large(
            "compiled message.content",
            MAX_COMPILED_MESSAGE_BYTES,
        ));
    }
    Ok(())
}

fn approximate_tokens(messages: &[ChatMessage]) -> TokenCount {
    let characters = messages
        .iter()
        .map(|message| message.content.chars().count())
        .sum::<usize>();
    TokenCount {
        approximate: characters.div_ceil(4),
        exact: None,
    }
}

fn unsupported(feature: &str, reason: &str) -> PromptError {
    PromptError::UnsupportedFeature {
        feature: feature.to_owned(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AdvancedSettings, ChatSelection, PromptTemplateSettings, RawPromptSpecial, RegexFlags,
        RegexRule,
    };
    use lorepia_providers::{OpenAiOptions, ProviderOptions};

    fn preset(blocks: Vec<PromptBlock>) -> PromptPreset {
        PromptPreset {
            name: "test".to_owned(),
            blocks,
            sampling: PromptSampling::default(),
            advanced: AdvancedSettings::default(),
        }
    }

    fn input(history: Vec<ChatMessage>) -> PromptCompileInput {
        PromptCompileInput {
            history,
            ..PromptCompileInput::default()
        }
    }

    fn raw(role: PromptRole, prompt: &str) -> PromptBlock {
        PromptBlock::Raw {
            name: prompt.to_owned(),
            enabled: true,
            role,
            special: None,
            prompt: prompt.to_owned(),
        }
    }

    struct FixedTokenCounter(usize);

    impl ExactTokenCounter for FixedTokenCounter {
        fn count_input_tokens<'a>(
            &'a self,
            _input: ExactTokenInput<'a>,
        ) -> Pin<Box<dyn Future<Output = ExactTokenResult<usize>> + Send + 'a>> {
            Box::pin(async move { Ok(self.0) })
        }
    }

    const COUNT_ONE_HUNDRED_TOKENS: FixedTokenCounter = FixedTokenCounter(100);

    struct InspectingTokenCounter;

    impl ExactTokenCounter for InspectingTokenCounter {
        fn count_input_tokens<'a>(
            &'a self,
            input: ExactTokenInput<'a>,
        ) -> Pin<Box<dyn Future<Output = ExactTokenResult<usize>> + Send + 'a>> {
            Box::pin(async move {
                assert_eq!(
                    input.compiled(),
                    &compile_request(input.request()).expect("counter input must already compile")
                );
                Ok(100)
            })
        }
    }

    #[test]
    fn preserves_block_and_half_open_history_order() {
        let preset = preset(vec![
            raw(PromptRole::System, "system"),
            PromptBlock::Chat {
                name: "chat".to_owned(),
                enabled: true,
                selection: ChatSelection {
                    start: 1,
                    end: ChatEnd::Index(3),
                },
            },
            raw(PromptRole::Assistant, "tail"),
        ]);
        let compiled = compile_prompt(
            &preset,
            &input(vec![
                ChatMessage::new(lorepia_providers::MessageRole::User, "zero"),
                ChatMessage::new(lorepia_providers::MessageRole::User, "one"),
                ChatMessage::new(lorepia_providers::MessageRole::Assistant, "two"),
                ChatMessage::new(lorepia_providers::MessageRole::User, "three"),
            ]),
        )
        .unwrap();
        assert_eq!(
            compiled
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            ["system", "one", "two", "tail"]
        );
        assert!(matches!(
            compiled.source_map[1],
            MessageSource::History {
                history_index: 1,
                ..
            }
        ));
    }

    #[test]
    fn all_ten_prompt_block_types_compile_to_the_owned_ir() {
        let blocks = vec![
            PromptBlock::Raw {
                name: "main".to_owned(),
                enabled: true,
                role: PromptRole::System,
                special: Some(RawPromptSpecial::Main),
                prompt: "main".to_owned(),
            },
            PromptBlock::Persona {
                name: "persona".to_owned(),
                enabled: true,
                role: PromptRole::System,
                format: ContentFormat::Custom {
                    template: "persona: ${value}".to_owned(),
                },
            },
            PromptBlock::CharacterDescription {
                name: "character".to_owned(),
                enabled: true,
                role: PromptRole::System,
                format: ContentFormat::Plain,
            },
            PromptBlock::AuthorNote {
                name: "author".to_owned(),
                enabled: true,
                role: PromptRole::System,
                default_prompt: Some("fallback".to_owned()),
                format: ContentFormat::Plain,
            },
            PromptBlock::Lorebook {
                name: "lore".to_owned(),
                enabled: true,
                role: PromptRole::System,
                format: ContentFormat::Plain,
            },
            PromptBlock::LongTermMemory {
                name: "memory".to_owned(),
                enabled: true,
                role: PromptRole::System,
                format: ContentFormat::Plain,
            },
            PromptBlock::ChatMl {
                name: "chatml".to_owned(),
                enabled: true,
                role: PromptRole::System,
                prompt: "chatml".to_owned(),
            },
            PromptBlock::Chat {
                name: "history".to_owned(),
                enabled: true,
                selection: ChatSelection {
                    start: 0,
                    end: ChatEnd::EndOfChat,
                },
            },
            PromptBlock::FinalInsertion {
                name: "final".to_owned(),
                enabled: true,
                role: PromptRole::User,
                prompt: "final".to_owned(),
            },
            PromptBlock::CachePoint {
                name: "cache".to_owned(),
                enabled: true,
                depth: 1,
                role: PromptRole::User,
            },
        ];
        let compiled = compile_prompt(
            &preset(blocks),
            &PromptCompileInput {
                history: vec![
                    ChatMessage::new(lorepia_providers::MessageRole::User, "hello"),
                    ChatMessage::new(lorepia_providers::MessageRole::Assistant, "hi"),
                ],
                persona: "curious".to_owned(),
                character_description: "character".to_owned(),
                author_note: Some("author note".to_owned()),
                lorebook: "lore".to_owned(),
                long_term_memory: "memory".to_owned(),
                ..PromptCompileInput::default()
            },
        )
        .unwrap();

        assert_eq!(compiled.messages.len(), 10);
        assert_eq!(compiled.source_map.len(), 10);
        assert_eq!(compiled.cache_points[0].message_index, 9);
        assert_eq!(compiled.messages[1].content, "persona: curious");
    }

    #[test]
    fn rejects_history_end_beyond_snapshot() {
        let preset = preset(vec![PromptBlock::Chat {
            name: "chat".to_owned(),
            enabled: true,
            selection: ChatSelection {
                start: 0,
                end: ChatEnd::Index(2),
            },
        }]);
        assert!(
            compile_prompt(
                &preset,
                &input(vec![ChatMessage::new(
                    lorepia_providers::MessageRole::User,
                    "only"
                )])
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_system_after_conversation() {
        let preset = preset(vec![
            raw(PromptRole::User, "hello"),
            raw(PromptRole::System, "late"),
        ]);
        let error = compile_prompt(&preset, &PromptCompileInput::default()).unwrap_err();
        assert!(error.to_string().contains("system messages must precede"));
    }

    #[test]
    fn cache_point_must_match_addressed_role() {
        let preset = preset(vec![
            raw(PromptRole::User, "hello"),
            PromptBlock::CachePoint {
                name: "cache".to_owned(),
                enabled: true,
                depth: 1,
                role: PromptRole::Assistant,
            },
        ]);
        assert!(compile_prompt(&preset, &PromptCompileInput::default()).is_err());
    }

    #[test]
    fn model_binding_can_decline_prompt_sampling() {
        let mut preset = preset(vec![raw(PromptRole::User, "hello")]);
        preset.sampling.temperature = Some(0.7);
        let compiled = compile_prompt(
            &preset,
            &PromptCompileInput {
                binding_mode: PromptBindingMode::ModelPreset {
                    use_prompt_parameters: false,
                },
                ..PromptCompileInput::default()
            },
        )
        .unwrap();
        assert_eq!(compiled.sampling, None);
    }

    #[test]
    fn request_transform_changes_content_without_changing_source_map() {
        let mut preset = preset(vec![raw(PromptRole::User, "hello   world")]);
        preset.advanced.regex.push(RegexRule {
            name: "spaces".to_owned(),
            enabled: true,
            target: TransformTarget::Request,
            pattern: " +".to_owned(),
            replacement: " ".to_owned(),
            flags: RegexFlags::default(),
        });
        let compiled = compile_prompt(&preset, &PromptCompileInput::default()).unwrap();
        assert_eq!(compiled.messages[0].content, "hello world");
        assert_eq!(compiled.source_map.len(), 1);
    }

    #[tokio::test]
    async fn model_capacity_is_the_only_max_output_source() {
        let compiled = compile_prompt(
            &preset(vec![raw(PromptRole::User, "hello")]),
            &PromptCompileInput::default(),
        )
        .unwrap();
        let model = ModelPresetSnapshot {
            provider: ProviderId::OpenAi,
            model_id: "gpt-test".to_owned(),
            capacity: ModelCapacity {
                max_context_tokens: 8192,
                max_output_tokens: 512,
            },
            provider_options: ProviderOptions::OpenAi(OpenAiOptions::default()),
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
            generation: GenerationOptions {
                max_output_tokens: Some(9_999),
                ..GenerationOptions::default()
            },
        };
        let request = compile_prompt_request(&compiled, &model, &COUNT_ONE_HUNDRED_TOKENS)
            .await
            .unwrap();
        assert_eq!(
            request.provider_request().generation.max_output_tokens,
            Some(512)
        );
        assert_eq!(request.max_context_tokens(), 8192);
    }

    #[tokio::test]
    async fn unsupported_sampling_is_not_silently_dropped() {
        let mut preset = preset(vec![raw(PromptRole::User, "hello")]);
        preset.sampling.min_p = Some(0.1);
        let compiled = compile_prompt(&preset, &PromptCompileInput::default()).unwrap();
        let model = ModelPresetSnapshot {
            provider: ProviderId::OpenAi,
            model_id: "gpt-test".to_owned(),
            capacity: ModelCapacity {
                max_context_tokens: 8192,
                max_output_tokens: 512,
            },
            provider_options: ProviderOptions::OpenAi(OpenAiOptions::default()),
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
            generation: GenerationOptions::default(),
        };
        assert!(matches!(
            compile_prompt_request(&compiled, &model, &COUNT_ONE_HUNDRED_TOKENS).await,
            Err(PromptError::UnsupportedFeature { feature, .. }) if feature == "min_p"
        ));
    }

    #[test]
    fn duplicate_enabled_raw_special_is_rejected() {
        let block = || PromptBlock::Raw {
            name: "main".to_owned(),
            enabled: true,
            role: PromptRole::System,
            special: Some(RawPromptSpecial::Main),
            prompt: "system".to_owned(),
        };
        assert!(
            compile_prompt(
                &preset(vec![block(), block(), raw(PromptRole::User, "hello")]),
                &PromptCompileInput::default()
            )
            .is_err()
        );
    }

    #[test]
    fn preamble_and_runtime_variables_render_once() {
        let mut preset = preset(vec![raw(PromptRole::User, "${name}")]);
        preset.advanced.template = PromptTemplateSettings {
            preamble: None,
            epilogue: None,
            default_variables: BTreeMap::from([("name".to_owned(), "default".to_owned())]),
        };
        let compiled = compile_prompt(
            &preset,
            &PromptCompileInput {
                variables: BTreeMap::from([("name".to_owned(), "runtime".to_owned())]),
                ..PromptCompileInput::default()
            },
        )
        .unwrap();
        assert_eq!(compiled.messages[0].content, "runtime");
    }

    #[test]
    fn runtime_final_insertion_is_plain_data() {
        let preset = preset(vec![PromptBlock::FinalInsertion {
            name: "final".to_owned(),
            enabled: true,
            role: PromptRole::User,
            prompt: "fallback ${name}".to_owned(),
        }]);
        let compiled = compile_prompt(
            &preset,
            &PromptCompileInput {
                final_insertion: "runtime ${name}".to_owned(),
                variables: BTreeMap::from([("name".to_owned(), "expanded".to_owned())]),
                ..PromptCompileInput::default()
            },
        )
        .unwrap();
        assert_eq!(compiled.messages[0].content, "runtime ${name}");
    }

    #[test]
    fn cache_depth_resolves_against_final_messages() {
        let preset = preset(vec![
            PromptBlock::CachePoint {
                name: "cache".to_owned(),
                enabled: true,
                depth: 1,
                role: PromptRole::User,
            },
            raw(PromptRole::User, "final"),
        ]);
        let compiled = compile_prompt(&preset, &PromptCompileInput::default()).unwrap();
        assert_eq!(compiled.cache_points[0].message_index, 0);
    }

    #[test]
    fn duplicate_cache_target_is_rejected() {
        let cache = || PromptBlock::CachePoint {
            name: "cache".to_owned(),
            enabled: true,
            depth: 1,
            role: PromptRole::User,
        };
        let preset = preset(vec![
            raw(PromptRole::User, "final"),
            cache(),
            PromptBlock::CachePoint {
                name: "cache-two".to_owned(),
                enabled: true,
                depth: 1,
                role: PromptRole::User,
            },
        ]);
        assert!(compile_prompt(&preset, &PromptCompileInput::default()).is_err());
    }

    #[test]
    fn no_compiled_messages_is_rejected() {
        let preset = preset(vec![PromptBlock::Persona {
            name: "persona".to_owned(),
            enabled: true,
            role: PromptRole::System,
            format: ContentFormat::Plain,
        }]);
        assert!(compile_prompt(&preset, &PromptCompileInput::default()).is_err());
    }

    #[tokio::test]
    async fn inactive_prompt_binding_preserves_model_sampling_baseline() {
        let mut preset = preset(vec![raw(PromptRole::User, "hello")]);
        preset.sampling.temperature = Some(0.9);
        let compiled = compile_prompt(
            &preset,
            &PromptCompileInput {
                binding_mode: PromptBindingMode::ModelPreset {
                    use_prompt_parameters: false,
                },
                ..PromptCompileInput::default()
            },
        )
        .unwrap();
        let model = ModelPresetSnapshot {
            provider: ProviderId::OpenAi,
            model_id: "gpt-test".to_owned(),
            capacity: ModelCapacity {
                max_context_tokens: 8_192,
                max_output_tokens: 512,
            },
            provider_options: ProviderOptions::OpenAi(OpenAiOptions::default()),
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
            generation: GenerationOptions {
                temperature: Some(0.2),
                max_output_tokens: Some(1),
                ..GenerationOptions::default()
            },
        };
        let request = compile_prompt_request(&compiled, &model, &COUNT_ONE_HUNDRED_TOKENS)
            .await
            .unwrap();
        assert_eq!(request.provider_request().generation.temperature, Some(0.2));
        assert_eq!(
            request.provider_request().generation.max_output_tokens,
            Some(512)
        );
    }

    #[tokio::test]
    async fn active_prompt_binding_overlays_only_present_sampling_fields() {
        let mut preset = preset(vec![raw(PromptRole::User, "hello")]);
        preset.sampling.temperature = Some(0.9);
        let compiled = compile_prompt(&preset, &PromptCompileInput::default()).unwrap();
        let model = ModelPresetSnapshot {
            provider: ProviderId::OpenAi,
            model_id: "gpt-test".to_owned(),
            capacity: ModelCapacity {
                max_context_tokens: 8_192,
                max_output_tokens: 512,
            },
            provider_options: ProviderOptions::OpenAi(OpenAiOptions::default()),
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
            generation: GenerationOptions {
                temperature: Some(0.2),
                top_p: Some(0.5),
                ..GenerationOptions::default()
            },
        };
        let request = compile_prompt_request(&compiled, &model, &COUNT_ONE_HUNDRED_TOKENS)
            .await
            .unwrap();
        assert_eq!(request.provider_request().generation.temperature, Some(0.9));
        assert_eq!(request.provider_request().generation.top_p, Some(0.5));
    }

    #[test]
    fn merged_runtime_and_default_variables_share_one_limit() {
        let mut preset = preset(vec![raw(PromptRole::User, "hello")]);
        preset.advanced.template.default_variables = (0..MAX_VARIABLES)
            .map(|index| (format!("v{index}"), String::new()))
            .collect();
        let error = compile_prompt(
            &preset,
            &PromptCompileInput {
                variables: BTreeMap::from([("extra".to_owned(), "value".to_owned())]),
                ..PromptCompileInput::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("merged variables"));
    }

    #[test]
    fn persona_runtime_input_has_a_feature_specific_limit() {
        let preset = preset(vec![PromptBlock::Persona {
            name: "persona".to_owned(),
            enabled: true,
            role: PromptRole::System,
            format: ContentFormat::Plain,
        }]);
        let error = compile_prompt(
            &preset,
            &PromptCompileInput {
                persona: "a".repeat(MAX_PERSONA_INPUT_BYTES + 1),
                ..PromptCompileInput::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("persona"));
    }

    #[test]
    fn long_term_memory_runtime_input_is_bounded_and_nul_free() {
        let preset = preset(vec![PromptBlock::LongTermMemory {
            name: "memory".to_owned(),
            enabled: true,
            role: PromptRole::System,
            format: ContentFormat::Plain,
        }]);
        for memory in [
            "a".repeat(MAX_LONG_TERM_MEMORY_INPUT_BYTES + 1),
            "memory\0tail".to_owned(),
        ] {
            let error = compile_prompt(
                &preset,
                &PromptCompileInput {
                    long_term_memory: memory,
                    ..PromptCompileInput::default()
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("longTermMemory"));
        }
    }

    #[test]
    fn cache_depth_above_sixteen_is_rejected() {
        let mut blocks = (0..17)
            .map(|index| raw(PromptRole::User, &format!("message-{index}")))
            .collect::<Vec<_>>();
        blocks.push(PromptBlock::CachePoint {
            name: "cache".to_owned(),
            enabled: true,
            depth: 17,
            role: PromptRole::User,
        });
        assert!(compile_prompt(&preset(blocks), &PromptCompileInput::default()).is_err());
    }

    #[test]
    fn request_transforms_share_the_final_message_byte_limit() {
        let mut preset = preset(vec![
            PromptBlock::Lorebook {
                name: "lorebook".to_owned(),
                enabled: true,
                role: PromptRole::User,
                format: ContentFormat::Plain,
            },
            PromptBlock::CharacterDescription {
                name: "character".to_owned(),
                enabled: true,
                role: PromptRole::Assistant,
                format: ContentFormat::Plain,
            },
        ]);
        preset.advanced.regex.push(RegexRule {
            name: "double".to_owned(),
            enabled: true,
            target: TransformTarget::Request,
            pattern: "(?s).+".to_owned(),
            replacement: "${0}${0}".to_owned(),
            flags: RegexFlags::default(),
        });
        let large = "a".repeat(1_100_000);
        let error = compile_prompt(
            &preset,
            &PromptCompileInput {
                lorebook: large.clone(),
                character_description: large,
                ..PromptCompileInput::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("compiled messages"));
    }

    #[tokio::test]
    async fn exact_model_tokenizer_count_seals_context_capacity() {
        let compiled = compile_prompt(
            &preset(vec![raw(PromptRole::User, "한국어 😀")]),
            &PromptCompileInput::default(),
        )
        .unwrap();
        assert!(compiled.token_count.approximate < 6);
        assert_eq!(compiled.token_count.exact, None);

        let model = ModelPresetSnapshot {
            provider: ProviderId::OpenAi,
            model_id: "gpt-test".to_owned(),
            capacity: ModelCapacity {
                max_context_tokens: 10,
                max_output_tokens: 4,
            },
            provider_options: ProviderOptions::OpenAi(OpenAiOptions::default()),
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
            generation: GenerationOptions::default(),
        };

        let exact_boundary = FixedTokenCounter(6);
        let request = compile_prompt_request(&compiled, &model, &exact_boundary)
            .await
            .unwrap();
        assert_eq!(request.input_tokens(), 6);

        let over_capacity = FixedTokenCounter(7);
        let error = compile_prompt_request(&compiled, &model, &over_capacity)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exact input (7)"));
    }

    #[tokio::test]
    async fn exact_counter_and_sealed_request_share_one_compiled_wire_snapshot() {
        let compiled = compile_prompt(
            &preset(vec![raw(PromptRole::User, "hello")]),
            &PromptCompileInput::default(),
        )
        .unwrap();
        let model = ModelPresetSnapshot {
            provider: ProviderId::OpenAi,
            model_id: "gpt-test".to_owned(),
            capacity: ModelCapacity {
                max_context_tokens: 1_024,
                max_output_tokens: 128,
            },
            provider_options: ProviderOptions::OpenAi(OpenAiOptions::default()),
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
            generation: GenerationOptions::default(),
        };

        let sealed = compile_prompt_request(&compiled, &model, &InspectingTokenCounter)
            .await
            .unwrap();
        assert_eq!(
            sealed.compiled_provider_request(),
            &compile_request(sealed.provider_request()).unwrap()
        );
        assert_eq!(sealed.input_tokens(), 100);
    }

    #[tokio::test]
    async fn provider_adapter_denies_unmapped_cache_tool_and_module_metadata() {
        let base = compile_prompt(
            &preset(vec![raw(PromptRole::User, "hello")]),
            &PromptCompileInput::default(),
        )
        .unwrap();
        let model = ModelPresetSnapshot {
            provider: ProviderId::OpenAi,
            model_id: "gpt-test".to_owned(),
            capacity: ModelCapacity {
                max_context_tokens: 1_024,
                max_output_tokens: 128,
            },
            provider_options: ProviderOptions::OpenAi(OpenAiOptions::default()),
            tokenizer_override: None,
            additional_parameters: BTreeMap::new(),
            generation: GenerationOptions::default(),
        };

        let mut with_cache = base.clone();
        with_cache.cache_points.push(CachePoint {
            block_index: 0,
            message_index: 0,
            depth: 1,
            role: PromptRole::User,
        });
        assert!(matches!(
            compile_prompt_request(&with_cache, &model, &COUNT_ONE_HUNDRED_TOKENS).await,
            Err(PromptError::UnsupportedFeature { feature, .. }) if feature == "cache_points"
        ));

        let mut with_tool = base.clone();
        with_tool.active_tools.push(ToolReference {
            tool_id: "web.search".to_owned(),
            enabled: true,
        });
        assert!(matches!(
            compile_prompt_request(&with_tool, &model, &COUNT_ONE_HUNDRED_TOKENS).await,
            Err(PromptError::UnsupportedFeature { feature, .. }) if feature == "tools"
        ));

        let mut with_module = base;
        with_module.active_modules.push(ModuleReference {
            module_id: "story.module".to_owned(),
            version: "1.0.0".to_owned(),
            digest_sha256: "a".repeat(64),
            enabled: true,
        });
        assert!(matches!(
            compile_prompt_request(&with_module, &model, &COUNT_ONE_HUNDRED_TOKENS).await,
            Err(PromptError::UnsupportedFeature { feature, .. }) if feature == "modules"
        ));
    }
}
