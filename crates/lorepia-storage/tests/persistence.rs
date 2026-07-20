mod common;

use lorepia_storage::{
    DefaultMode, ProviderId, ProviderModelIds, RequestStatus, StorageError, Theme, TokenUsage,
    UpdatePreferences,
};

use common::{begin_turn, checkpoint, create_chat, database, timestamp};

#[test]
fn persists_chat_turn_usage_and_search_across_reopen() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(100)).expect("open store");
    let chat = create_chat(&store, 110);
    assert_eq!(chat.revision, 1);
    assert_eq!(chat.character_id.as_str(), "character-seraphine");

    let started = begin_turn(&store, &chat, "오늘 하늘은 어때?", 120);
    assert_eq!(started.last_seq, 0);
    let progress = store
        .checkpoint_response(lorepia_storage::ResponseCheckpoint {
            provider_response_id: Some("response-1".to_owned()),
            usage: Some(TokenUsage {
                input_tokens: 11,
                output_tokens: 3,
                cached_input_tokens: 4,
                reasoning_tokens: 0,
            }),
            ..checkpoint(&started, 0, 3, "맑은 하늘", 130)
        })
        .expect("batch checkpoint");
    assert_eq!(progress.last_seq, 3);
    assert_eq!(progress.text_bytes, "맑은 하늘".len());
    assert_eq!(progress.status, RequestStatus::Running);

    let terminal = store
        .complete_turn(lorepia_storage::ResponseCheckpoint {
            usage: Some(TokenUsage {
                input_tokens: 11,
                output_tokens: 8,
                cached_input_tokens: 4,
                reasoning_tokens: 1,
            }),
            ..checkpoint(&started, 3, 4, "이 펼쳐져 있어.", 140)
        })
        .expect("complete turn");
    assert_eq!(terminal.status, RequestStatus::Completed);
    drop(store);

    let reopened = lorepia_storage::Store::open_at(&path, timestamp(200)).expect("reopen store");
    let loaded_chat = reopened.get_chat(&chat.id).expect("load chat");
    assert_eq!(loaded_chat.character_id, chat.character_id);
    let messages = reopened
        .load_messages(&chat.id, None, 20)
        .expect("load messages");
    assert_eq!(messages.messages.len(), 2);
    assert_eq!(messages.messages[0].text, "오늘 하늘은 어때?");
    assert_eq!(messages.messages[1].text, "맑은 하늘이 펼쳐져 있어.");
    assert_eq!(
        messages.messages[1].status,
        lorepia_storage::MessageStatus::Complete
    );

    let request = reopened
        .get_request_state(&started.request_state_id)
        .expect("load request");
    assert_eq!(request.status, RequestStatus::Completed);
    assert_eq!(request.last_seq, 4);
    assert_eq!(request.provider_response_id.as_deref(), Some("response-1"));
    assert_eq!(request.usage.expect("usage").output_tokens, 8);

    let korean_hits = reopened
        .search_messages(&chat.id, "맑은 하늘", 10)
        .expect("FTS search");
    assert_eq!(korean_hits.len(), 1);
    assert_eq!(korean_hits[0].message.id, started.assistant_message_id);
    let short_hits = reopened
        .search_messages(&chat.id, "늘", 10)
        .expect("short substring search");
    assert_eq!(short_hits.len(), 2);
}

#[test]
fn chat_list_and_title_updates_use_stable_cursor_and_optimistic_revision() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let first = create_chat(&store, 20);
    let second = create_chat(&store, 30);
    let third = create_chat(&store, 40);

    let page_one = store.list_chats(2, None).expect("first page");
    assert_eq!(page_one.chats.len(), 2);
    assert_eq!(page_one.chats[0].id, third.id);
    assert_eq!(page_one.chats[1].id, second.id);
    let page_two = store
        .list_chats(2, page_one.next_cursor.as_ref())
        .expect("second page");
    assert_eq!(page_two.chats.len(), 1);
    assert_eq!(page_two.chats[0].id, first.id);
    assert!(page_two.next_cursor.is_none());

    let renamed = store
        .rename_chat(&first.id, 1, "새 제목", timestamp(50))
        .expect("rename chat");
    assert_eq!(renamed.revision, 2);
    assert_eq!(renamed.title, "새 제목");
    let conflict = store
        .rename_chat(&first.id, 1, "stale", timestamp(60))
        .expect_err("reject stale revision");
    assert!(matches!(
        conflict,
        StorageError::Conflict {
            entity: "chat revision"
        }
    ));
}

#[test]
fn typed_preferences_round_trip_and_reject_stale_revision() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let models = ProviderModelIds {
        openai: "gpt-4o".to_owned(),
        anthropic: "claude-sonnet".to_owned(),
        deepseek: "deepseek-chat".to_owned(),
        ollama_cloud: "".to_owned(),
        gemini: "gemini-2.5-flash".to_owned(),
    };
    let updated = store
        .save_preferences(UpdatePreferences {
            expected_revision: 0,
            selected_provider_id: ProviderId::Gemini,
            model_ids: models.clone(),
            theme: Theme::Dark,
            default_mode: DefaultMode::Story,
            at_ms: timestamp(20),
        })
        .expect("save preferences");
    assert_eq!(updated.revision, 1);
    assert_eq!(updated.selected_provider_id, ProviderId::Gemini);
    assert_eq!(updated.model_ids, models);
    assert_eq!(updated.theme, Theme::Dark);
    assert_eq!(updated.default_mode, DefaultMode::Story);

    let stale = store
        .save_preferences(UpdatePreferences {
            expected_revision: 0,
            selected_provider_id: ProviderId::OpenAi,
            model_ids: Default::default(),
            theme: Theme::System,
            default_mode: DefaultMode::Chat,
            at_ms: timestamp(30),
        })
        .expect_err("reject stale preferences");
    assert!(matches!(
        stale,
        StorageError::Conflict {
            entity: "settings revision"
        }
    ));
    assert_eq!(store.load_preferences().expect("reload"), updated);

    let control_character = store
        .save_preferences(UpdatePreferences {
            expected_revision: 1,
            selected_provider_id: ProviderId::OpenAi,
            model_ids: ProviderModelIds {
                openai: "bad\nmodel".to_owned(),
                ..Default::default()
            },
            theme: Theme::System,
            default_mode: DefaultMode::Chat,
            at_ms: timestamp(40),
        })
        .expect_err("reject control characters in stored model IDs");
    assert!(matches!(
        control_character,
        StorageError::InvalidInput {
            field: "model ID preference",
            ..
        }
    ));
}

#[test]
fn deleting_completed_chat_cascades_but_running_chat_is_protected() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);

    let error = store
        .delete_chat(&chat.id)
        .expect_err("active chat must not be deleted");
    assert!(matches!(
        error,
        StorageError::Conflict {
            entity: "chat with an active request"
        }
    ));

    store
        .cancel_turn(checkpoint(&started, 0, 1, "partial", 40))
        .expect("cancel turn");
    store.delete_chat(&chat.id).expect("delete completed chat");
    assert!(matches!(
        store.get_chat(&chat.id),
        Err(StorageError::NotFound { entity: "chat" })
    ));
    assert!(matches!(
        store.get_request_state(&started.request_state_id),
        Err(StorageError::NotFound {
            entity: "request state"
        })
    ));
}
