use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use regex::{Regex, RegexBuilder};
use sha2::{Digest, Sha256};

use crate::{
    Activation, EntryId, LoreEntry, LorebookCatalog, LorebookError, MAX_ACTIVE_REGEX_CONDITIONS,
    MAX_ACTIVE_REGEX_PATTERN_BYTES, MatchCondition, MatchConditions, MessageState,
    PartialMessagePolicy, Result, SecondaryConditions, SelectionRequest, SummarySnapshot,
    normalize_search_text,
};

const REGEX_SIZE_LIMIT: usize = 256 * 1024;
const REGEX_DFA_SIZE_LIMIT: usize = 256 * 1024;
const MAX_CACHE_ENTRIES: usize = 16;
const RANDOM_DOMAIN: &[u8] = b"lorepia-lorebook-probability-v1";
const CACHE_DOMAIN: &[u8] = b"lorepia-lorebook-cache-v1";

#[derive(Clone)]
pub struct Selection {
    prompt_text: Arc<str>,
    receipt: SelectionReceipt,
}

impl Selection {
    /// Opaque lore text for `PromptCompileInput::lorebook`. It is never
    /// template-expanded or reparsed by this crate.
    #[must_use]
    pub fn prompt_text(&self) -> &str {
        &self.prompt_text
    }

    #[must_use]
    pub const fn receipt(&self) -> &SelectionReceipt {
        &self.receipt
    }
}

impl fmt::Debug for Selection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Selection")
            .field("prompt_bytes", &self.prompt_text.len())
            .field("receipt", &self.receipt)
            .finish()
    }
}

#[derive(Clone)]
pub struct SelectionReceipt {
    catalog_revision: u64,
    chat_revision: u64,
    branch_revision: u64,
    cache_fingerprint: [u8; 32],
    cache_hit: bool,
    candidate_count: usize,
    matched_count: usize,
    selected_ids: Vec<EntryId>,
    selected_tokens: u32,
    selected_bytes: usize,
    literal_match_events: usize,
    regex_evaluations: usize,
    regex_scan_bytes: usize,
    regex_matches: usize,
}

impl SelectionReceipt {
    #[must_use]
    pub const fn cache_hit(&self) -> bool {
        self.cache_hit
    }

    #[must_use]
    pub const fn cache_fingerprint(&self) -> &[u8; 32] {
        &self.cache_fingerprint
    }

    #[must_use]
    pub const fn candidate_count(&self) -> usize {
        self.candidate_count
    }

    #[must_use]
    pub const fn matched_count(&self) -> usize {
        self.matched_count
    }

    #[must_use]
    pub fn selected_ids(&self) -> &[EntryId] {
        &self.selected_ids
    }

    #[must_use]
    pub const fn selected_tokens(&self) -> u32 {
        self.selected_tokens
    }

    #[must_use]
    pub const fn selected_bytes(&self) -> usize {
        self.selected_bytes
    }

    #[must_use]
    pub const fn literal_match_events(&self) -> usize {
        self.literal_match_events
    }

    #[must_use]
    pub const fn regex_evaluations(&self) -> usize {
        self.regex_evaluations
    }

    #[must_use]
    pub const fn regex_scan_bytes(&self) -> usize {
        self.regex_scan_bytes
    }

    #[must_use]
    pub const fn regex_matches(&self) -> usize {
        self.regex_matches
    }
}

impl fmt::Debug for SelectionReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectionReceipt")
            .field("catalog_revision", &self.catalog_revision)
            .field("chat_revision", &self.chat_revision)
            .field("branch_revision", &self.branch_revision)
            .field("cache_fingerprint", &hex_prefix(&self.cache_fingerprint))
            .field("cache_hit", &self.cache_hit)
            .field("candidate_count", &self.candidate_count)
            .field("matched_count", &self.matched_count)
            .field("selected_count", &self.selected_ids.len())
            .field("selected_tokens", &self.selected_tokens)
            .field("selected_bytes", &self.selected_bytes)
            .field("literal_match_events", &self.literal_match_events)
            .field("regex_evaluations", &self.regex_evaluations)
            .field("regex_scan_bytes", &self.regex_scan_bytes)
            .field("regex_matches", &self.regex_matches)
            .finish()
    }
}

fn hex_prefix(value: &[u8; 32]) -> String {
    value[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct CacheKey {
    catalog_revision: u64,
    catalog_generation: u64,
    chat_revision: u64,
    branch_revision: u64,
    search_depth: u64,
    seed: u64,
    tokenizer_fingerprint: [u8; 32],
    settings_fingerprint: [u8; 32],
    search_slice_fingerprint: [u8; 32],
}

impl CacheKey {
    fn fingerprint(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(CACHE_DOMAIN);
        digest_u64(&mut digest, self.catalog_revision);
        digest_u64(&mut digest, self.catalog_generation);
        digest_u64(&mut digest, self.chat_revision);
        digest_u64(&mut digest, self.branch_revision);
        digest_u64(&mut digest, self.search_depth);
        digest_u64(&mut digest, self.seed);
        digest.update(self.tokenizer_fingerprint);
        digest.update(self.settings_fingerprint);
        digest.update(self.search_slice_fingerprint);
        digest.finalize().into()
    }
}

struct CompiledCatalog {
    revision: u64,
    generation: u64,
    entries: Vec<CompiledEntry>,
    sensitive: PatternIndex,
    folded: PatternIndex,
    unconditional: Vec<usize>,
}

struct CompiledEntry {
    source: LoreEntry,
    activation: CompiledActivation,
}

enum CompiledActivation {
    Constant,
    Selective(CompiledConditions),
    Probability {
        basis_points: u16,
        conditions: Option<CompiledConditions>,
    },
}

struct CompiledConditions {
    primary: Vec<CompiledCondition>,
    secondary: CompiledSecondary,
}

enum CompiledSecondary {
    None,
    Any(Vec<CompiledCondition>),
    All(Vec<CompiledCondition>),
}

enum CompiledCondition {
    Literal(PatternRef),
    Regex { regex: Regex },
}

#[derive(Clone, Copy)]
struct PatternRef {
    folded: bool,
    index: usize,
}

struct PatternIndex {
    matcher: Option<AhoCorasick>,
    patterns: Vec<String>,
    candidate_entries: Vec<Vec<usize>>,
}

impl PatternIndex {
    fn build(patterns: BTreeSet<String>) -> Result<Self> {
        let patterns: Vec<_> = patterns.into_iter().collect();
        let matcher = if patterns.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .match_kind(MatchKind::Standard)
                    .build(&patterns)
                    .map_err(|_| {
                        LorebookError::invalid("catalog.conditions", "cannot build search index")
                    })?,
            )
        };
        let candidate_entries = vec![Vec::new(); patterns.len()];
        Ok(Self {
            matcher,
            patterns,
            candidate_entries,
        })
    }

    fn lookup(&self, pattern: &str) -> Result<usize> {
        self.patterns
            .binary_search_by(|item| item.as_str().cmp(pattern))
            .map_err(|_| {
                LorebookError::invalid("catalog.conditions", "search index is inconsistent")
            })
    }

    fn matches(&self, haystack: &str, work: &mut LiteralWork) -> Result<Vec<bool>> {
        let mut hits = vec![false; self.patterns.len()];
        if let Some(matcher) = &self.matcher {
            for found in matcher.find_overlapping_iter(haystack) {
                work.charge()?;
                hits[found.pattern().as_usize()] = true;
            }
        }
        Ok(hits)
    }
}

struct LiteralWork {
    events: usize,
    max_events: usize,
}

impl LiteralWork {
    const fn new(max_events: usize) -> Self {
        Self {
            events: 0,
            max_events,
        }
    }

    fn charge(&mut self) -> Result<()> {
        self.events = self
            .events
            .checked_add(1)
            .ok_or(LorebookError::SearchLimitExceeded {
                limit: crate::LimitKind::LiteralMatchEvents,
            })?;
        if self.events > self.max_events {
            return Err(LorebookError::SearchLimitExceeded {
                limit: crate::LimitKind::LiteralMatchEvents,
            });
        }
        Ok(())
    }
}

pub struct LorebookEngine {
    compiled: CompiledCatalog,
    cache: Mutex<BTreeMap<CacheKey, Selection>>,
}

impl fmt::Debug for LorebookEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LorebookEngine")
            .field("catalog_revision", &self.compiled.revision)
            .field("catalog_generation", &self.compiled.generation)
            .field("entry_count", &self.compiled.entries.len())
            .finish()
    }
}

impl LorebookEngine {
    pub fn new(catalog: LorebookCatalog) -> Result<Self> {
        Ok(Self {
            compiled: compile_catalog(catalog, 1)?,
            cache: Mutex::new(BTreeMap::new()),
        })
    }

    /// Rebuilds immutable candidate indexes and clears every cached selection.
    /// This is the only mutation path; callers cannot retain a stale index.
    pub fn replace_catalog(&mut self, catalog: LorebookCatalog) -> Result<()> {
        let generation = self.compiled.generation.checked_add(1).ok_or_else(|| {
            LorebookError::invalid("catalog.generation", "revision counter overflowed")
        })?;
        let compiled = compile_catalog(catalog, generation)?;
        self.compiled = compiled;
        mutex_lock(&self.cache).clear();
        Ok(())
    }

    /// Performs CPU work synchronously. Product adapters must call this from a
    /// bounded worker (`spawn_blocking` on Tauri), never from a WebView/UI loop.
    pub fn select(&self, request: &SelectionRequest) -> Result<Selection> {
        request.validate()?;
        let search = SearchInput::build(request);
        let key = build_cache_key(&self.compiled, request, &search);

        if let Some(cached) = mutex_lock(&self.cache).get(&key) {
            let mut cached = cached.clone();
            cached.receipt.cache_hit = true;
            return Ok(cached);
        }

        let selection = self.select_uncached(request, &search, key.fingerprint())?;
        let mut cache = mutex_lock(&self.cache);
        if cache.len() >= MAX_CACHE_ENTRIES
            && let Some(oldest) = cache.keys().next().cloned()
        {
            cache.remove(&oldest);
        }
        cache.insert(key, selection.clone());
        Ok(selection)
    }

    fn select_uncached(
        &self,
        request: &SelectionRequest,
        search: &SearchInput,
        cache_fingerprint: [u8; 32],
    ) -> Result<Selection> {
        let mut literal_work = LiteralWork::new(request.settings.max_literal_match_events);
        let sensitive_hits = self
            .compiled
            .sensitive
            .matches(&search.sensitive, &mut literal_work)?;
        let folded_hits = self
            .compiled
            .folded
            .matches(&search.folded, &mut literal_work)?;
        let mut seen = vec![false; self.compiled.entries.len()];
        let mut candidates = Vec::new();

        for &entry_index in &self.compiled.unconditional {
            add_candidate(entry_index, &mut seen, &mut candidates);
        }
        add_pattern_candidates(
            &self.compiled.sensitive,
            &sensitive_hits,
            &mut seen,
            &mut candidates,
        );
        add_pattern_candidates(
            &self.compiled.folded,
            &folded_hits,
            &mut seen,
            &mut candidates,
        );

        let hits = PatternHits {
            sensitive: &sensitive_hits,
            folded: &folded_hits,
        };
        let mut limits = EvaluationLimits::new(request);
        let mut matched = Vec::new();
        for entry_index in candidates.iter().copied() {
            let entry = &self.compiled.entries[entry_index];
            if entry.activation.matches(
                &entry.source,
                &hits,
                &search.regex,
                request.seed,
                &mut limits,
            )? {
                matched.push(entry_index);
            }
        }

        matched.sort_unstable_by(|left, right| {
            compare_entries(
                &self.compiled.entries[*left].source,
                &self.compiled.entries[*right].source,
            )
        });

        let mut prompt_text = String::new();
        let mut selected_ids = Vec::new();
        let mut selected_tokens = 0u32;
        for entry_index in matched.iter().copied() {
            let entry = &self.compiled.entries[entry_index].source;
            let separator_bytes = if selected_ids.is_empty() {
                0
            } else {
                request.settings.separator.len()
            };
            let next_bytes = prompt_text
                .len()
                .checked_add(separator_bytes)
                .and_then(|value| value.checked_add(entry.content.len()));
            let next_tokens = selected_tokens.checked_add(entry.reserved_tokens);
            let (Some(next_bytes), Some(next_tokens)) = (next_bytes, next_tokens) else {
                continue;
            };
            if next_bytes > request.settings.max_output_bytes
                || next_tokens > request.settings.max_output_tokens
            {
                continue;
            }
            if !selected_ids.is_empty() {
                prompt_text.push_str(&request.settings.separator);
            }
            prompt_text.push_str(&entry.content);
            selected_ids.push(entry.id.clone());
            selected_tokens = next_tokens;
        }

        let selected_bytes = prompt_text.len();
        Ok(Selection {
            prompt_text: Arc::from(prompt_text),
            receipt: SelectionReceipt {
                catalog_revision: self.compiled.revision,
                chat_revision: request.conversation.chat_revision,
                branch_revision: request.conversation.branch_revision,
                cache_fingerprint,
                cache_hit: false,
                candidate_count: candidates.len(),
                matched_count: matched.len(),
                selected_ids,
                selected_tokens,
                selected_bytes,
                literal_match_events: literal_work.events,
                regex_evaluations: limits.evaluations,
                regex_scan_bytes: limits.scan_bytes,
                regex_matches: limits.matches,
            },
        })
    }
}

fn mutex_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn compare_entries(left: &LoreEntry, right: &LoreEntry) -> Ordering {
    right
        .priority
        .cmp(&left.priority)
        .then_with(|| right.source.precedence().cmp(&left.source.precedence()))
        .then_with(|| left.order.cmp(&right.order))
        .then_with(|| left.id.cmp(&right.id))
}

fn add_candidate(entry: usize, seen: &mut [bool], candidates: &mut Vec<usize>) {
    if !seen[entry] {
        seen[entry] = true;
        candidates.push(entry);
    }
}

fn add_pattern_candidates(
    index: &PatternIndex,
    hits: &[bool],
    seen: &mut [bool],
    candidates: &mut Vec<usize>,
) {
    for (pattern_index, hit) in hits.iter().copied().enumerate() {
        if hit {
            for &entry in &index.candidate_entries[pattern_index] {
                add_candidate(entry, seen, candidates);
            }
        }
    }
}

struct SearchInput {
    regex: String,
    sensitive: String,
    folded: String,
    fingerprint: [u8; 32],
}

impl SearchInput {
    fn build(request: &SelectionRequest) -> Self {
        let mut raw = String::new();
        let mut digest = Sha256::new();
        digest.update(b"lorepia-lorebook-search-slice-v1");

        match &request.conversation.summary {
            SummarySnapshot::Missing => digest.update([0]),
            SummarySnapshot::Corrupt { revision } => {
                digest.update([1]);
                digest_u64(&mut digest, *revision);
            }
            SummarySnapshot::Available { revision, content } => {
                digest.update([2]);
                digest_u64(&mut digest, *revision);
                digest_text(&mut digest, content);
                append_segment(&mut raw, content);
            }
        }

        let start = request
            .conversation
            .recent_turns
            .len()
            .saturating_sub(request.settings.search_depth);
        for turn in &request.conversation.recent_turns[start..] {
            digest_u64(&mut digest, turn.revision);
            digest.update([match turn.state {
                MessageState::Complete => 0,
                MessageState::Partial => 1,
                MessageState::Failed => 2,
            }]);
            let include = match turn.state {
                MessageState::Complete => true,
                MessageState::Partial => {
                    request.settings.partial_messages == PartialMessagePolicy::Include
                }
                MessageState::Failed => false,
            };
            if include {
                digest_text(&mut digest, &turn.content);
                append_segment(&mut raw, &turn.content);
            }
        }

        Self {
            regex: raw.clone(),
            sensitive: normalize_search_text(&raw, true),
            folded: normalize_search_text(&raw, false),
            fingerprint: digest.finalize().into(),
        }
    }
}

fn append_segment(target: &mut String, segment: &str) {
    if !target.is_empty() {
        target.push('\n');
    }
    target.push_str(segment);
}

fn digest_text(digest: &mut Sha256, value: &str) {
    digest_u64(
        digest,
        u64::try_from(value.len()).expect("bounded strings fit in u64"),
    );
    digest.update(value.as_bytes());
}

fn digest_u64(digest: &mut Sha256, value: u64) {
    digest.update(value.to_le_bytes());
}

fn build_cache_key(
    catalog: &CompiledCatalog,
    request: &SelectionRequest,
    search: &SearchInput,
) -> CacheKey {
    let mut tokenizer = Sha256::new();
    digest_text(&mut tokenizer, &request.settings.tokenizer_id);
    digest_text(&mut tokenizer, &request.settings.tokenizer_revision);

    let mut settings = Sha256::new();
    digest_u64(
        &mut settings,
        u64::try_from(request.settings.search_depth).expect("bounded search depth fits in u64"),
    );
    settings.update([match request.settings.partial_messages {
        PartialMessagePolicy::Exclude => 0,
        PartialMessagePolicy::Include => 1,
    }]);
    digest_u64(&mut settings, u64::from(request.settings.max_output_tokens));
    digest_u64(
        &mut settings,
        u64::try_from(request.settings.max_output_bytes).expect("bounded output fits in u64"),
    );
    digest_u64(
        &mut settings,
        u64::try_from(request.settings.max_literal_match_events)
            .expect("bounded literal work fits in u64"),
    );
    digest_u64(
        &mut settings,
        u64::try_from(request.settings.max_regex_evaluations)
            .expect("bounded regex evaluations fit in u64"),
    );
    digest_u64(
        &mut settings,
        u64::try_from(request.settings.max_regex_scan_bytes)
            .expect("bounded regex scan fits in u64"),
    );
    digest_u64(
        &mut settings,
        u64::try_from(request.settings.max_regex_matches)
            .expect("bounded regex matches fit in u64"),
    );
    digest_text(&mut settings, &request.settings.separator);

    CacheKey {
        catalog_revision: catalog.revision,
        catalog_generation: catalog.generation,
        chat_revision: request.conversation.chat_revision,
        branch_revision: request.conversation.branch_revision,
        search_depth: u64::try_from(request.settings.search_depth)
            .expect("bounded search depth fits in u64"),
        seed: request.seed,
        tokenizer_fingerprint: tokenizer.finalize().into(),
        settings_fingerprint: settings.finalize().into(),
        search_slice_fingerprint: search.fingerprint,
    }
}

struct PatternHits<'a> {
    sensitive: &'a [bool],
    folded: &'a [bool],
}

impl PatternHits<'_> {
    fn contains(&self, reference: PatternRef) -> bool {
        if reference.folded {
            self.folded[reference.index]
        } else {
            self.sensitive[reference.index]
        }
    }
}

struct EvaluationLimits {
    max_evaluations: usize,
    max_scan_bytes: usize,
    max_matches: usize,
    evaluations: usize,
    scan_bytes: usize,
    matches: usize,
}

impl EvaluationLimits {
    fn new(request: &SelectionRequest) -> Self {
        Self {
            max_evaluations: request.settings.max_regex_evaluations,
            max_scan_bytes: request.settings.max_regex_scan_bytes,
            max_matches: request.settings.max_regex_matches,
            evaluations: 0,
            scan_bytes: 0,
            matches: 0,
        }
    }

    fn evaluate(&mut self, regex: &Regex, input: &str) -> Result<bool> {
        self.evaluations =
            self.evaluations
                .checked_add(1)
                .ok_or(LorebookError::SearchLimitExceeded {
                    limit: crate::LimitKind::RegexEvaluations,
                })?;
        if self.evaluations > self.max_evaluations {
            return Err(LorebookError::SearchLimitExceeded {
                limit: crate::LimitKind::RegexEvaluations,
            });
        }
        self.scan_bytes =
            self.scan_bytes
                .checked_add(input.len())
                .ok_or(LorebookError::SearchLimitExceeded {
                    limit: crate::LimitKind::RegexScanBytes,
                })?;
        if self.scan_bytes > self.max_scan_bytes {
            return Err(LorebookError::SearchLimitExceeded {
                limit: crate::LimitKind::RegexScanBytes,
            });
        }
        let mut matched = false;
        for _ in regex.find_iter(input) {
            matched = true;
            self.matches =
                self.matches
                    .checked_add(1)
                    .ok_or(LorebookError::SearchLimitExceeded {
                        limit: crate::LimitKind::RegexMatches,
                    })?;
            if self.matches > self.max_matches {
                return Err(LorebookError::SearchLimitExceeded {
                    limit: crate::LimitKind::RegexMatches,
                });
            }
        }
        Ok(matched)
    }
}

impl CompiledActivation {
    fn matches(
        &self,
        entry: &LoreEntry,
        hits: &PatternHits<'_>,
        regex_input: &str,
        seed: u64,
        limits: &mut EvaluationLimits,
    ) -> Result<bool> {
        match self {
            Self::Constant => Ok(true),
            Self::Selective(conditions) => conditions.matches(hits, regex_input, limits),
            Self::Probability {
                basis_points,
                conditions,
            } => {
                if let Some(conditions) = conditions
                    && !conditions.matches(hits, regex_input, limits)?
                {
                    return Ok(false);
                }
                Ok(probability_admits(seed, entry.id.as_str(), *basis_points))
            }
        }
    }
}

impl CompiledConditions {
    fn matches(
        &self,
        hits: &PatternHits<'_>,
        regex_input: &str,
        limits: &mut EvaluationLimits,
    ) -> Result<bool> {
        if !any_matches(&self.primary, hits, regex_input, limits)? {
            return Ok(false);
        }
        match &self.secondary {
            CompiledSecondary::None => Ok(true),
            CompiledSecondary::Any(conditions) => {
                any_matches(conditions, hits, regex_input, limits)
            }
            CompiledSecondary::All(conditions) => {
                for condition in conditions {
                    if !condition.matches(hits, regex_input, limits)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
}

fn any_matches(
    conditions: &[CompiledCondition],
    hits: &PatternHits<'_>,
    regex_input: &str,
    limits: &mut EvaluationLimits,
) -> Result<bool> {
    for condition in conditions {
        if condition.matches(hits, regex_input, limits)? {
            return Ok(true);
        }
    }
    Ok(false)
}

impl CompiledCondition {
    fn matches(
        &self,
        hits: &PatternHits<'_>,
        regex_input: &str,
        limits: &mut EvaluationLimits,
    ) -> Result<bool> {
        match self {
            Self::Literal(reference) => Ok(hits.contains(*reference)),
            Self::Regex { regex } => limits.evaluate(regex, regex_input),
        }
    }
}

fn probability_admits(seed: u64, entry_id: &str, basis_points: u16) -> bool {
    if basis_points == 0 {
        return false;
    }
    if basis_points == 10_000 {
        return true;
    }
    let mut digest = Sha256::new();
    digest.update(RANDOM_DOMAIN);
    digest_u64(&mut digest, seed);
    digest_text(&mut digest, entry_id);
    let bytes: [u8; 32] = digest.finalize().into();
    let value = u64::from_le_bytes(bytes[..8].try_into().expect("fixed digest prefix"));
    let bucket = ((u128::from(value) * 10_000) >> 64) as u16;
    bucket < basis_points
}

fn compile_catalog(catalog: LorebookCatalog, generation: u64) -> Result<CompiledCatalog> {
    catalog.validate()?;
    let mut sensitive_patterns = BTreeSet::new();
    let mut folded_patterns = BTreeSet::new();
    let mut active_regex_conditions = 0usize;
    let mut active_regex_pattern_bytes = 0usize;
    for entry in catalog.entries().iter().filter(|entry| entry.enabled) {
        if let Some(conditions) = activation_conditions(&entry.activation) {
            for condition in condition_iter(conditions) {
                match condition {
                    MatchCondition::Literal {
                        value,
                        case_sensitive,
                    } => {
                        let normalized = normalize_search_text(value, *case_sensitive);
                        if normalized.is_empty() {
                            return Err(LorebookError::invalid(
                                "entry.condition.key",
                                "normalization cannot produce an empty key",
                            ));
                        }
                        if *case_sensitive {
                            sensitive_patterns.insert(normalized);
                        } else {
                            folded_patterns.insert(normalized);
                        }
                    }
                    MatchCondition::Regex { pattern, .. } => {
                        active_regex_conditions =
                            active_regex_conditions.checked_add(1).ok_or_else(|| {
                                LorebookError::too_many(
                                    "activeRegexConditions",
                                    MAX_ACTIVE_REGEX_CONDITIONS,
                                )
                            })?;
                        active_regex_pattern_bytes = active_regex_pattern_bytes
                            .checked_add(pattern.len())
                            .ok_or_else(|| {
                                LorebookError::too_large(
                                    "activeRegexPatterns",
                                    MAX_ACTIVE_REGEX_PATTERN_BYTES,
                                )
                            })?;
                    }
                }
            }
        }
    }
    if active_regex_conditions > MAX_ACTIVE_REGEX_CONDITIONS {
        return Err(LorebookError::too_many(
            "activeRegexConditions",
            MAX_ACTIVE_REGEX_CONDITIONS,
        ));
    }
    if active_regex_pattern_bytes > MAX_ACTIVE_REGEX_PATTERN_BYTES {
        return Err(LorebookError::too_large(
            "activeRegexPatterns",
            MAX_ACTIVE_REGEX_PATTERN_BYTES,
        ));
    }

    let mut sensitive = PatternIndex::build(sensitive_patterns)?;
    let mut folded = PatternIndex::build(folded_patterns)?;
    let mut entries = Vec::new();
    let mut unconditional = Vec::new();

    for entry in catalog.entries().iter().filter(|entry| entry.enabled) {
        let entry_index = entries.len();
        let activation = compile_activation(entry_index, &entry.activation, &sensitive, &folded)?;
        if matches!(
            activation,
            CompiledActivation::Constant
                | CompiledActivation::Probability {
                    conditions: None,
                    ..
                }
        ) || compiled_activation_contains_regex(&activation)
        {
            unconditional.push(entry_index);
        }
        register_activation_candidates(entry_index, &activation, &mut sensitive, &mut folded);
        entries.push(CompiledEntry {
            source: entry.clone(),
            activation,
        });
    }

    Ok(CompiledCatalog {
        revision: catalog.revision(),
        generation,
        entries,
        sensitive,
        folded,
        unconditional,
    })
}

fn activation_conditions(activation: &Activation) -> Option<&MatchConditions> {
    match activation {
        Activation::Constant => None,
        Activation::Selective { conditions } => Some(conditions),
        Activation::Probability { conditions, .. } => conditions.as_ref(),
    }
}

fn condition_iter(conditions: &MatchConditions) -> impl Iterator<Item = &MatchCondition> {
    conditions
        .primary
        .iter()
        .chain(match &conditions.secondary {
            SecondaryConditions::None => &[][..],
            SecondaryConditions::Any(values) | SecondaryConditions::All(values) => values,
        })
}

fn compile_activation(
    entry_index: usize,
    activation: &Activation,
    sensitive: &PatternIndex,
    folded: &PatternIndex,
) -> Result<CompiledActivation> {
    match activation {
        Activation::Constant => Ok(CompiledActivation::Constant),
        Activation::Selective { conditions } => Ok(CompiledActivation::Selective(
            compile_conditions(entry_index, conditions, sensitive, folded)?,
        )),
        Activation::Probability {
            basis_points,
            conditions,
        } => Ok(CompiledActivation::Probability {
            basis_points: *basis_points,
            conditions: conditions
                .as_ref()
                .map(|value| compile_conditions(entry_index, value, sensitive, folded))
                .transpose()?,
        }),
    }
}

fn compile_conditions(
    entry_index: usize,
    conditions: &MatchConditions,
    sensitive: &PatternIndex,
    folded: &PatternIndex,
) -> Result<CompiledConditions> {
    let compile_many = |values: &[MatchCondition]| {
        values
            .iter()
            .map(|value| compile_condition(entry_index, value, sensitive, folded))
            .collect::<Result<Vec<_>>>()
    };
    Ok(CompiledConditions {
        primary: compile_many(&conditions.primary)?,
        secondary: match &conditions.secondary {
            SecondaryConditions::None => CompiledSecondary::None,
            SecondaryConditions::Any(values) => CompiledSecondary::Any(compile_many(values)?),
            SecondaryConditions::All(values) => CompiledSecondary::All(compile_many(values)?),
        },
    })
}

fn compile_condition(
    entry_index: usize,
    condition: &MatchCondition,
    sensitive: &PatternIndex,
    folded: &PatternIndex,
) -> Result<CompiledCondition> {
    match condition {
        MatchCondition::Literal {
            value,
            case_sensitive,
        } => {
            let normalized = normalize_search_text(value, *case_sensitive);
            let reference = PatternRef {
                folded: !case_sensitive,
                index: if *case_sensitive {
                    sensitive.lookup(&normalized)?
                } else {
                    folded.lookup(&normalized)?
                },
            };
            Ok(CompiledCondition::Literal(reference))
        }
        MatchCondition::Regex {
            pattern,
            case_insensitive,
            ..
        } => {
            let regex = RegexBuilder::new(pattern)
                .unicode(true)
                .case_insensitive(*case_insensitive)
                .size_limit(REGEX_SIZE_LIMIT)
                .dfa_size_limit(REGEX_DFA_SIZE_LIMIT)
                .build()
                .map_err(|_| LorebookError::InvalidRegex { entry_index })?;
            Ok(CompiledCondition::Regex { regex })
        }
    }
}

fn register_activation_candidates(
    entry_index: usize,
    activation: &CompiledActivation,
    sensitive: &mut PatternIndex,
    folded: &mut PatternIndex,
) {
    let conditions = match activation {
        CompiledActivation::Constant => return,
        CompiledActivation::Selective(conditions) => conditions,
        CompiledActivation::Probability {
            conditions: Some(conditions),
            ..
        } => conditions,
        CompiledActivation::Probability {
            conditions: None, ..
        } => return,
    };
    for condition in compiled_condition_iter(conditions) {
        let reference = match condition {
            CompiledCondition::Literal(reference) => *reference,
            CompiledCondition::Regex { .. } => continue,
        };
        let index = if reference.folded {
            &mut folded.candidate_entries[reference.index]
        } else {
            &mut sensitive.candidate_entries[reference.index]
        };
        if index.last() != Some(&entry_index) {
            index.push(entry_index);
        }
    }
}

fn compiled_activation_contains_regex(activation: &CompiledActivation) -> bool {
    let conditions = match activation {
        CompiledActivation::Constant => return false,
        CompiledActivation::Selective(conditions) => conditions,
        CompiledActivation::Probability {
            conditions: Some(conditions),
            ..
        } => conditions,
        CompiledActivation::Probability {
            conditions: None, ..
        } => return false,
    };
    compiled_condition_iter(conditions)
        .any(|condition| matches!(condition, CompiledCondition::Regex { .. }))
}

fn compiled_condition_iter(
    conditions: &CompiledConditions,
) -> impl Iterator<Item = &CompiledCondition> {
    conditions
        .primary
        .iter()
        .chain(match &conditions.secondary {
            CompiledSecondary::None => &[][..],
            CompiledSecondary::Any(values) | CompiledSecondary::All(values) => values,
        })
}
