mod common;

use lorepia_storage::{
    BeginTurn, MessageStatus, RequestFailureCode, RequestStatus, ResponseCheckpoint, StorageError,
    TokenUsage,
};

use common::{begin_turn, checkpoint, create_chat, database, selection, timestamp};

#[test]
fn checkpoint_atomically_advances_a_batched_sequence_and_metadata() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);

    let metadata_only = store
        .checkpoint_response(ResponseCheckpoint {
            provider_response_id: Some("provider-response".to_owned()),
            usage: Some(TokenUsage {
                input_tokens: 5,
                output_tokens: 0,
                cached_input_tokens: 2,
                reasoning_tokens: 0,
            }),
            ..checkpoint(&started, 0, 1, "", 40)
        })
        .expect("metadata checkpoint");
    assert_eq!(metadata_only.last_seq, 1);
    assert_eq!(metadata_only.text_bytes, 0);

    let batch = store
        .checkpoint_response(ResponseCheckpoint {
            provider_response_id: Some("provider-response".to_owned()),
            usage: Some(TokenUsage {
                input_tokens: 5,
                output_tokens: 4,
                cached_input_tokens: 2,
                reasoning_tokens: 1,
            }),
            ..checkpoint(&started, 1, 5, "batched", 50)
        })
        .expect("batched checkpoint");
    assert_eq!(batch.last_seq, 5);
    assert_eq!(batch.text_bytes, 7);

    let state = store
        .get_request_state(&started.request_state_id)
        .expect("request state");
    assert_eq!(state.last_seq, 5);
    assert_eq!(
        state.provider_response_id.as_deref(),
        Some("provider-response")
    );
    assert_eq!(state.usage.expect("usage").output_tokens, 4);
}

#[test]
fn stale_sequence_or_nonmonotonic_metadata_rolls_back_text() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);
    store
        .checkpoint_response(ResponseCheckpoint {
            provider_response_id: Some("stable-id".to_owned()),
            usage: Some(TokenUsage {
                input_tokens: 4,
                output_tokens: 4,
                cached_input_tokens: 0,
                reasoning_tokens: 0,
            }),
            ..checkpoint(&started, 0, 2, "first", 40)
        })
        .expect("first checkpoint");

    let stale = store
        .checkpoint_response(checkpoint(&started, 0, 3, "must-not-append", 50))
        .expect_err("reject stale sequence");
    assert!(matches!(
        stale,
        StorageError::SequenceMismatch {
            expected: 2,
            actual: 0
        }
    ));

    let changed_id = store
        .checkpoint_response(ResponseCheckpoint {
            provider_response_id: Some("different-id".to_owned()),
            ..checkpoint(&started, 2, 3, "must-not-append", 50)
        })
        .expect_err("reject changed response ID");
    assert!(matches!(
        changed_id,
        StorageError::InvalidInput {
            field: "provider response ID",
            ..
        }
    ));

    let lower_usage = store
        .checkpoint_response(ResponseCheckpoint {
            usage: Some(TokenUsage {
                input_tokens: 4,
                output_tokens: 3,
                cached_input_tokens: 0,
                reasoning_tokens: 0,
            }),
            ..checkpoint(&started, 2, 3, "must-not-append", 50)
        })
        .expect_err("reject decreasing usage");
    assert!(matches!(
        lower_usage,
        StorageError::InvalidInput {
            field: "token usage",
            ..
        }
    ));

    let messages = store
        .load_messages(&chat.id, None, 10)
        .expect("load messages");
    assert_eq!(messages.messages[1].text, "first");
    assert_eq!(
        store
            .get_request_state(&started.request_state_id)
            .expect("state")
            .last_seq,
        2
    );
}

#[test]
fn terminal_flush_and_status_transition_are_atomic() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");

    let completed_chat = create_chat(&store, 20);
    let completed = begin_turn(&store, &completed_chat, "one", 30);
    store
        .complete_turn(checkpoint(&completed, 0, 3, "finished", 40))
        .expect("complete with final flush");
    let complete_messages = store
        .load_messages(&completed_chat.id, None, 10)
        .expect("complete messages");
    assert_eq!(complete_messages.messages[1].text, "finished");
    assert_eq!(
        complete_messages.messages[1].status,
        MessageStatus::Complete
    );

    let failed_chat = create_chat(&store, 50);
    let failed = begin_turn(&store, &failed_chat, "two", 60);
    store
        .fail_turn(
            checkpoint(&failed, 0, 2, "partial answer", 70),
            RequestFailureCode::Timeout,
        )
        .expect("fail with final flush");
    let failed_messages = store
        .load_messages(&failed_chat.id, None, 10)
        .expect("failed messages");
    assert_eq!(failed_messages.messages[1].text, "partial answer");
    assert_eq!(failed_messages.messages[1].status, MessageStatus::Failed);
    let state = store
        .get_request_state(&failed.request_state_id)
        .expect("failed state");
    assert_eq!(state.status, RequestStatus::Failed);
    assert_eq!(state.failure_code, Some(RequestFailureCode::Timeout));

    let second_terminal = store
        .cancel_turn(checkpoint(&failed, 2, 3, "not stored", 80))
        .expect_err("terminal state is immutable");
    assert!(matches!(second_terminal, StorageError::InvalidState { .. }));
    assert_eq!(
        store
            .load_messages(&failed_chat.id, None, 10)
            .expect("messages after rejection")
            .messages[1]
            .text,
        "partial answer"
    );
}

#[test]
fn app_restarted_code_is_reserved_and_concurrent_turn_is_rolled_back() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "first", 30);

    let conflict = store
        .begin_turn(BeginTurn {
            chat_id: chat.id.clone(),
            selection: selection(),
            user_text: "second".to_owned(),
            started_at_ms: timestamp(40),
        })
        .expect_err("only one running request per chat");
    assert!(matches!(
        conflict,
        StorageError::Conflict {
            entity: "active request"
        }
    ));
    assert_eq!(
        store
            .load_messages(&chat.id, None, 10)
            .expect("rolled back messages")
            .messages
            .len(),
        2
    );

    let reserved = store
        .fail_turn(
            checkpoint(&started, 0, 1, "", 50),
            RequestFailureCode::AppRestarted,
        )
        .expect_err("startup-only failure code");
    assert!(matches!(
        reserved,
        StorageError::InvalidInput {
            field: "failure code",
            ..
        }
    ));
    assert_eq!(
        store
            .get_request_state(&started.request_state_id)
            .expect("still running")
            .status,
        RequestStatus::Running
    );
}
