use std::{collections::BTreeMap, sync::Arc};

use lorepia_lorebook::{
    Activation, ChatTurn, ConversationSnapshot, EntryId, EntrySource, ImportTrust, LimitKind,
    LoreEntry, LorebookCatalog, LorebookEngine, LorebookError, MAX_KEY_BYTES,
    MAX_LITERAL_MATCH_EVENTS, MAX_OUTPUT_BYTES, MAX_OUTPUT_TOKENS, MAX_REGEX_EVALUATIONS,
    MAX_REGEX_MATCHES, MAX_REGEX_SCAN_BYTES, MAX_TURN_BYTES, MatchCondition, MatchConditions,
    MessageState, PartialMessagePolicy, SecondaryConditions, SelectionRequest, SelectionSettings,
    SummarySnapshot, export_catalog, import_catalog, import_catalog_with_trust,
    import_trusted_engine,
};
use lorepia_prompt::{
    ContentFormat, PromptBlock, PromptCompileInput, PromptPreset, PromptRole, compile_prompt,
};

fn entry(
    id: &str,
    source: EntrySource,
    priority: i32,
    order: i32,
    activation: Activation,
    content: &str,
    tokens: u32,
) -> LoreEntry {
    LoreEntry::new(
        EntryId::parse(id).unwrap(),
        source,
        priority,
        order,
        true,
        activation,
        content,
        tokens,
    )
    .unwrap()
}

fn selective(key: impl Into<String>) -> Activation {
    Activation::Selective {
        conditions: MatchConditions::any(vec![MatchCondition::literal(key)]),
    }
}

fn request(text: &str) -> SelectionRequest {
    SelectionRequest {
        conversation: ConversationSnapshot {
            chat_revision: 1,
            branch_revision: 1,
            total_turns: 1,
            recent_turns: vec![ChatTurn {
                revision: 1,
                state: MessageState::Complete,
                content: text.to_owned(),
            }],
            summary: SummarySnapshot::Missing,
        },
        settings: SelectionSettings {
            search_depth: 5,
            partial_messages: PartialMessagePolicy::Exclude,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_output_bytes: MAX_OUTPUT_BYTES,
            max_literal_match_events: MAX_LITERAL_MATCH_EVENTS,
            max_regex_evaluations: MAX_REGEX_EVALUATIONS,
            max_regex_scan_bytes: MAX_REGEX_SCAN_BYTES,
            max_regex_matches: MAX_REGEX_MATCHES,
            tokenizer_id: "test-tokenizer".to_owned(),
            tokenizer_revision: "1".to_owned(),
            separator: "\n\n".to_owned(),
        },
        seed: 7,
    }
}

fn engine(entries: Vec<LoreEntry>) -> LorebookEngine {
    LorebookEngine::new(LorebookCatalog::new(1, entries).unwrap()).unwrap()
}

fn ids(selection: &lorepia_lorebook::Selection) -> Vec<&str> {
    selection
        .receipt()
        .selected_ids()
        .iter()
        .map(EntryId::as_str)
        .collect()
}

#[test]
fn lore_001_zero_one_one_thousand_and_ten_thousand_entries() {
    for count in [0usize, 1, 1_000, 10_000] {
        let entries = (0..count)
            .map(|index| {
                entry(
                    &format!("entry-{index:05}"),
                    EntrySource::Global,
                    0,
                    index as i32,
                    Activation::Constant,
                    "x",
                    1,
                )
            })
            .collect();
        let selected = engine(entries).select(&request("")).unwrap();
        assert_eq!(selected.receipt().selected_ids().len(), count);
    }
}

#[test]
fn lore_002_every_entry_can_match_and_overlapping_keys_are_preserved() {
    let entries = (0..1_000)
        .map(|index| {
            entry(
                &format!("all-{index}"),
                EntrySource::Global,
                0,
                index,
                selective(if index % 2 == 0 { "aba" } else { "ba" }),
                "matched",
                1,
            )
        })
        .collect();
    let selected = engine(entries).select(&request("ababa")).unwrap();
    assert_eq!(selected.receipt().matched_count(), 1_000);
}

#[test]
fn lore_003_no_entry_matches_without_a_candidate_hit() {
    let selected = engine(vec![entry(
        "never",
        EntrySource::Global,
        0,
        0,
        selective("dragon"),
        "secret",
        1,
    )])
    .select(&request("ordinary conversation"))
    .unwrap();
    assert!(selected.prompt_text().is_empty());
    assert_eq!(selected.receipt().candidate_count(), 0);
}

#[test]
fn lore_004_and_022_order_is_priority_scope_order_then_id() {
    let entries = vec![
        entry("z", EntrySource::Global, 8, 0, Activation::Constant, "z", 1),
        entry("b", EntrySource::Chat, 7, 5, Activation::Constant, "b", 1),
        entry("a", EntrySource::Chat, 7, 5, Activation::Constant, "a", 1),
        entry(
            "card",
            EntrySource::CharacterCard,
            7,
            0,
            Activation::Constant,
            "card",
            1,
        ),
        entry(
            "global",
            EntrySource::Global,
            7,
            -1,
            Activation::Constant,
            "global",
            1,
        ),
    ];
    let selected = engine(entries).select(&request("")).unwrap();
    assert_eq!(ids(&selected), ["z", "a", "b", "card", "global"]);
}

#[test]
fn lore_005_one_character_and_maximum_length_keys() {
    let long = "가".repeat(MAX_KEY_BYTES / "가".len());
    let selected = engine(vec![
        entry("one", EntrySource::Global, 0, 0, selective("x"), "one", 1),
        entry(
            "long",
            EntrySource::Global,
            0,
            1,
            selective(long.clone()),
            "long",
            1,
        ),
    ])
    .select(&request(&format!("x {long}")))
    .unwrap();
    assert_eq!(selected.receipt().selected_ids().len(), 2);
}

#[test]
fn lore_006_korean_particles_spacing_nfkc_and_case_are_deterministic() {
    // Decomposed Hangul normalizes to the composed key; an attached particle
    // still matches because keys are substring conditions. Whitespace runs
    // collapse without language-specific guessing.
    let selected = engine(vec![
        entry(
            "hangul",
            EntrySource::Global,
            0,
            0,
            selective("한글"),
            "hangul",
            1,
        ),
        entry(
            "space",
            EntrySource::Global,
            0,
            1,
            selective("마법 학교"),
            "space",
            1,
        ),
        entry(
            "case",
            EntrySource::Global,
            0,
            2,
            selective("LORE"),
            "case",
            1,
        ),
    ])
    .select(&request("한글에서 마법\n\t 학교 lore"))
    .unwrap();
    assert_eq!(ids(&selected), ["hangul", "space", "case"]);
}

#[test]
fn lore_007_and_008_rust_regex_handles_catastrophic_shape_without_backtracking() {
    let selected = engine(vec![entry(
        "regex",
        EntrySource::Global,
        0,
        0,
        Activation::Selective {
            conditions: MatchConditions::any(vec![MatchCondition::regex("^(a+)+b$", "a")]),
        },
        "matched",
        1,
    )])
    .select(&request(&format!("{}c", "a".repeat(100_000))))
    .unwrap();
    assert!(selected.prompt_text().is_empty());
    assert_eq!(selected.receipt().regex_evaluations(), 1);
}

#[test]
fn lore_009_regex_scan_and_match_limits_reject_max_plus_one() {
    let regex_entry = entry(
        "regex",
        EntrySource::Global,
        0,
        0,
        Activation::Selective {
            conditions: MatchConditions::any(vec![MatchCondition::regex("a", "a")]),
        },
        "matched",
        1,
    );
    let regex_engine = engine(vec![regex_entry]);

    let mut scan_limited = request("aaaa");
    scan_limited.settings.max_regex_scan_bytes = 3;
    assert!(matches!(
        regex_engine.select(&scan_limited),
        Err(LorebookError::SearchLimitExceeded {
            limit: LimitKind::RegexScanBytes
        })
    ));

    let mut match_limited = request("aaaa");
    match_limited.settings.max_regex_matches = 3;
    assert!(matches!(
        regex_engine.select(&match_limited),
        Err(LorebookError::SearchLimitExceeded {
            limit: LimitKind::RegexMatches
        })
    ));

    let two_regexes = engine(vec![
        entry(
            "regex-a",
            EntrySource::Global,
            0,
            0,
            Activation::Selective {
                conditions: MatchConditions::any(vec![MatchCondition::regex("a", "unused")]),
            },
            "a",
            1,
        ),
        entry(
            "regex-b",
            EntrySource::Global,
            0,
            1,
            Activation::Selective {
                conditions: MatchConditions::any(vec![MatchCondition::regex("a", "unused")]),
            },
            "b",
            1,
        ),
    ]);
    let mut evaluation_limited = request("a");
    evaluation_limited.settings.max_regex_evaluations = 1;
    assert!(matches!(
        two_regexes.select(&evaluation_limited),
        Err(LorebookError::SearchLimitExceeded {
            limit: LimitKind::RegexEvaluations
        })
    ));
}

#[test]
fn lore_025_nested_literal_match_explosion_fails_closed_at_work_limit() {
    let entries = (1..=1_024)
        .map(|length| {
            entry(
                &format!("nested-{length}"),
                EntrySource::Global,
                0,
                length,
                selective("a".repeat(length as usize)),
                "x",
                1,
            )
        })
        .collect();
    let engine = engine(entries);
    let mut hostile = request("");
    hostile.conversation.total_turns = 8;
    hostile.conversation.recent_turns = (1..=8)
        .map(|revision| ChatTurn {
            revision,
            state: MessageState::Complete,
            content: "a".repeat(MAX_TURN_BYTES),
        })
        .collect();
    hostile.settings.search_depth = 8;
    hostile.settings.max_literal_match_events = 50_000;
    assert!(matches!(
        engine.select(&hostile),
        Err(LorebookError::SearchLimitExceeded {
            limit: LimitKind::LiteralMatchEvents
        })
    ));
}

#[test]
fn lore_010_import_reports_invalid_regex_without_echoing_it() {
    let catalog = LorebookCatalog::new(
        1,
        vec![entry(
            "bad",
            EntrySource::Global,
            0,
            0,
            Activation::Selective {
                conditions: MatchConditions::any(vec![MatchCondition::regex("x+", "x")]),
            },
            "private lore",
            1,
        )],
    )
    .unwrap();
    let bytes = export_catalog(&catalog).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["catalog"]["entries"][0]["activation"]["conditions"]["primary"][0]["pattern"] =
        "(?P<secret>".into();
    let invalid_bytes = serde_json::to_vec(&value).unwrap();
    let quarantined = import_catalog(&invalid_bytes).unwrap();
    assert!(!quarantined.entries()[0].enabled());
    let error = import_trusted_engine(&invalid_bytes).unwrap_err();
    let debug = format!("{error:?}");
    assert!(matches!(
        error,
        LorebookError::InvalidRegex { entry_index: 0 }
    ));
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("private lore"));
}

#[test]
fn lore_011_regex_hints_cannot_create_false_negative_candidates() {
    let selected = engine(vec![
        entry(
            "literal",
            EntrySource::Global,
            0,
            0,
            selective("fox"),
            "literal",
            1,
        ),
        entry(
            "regex",
            EntrySource::Global,
            0,
            1,
            Activation::Selective {
                conditions: MatchConditions::any(vec![MatchCondition::regex("fox[0-9]+", "fox")]),
            },
            "regex",
            1,
        ),
    ])
    .select(&request("fox42"))
    .unwrap();
    assert_eq!(selected.receipt().candidate_count(), 2);
    assert_eq!(selected.receipt().matched_count(), 2);

    let miss = engine(vec![entry(
        "regex",
        EntrySource::Global,
        0,
        0,
        Activation::Selective {
            conditions: MatchConditions::any(vec![MatchCondition::regex("fox[0-9]+", "fox")]),
        },
        "regex",
        1,
    )])
    .select(&request("wolf42"))
    .unwrap();
    assert_eq!(miss.receipt().candidate_count(), 1);
    assert_eq!(miss.receipt().regex_evaluations(), 1);
    assert!(miss.prompt_text().is_empty());

    let dishonest_hint = engine(vec![entry(
        "sound",
        EntrySource::Global,
        0,
        0,
        Activation::Selective {
            conditions: MatchConditions::any(vec![MatchCondition::regex("dragon", "never")]),
        },
        "sound",
        1,
    )])
    .select(&request("dragon"))
    .unwrap();
    assert_eq!(ids(&dishonest_hint), ["sound"]);

    // Regex is evaluated against bounded raw Unicode input, so its pattern and
    // case option cannot disagree with a separately normalized candidate gate.
    let fullwidth = engine(vec![entry(
        "unicode-regex",
        EntrySource::Global,
        0,
        0,
        Activation::Selective {
            conditions: MatchConditions::any(vec![MatchCondition::regex("ＦＯＯ", "not-used")]),
        },
        "unicode",
        1,
    )])
    .select(&request("ＦＯＯ"))
    .unwrap();
    assert_eq!(ids(&fullwidth), ["unicode-regex"]);
}

#[test]
fn secondary_keys_support_any_and_all_without_losing_primary_gate() {
    let selected = engine(vec![
        entry(
            "all",
            EntrySource::Global,
            0,
            0,
            Activation::Selective {
                conditions: MatchConditions {
                    primary: vec![MatchCondition::literal("hero")],
                    secondary: SecondaryConditions::All(vec![
                        MatchCondition::literal("sword"),
                        MatchCondition::literal("shield"),
                    ]),
                },
            },
            "all",
            1,
        ),
        entry(
            "any",
            EntrySource::Global,
            0,
            1,
            Activation::Selective {
                conditions: MatchConditions {
                    primary: vec![MatchCondition::literal("hero")],
                    secondary: SecondaryConditions::Any(vec![
                        MatchCondition::literal("wand"),
                        MatchCondition::literal("shield"),
                    ]),
                },
            },
            "any",
            1,
        ),
    ])
    .select(&request("hero sword shield"))
    .unwrap();
    assert_eq!(ids(&selected), ["all", "any"]);
}

#[test]
fn lore_012_013_and_014_no_expansion_or_recursion_and_output_is_bounded() {
    let raw = "${a} {{recursive::a}} <script>not code</script>";
    let engine = engine(vec![
        entry(
            "raw",
            EntrySource::Global,
            10,
            0,
            Activation::Constant,
            raw,
            1,
        ),
        entry(
            "too-big",
            EntrySource::Global,
            0,
            1,
            Activation::Constant,
            "0123456789",
            1,
        ),
    ]);
    let mut request = request("");
    request.settings.max_output_bytes = raw.len();
    let selected = engine.select(&request).unwrap();
    assert_eq!(selected.prompt_text(), raw);
}

#[test]
fn lore_015_token_and_byte_budget_use_exact_deterministic_boundaries() {
    let engine = engine(vec![
        entry(
            "a",
            EntrySource::Global,
            3,
            0,
            Activation::Constant,
            "aaa",
            3,
        ),
        entry(
            "b",
            EntrySource::Global,
            2,
            0,
            Activation::Constant,
            "bbbb",
            4,
        ),
        entry("c", EntrySource::Global, 1, 0, Activation::Constant, "c", 1),
    ]);
    let mut request = request("");
    request.settings.max_output_tokens = 4;
    request.settings.max_output_bytes = 6; // "aaa\n\nc" is exactly six bytes.
    let selected = engine.select(&request).unwrap();
    assert_eq!(selected.prompt_text(), "aaa\n\nc");
    assert_eq!(selected.receipt().selected_tokens(), 4);
    assert_eq!(selected.receipt().selected_bytes(), 6);
}

#[test]
fn lore_016_tokenizer_revision_is_part_of_cache_identity() {
    let engine = engine(vec![entry(
        "a",
        EntrySource::Global,
        0,
        0,
        Activation::Constant,
        "lore",
        1,
    )]);
    let first = engine.select(&request("")).unwrap();
    let cached = engine.select(&request("")).unwrap();
    assert!(!first.receipt().cache_hit());
    assert!(cached.receipt().cache_hit());

    let mut changed = request("");
    changed.settings.tokenizer_revision = "2".to_owned();
    let changed = engine.select(&changed).unwrap();
    assert!(!changed.receipt().cache_hit());
    assert_ne!(
        first.receipt().cache_fingerprint(),
        changed.receipt().cache_fingerprint()
    );
}

#[test]
fn lore_017_a_million_turn_conversation_uses_only_the_bounded_tail_and_depth() {
    let engine = engine(vec![
        entry(
            "old",
            EntrySource::Global,
            0,
            0,
            selective("old-key"),
            "old",
            1,
        ),
        entry(
            "new",
            EntrySource::Global,
            0,
            1,
            selective("new-key"),
            "new",
            1,
        ),
    ]);
    let mut request = request("");
    request.conversation.total_turns = 1_000_000;
    request.conversation.recent_turns = vec![
        ChatTurn {
            revision: 999_999,
            state: MessageState::Complete,
            content: "old-key".to_owned(),
        },
        ChatTurn {
            revision: 1_000_000,
            state: MessageState::Complete,
            content: "new-key".to_owned(),
        },
    ];
    request.settings.search_depth = 1;
    let selected = engine.select(&request).unwrap();
    assert_eq!(ids(&selected), ["new"]);
}

#[test]
fn lore_018_missing_or_corrupt_summary_falls_back_to_recent_chat() {
    let engine = engine(vec![
        entry(
            "chat",
            EntrySource::Global,
            0,
            0,
            selective("chat-key"),
            "chat",
            1,
        ),
        entry(
            "summary",
            EntrySource::Global,
            0,
            1,
            selective("summary-key"),
            "summary",
            1,
        ),
    ]);
    for summary in [
        SummarySnapshot::Missing,
        SummarySnapshot::Corrupt { revision: 9 },
    ] {
        let mut request = request("chat-key");
        request.conversation.summary = summary;
        assert_eq!(ids(&engine.select(&request).unwrap()), ["chat"]);
    }
}

#[test]
fn lore_019_branch_revision_and_content_change_cache_and_context() {
    let engine = engine(vec![
        entry("a", EntrySource::Global, 0, 0, selective("alpha"), "a", 1),
        entry("b", EntrySource::Global, 0, 1, selective("beta"), "b", 1),
    ]);
    let first = engine.select(&request("alpha")).unwrap();
    let mut branch = request("beta");
    branch.conversation.branch_revision = 2;
    let second = engine.select(&branch).unwrap();
    assert_eq!(ids(&first), ["a"]);
    assert_eq!(ids(&second), ["b"]);
    assert_ne!(
        first.receipt().cache_fingerprint(),
        second.receipt().cache_fingerprint()
    );
}

#[test]
fn lore_020_partial_policy_is_explicit_and_lore_021_failed_is_always_excluded() {
    let engine = engine(vec![
        entry(
            "partial",
            EntrySource::Global,
            0,
            0,
            selective("partial-key"),
            "partial",
            1,
        ),
        entry(
            "failed",
            EntrySource::Global,
            0,
            1,
            selective("failed-key"),
            "failed",
            1,
        ),
    ]);
    let mut request = request("");
    request.conversation.total_turns = 2;
    request.conversation.recent_turns = vec![
        ChatTurn {
            revision: 1,
            state: MessageState::Partial,
            content: "partial-key".to_owned(),
        },
        ChatTurn {
            revision: 2,
            state: MessageState::Failed,
            content: "failed-key".to_owned(),
        },
    ];
    assert!(engine.select(&request).unwrap().prompt_text().is_empty());
    request.settings.partial_messages = PartialMessagePolicy::Include;
    assert_eq!(ids(&engine.select(&request).unwrap()), ["partial"]);
}

#[test]
fn lore_023_probability_is_seeded_and_byte_for_byte_deterministic() {
    let entries = (0..100)
        .map(|index| {
            entry(
                &format!("p-{index}"),
                EntrySource::Global,
                0,
                index,
                Activation::Probability {
                    basis_points: 5_000,
                    conditions: None,
                },
                &format!("lore-{index}"),
                1,
            )
        })
        .collect();
    let engine = engine(entries);
    let first = engine.select(&request("")).unwrap();
    let second = engine.select(&request("")).unwrap();
    assert_eq!(first.prompt_text(), second.prompt_text());
    assert_eq!(ids(&first), ids(&second));

    let mut changed = request("");
    changed.seed = 8;
    assert_ne!(
        first.prompt_text(),
        engine.select(&changed).unwrap().prompt_text()
    );
}

#[test]
fn lore_024_debug_receipts_errors_and_models_redact_raw_content() {
    let secret = "TOP-SECRET-PROMPT-CONTENT";
    let entry = entry(
        "safe-id",
        EntrySource::Global,
        0,
        0,
        selective(secret),
        secret,
        1,
    );
    assert!(!format!("{entry:?}").contains(secret));
    let engine = engine(vec![entry]);
    assert!(!format!("{engine:?}").contains(secret));
    let selected = engine.select(&request(secret)).unwrap();
    assert!(!format!("{selected:?}").contains(secret));

    let error = import_catalog(br#"{"unknown-secret-field":true}"#).unwrap_err();
    assert!(!format!("{error:?}").contains("unknown-secret-field"));
}

#[test]
fn untrusted_import_disables_entries_and_schema_is_closed_and_versioned() {
    let catalog = LorebookCatalog::new(
        3,
        vec![entry(
            "active",
            EntrySource::Global,
            0,
            0,
            Activation::Constant,
            "lore",
            1,
        )],
    )
    .unwrap();
    let bytes = export_catalog(&catalog).unwrap();
    let untrusted = import_catalog(&bytes).unwrap();
    assert!(!untrusted.entries()[0].enabled());
    let trusted = import_catalog_with_trust(&bytes, ImportTrust::LocallyTrusted).unwrap();
    assert!(trusted.entries()[0].enabled());

    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["extra"] = true.into();
    assert!(matches!(
        import_catalog(&serde_json::to_vec(&value).unwrap()),
        Err(LorebookError::ImportSchema)
    ));
    value.as_object_mut().unwrap().remove("extra");
    value["schemaVersion"] = 2.into();
    assert!(matches!(
        import_catalog(&serde_json::to_vec(&value).unwrap()),
        Err(LorebookError::UnsupportedImportVersion)
    ));
}

#[test]
fn replacing_catalog_invalidates_cache_even_if_external_revision_is_reused() {
    let mut engine = engine(vec![entry(
        "a",
        EntrySource::Global,
        0,
        0,
        Activation::Constant,
        "a",
        1,
    )]);
    assert!(!engine.select(&request("")).unwrap().receipt().cache_hit());
    assert!(engine.select(&request("")).unwrap().receipt().cache_hit());
    engine
        .replace_catalog(
            LorebookCatalog::new(
                1,
                vec![entry(
                    "b",
                    EntrySource::Global,
                    0,
                    0,
                    Activation::Constant,
                    "b",
                    1,
                )],
            )
            .unwrap(),
        )
        .unwrap();
    let replaced = engine.select(&request("")).unwrap();
    assert!(!replaced.receipt().cache_hit());
    assert_eq!(replaced.prompt_text(), "b");
}

#[test]
fn lorebook_output_is_consumed_by_prompt_as_opaque_lorebook_data() {
    let raw = "${not-expanded} {{not-a-macro}}";
    let selected = engine(vec![entry(
        "raw",
        EntrySource::Global,
        0,
        0,
        Activation::Constant,
        raw,
        4,
    )])
    .select(&request(""))
    .unwrap();
    let preset = PromptPreset {
        name: "consumer-contract".to_owned(),
        blocks: vec![PromptBlock::Lorebook {
            name: "lorebook".to_owned(),
            enabled: true,
            role: PromptRole::System,
            format: ContentFormat::Plain,
        }],
        sampling: Default::default(),
        advanced: Default::default(),
    };
    let input = PromptCompileInput {
        lorebook: selected.prompt_text().to_owned(),
        variables: BTreeMap::new(),
        ..Default::default()
    };
    let compiled = compile_prompt(&preset, &input).unwrap();
    assert_eq!(compiled.messages.len(), 1);
    assert_eq!(compiled.messages[0].content, raw);
}

#[test]
#[ignore = "LORE-001/025 metadata-only 100k performance smoke; run explicitly"]
fn lore_001_and_025_one_hundred_thousand_entries_use_index_on_a_worker() {
    let entries = (0..100_000)
        .map(|index| {
            entry(
                &format!("entry-{index:06}"),
                EntrySource::Global,
                0,
                index,
                selective(format!("key-{index:06}")),
                "x",
                1,
            )
        })
        .collect();
    let engine = Arc::new(engine(entries));
    let worker = {
        let engine = Arc::clone(&engine);
        std::thread::spawn(move || engine.select(&request("key-099999")))
    };
    let selected = worker.join().unwrap().unwrap();
    assert_eq!(ids(&selected), ["entry-099999"]);
    assert_eq!(selected.receipt().candidate_count(), 1);
}
