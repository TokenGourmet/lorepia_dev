mod common;

use lorepia_storage::{
    BeginTurn, CumulativeAck, DeliveryCheckpoint, MessageStatus, RequestFailureCode, RequestStatus,
    ResponseCheckpoint, StorageError, StreamGeneration, StreamOwnerLabel, TokenUsage,
};

use common::{
    begin_turn, checkpoint, create_chat, database, deliver_through, selection, timestamp,
};

#[test]
fn checkpoint_atomically_advances_a_batched_sequence_and_metadata() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);
    deliver_through(&store, &started, 0, 5, 35);

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
    assert_eq!(metadata_only.last_durable_seq, 1);
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
    assert_eq!(batch.last_durable_seq, 5);
    assert_eq!(batch.text_bytes, 7);

    let state = store
        .get_request_state(&started.request_state_id)
        .expect("request state");
    assert_eq!(state.last_delivered_seq, 5);
    assert_eq!(state.last_durable_seq, 5);
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
    deliver_through(&store, &started, 0, 3, 35);
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
            .last_durable_seq,
        2
    );
}

#[test]
fn terminal_flush_and_status_transition_are_atomic() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");

    let completed_chat = create_chat(&store, 20);
    let completed = begin_turn(&store, &completed_chat, "one", 30);
    deliver_through(&store, &completed, 0, 3, 35);
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
    deliver_through(&store, &failed, 0, 3, 65);
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
            owner_label: StreamOwnerLabel::parse("main").expect("owner label"),
            stream_generation: StreamGeneration::new(),
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

#[test]
fn delivery_durable_and_cumulative_ack_sequences_are_owned_and_ordered() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);

    let delivery = |expected_last_delivered_seq, through_seq, at_ms| DeliveryCheckpoint {
        request_state_id: started.request_state_id.clone(),
        owner_label: started.owner_label.clone(),
        stream_generation: started.stream_generation.clone(),
        expected_last_delivered_seq,
        through_seq,
        at_ms: timestamp(at_ms),
    };
    store
        .record_response_delivery(delivery(0, 1, 31))
        .expect("first delivery");

    let replay = store
        .record_response_delivery(delivery(0, 1, 32))
        .expect_err("reject replayed delivery");
    assert!(matches!(replay, StorageError::SequenceMismatch { .. }));
    let gap = store
        .record_response_delivery(delivery(1, 3, 32))
        .expect_err("reject delivery gap");
    assert!(matches!(
        gap,
        StorageError::InvalidInput {
            field: "delivered sequence",
            ..
        }
    ));
    let forged_owner = store
        .record_response_delivery(DeliveryCheckpoint {
            owner_label: StreamOwnerLabel::parse("other").expect("owner label"),
            ..delivery(1, 2, 32)
        })
        .expect_err("reject another owner");
    assert!(matches!(
        forged_owner,
        StorageError::Conflict {
            entity: "stream identity"
        }
    ));
    let forged_generation = store
        .record_response_delivery(DeliveryCheckpoint {
            stream_generation: StreamGeneration::new(),
            ..delivery(1, 2, 32)
        })
        .expect_err("reject stale generation");
    assert!(matches!(
        forged_generation,
        StorageError::Conflict {
            entity: "stream identity"
        }
    ));

    store
        .record_response_delivery(delivery(1, 2, 32))
        .expect("second delivery");
    store
        .record_response_delivery(delivery(2, 3, 33))
        .expect("third delivery");
    let durable = store
        .checkpoint_response(checkpoint(&started, 0, 2, "two deltas", 34))
        .expect("durable batch through two");
    assert_eq!(durable.last_delivered_seq, 3);
    assert_eq!(durable.last_durable_seq, 2);
    assert_eq!(durable.last_acked_seq, None);

    let ahead = store
        .acknowledge_response(CumulativeAck {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: started.stream_generation.clone(),
            expected_last_acked_seq: None,
            through_seq: 3,
            at_ms: timestamp(35),
        })
        .expect_err("ACK cannot outrun durability");
    assert!(matches!(
        ahead,
        StorageError::InvalidInput {
            field: "acknowledged sequence",
            ..
        }
    ));
    let acked = store
        .acknowledge_response(CumulativeAck {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: started.stream_generation.clone(),
            expected_last_acked_seq: None,
            through_seq: 2,
            at_ms: timestamp(35),
        })
        .expect("cumulative ACK through durable sequence");
    assert_eq!(acked.last_acked_seq, Some(2));
    let replayed_ack = store
        .acknowledge_response(CumulativeAck {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: started.stream_generation.clone(),
            expected_last_acked_seq: None,
            through_seq: 2,
            at_ms: timestamp(36),
        })
        .expect_err("reject replayed cumulative ACK");
    assert!(matches!(
        replayed_ack,
        StorageError::SequenceMismatch { .. }
    ));

    store
        .complete_turn(checkpoint(&started, 2, 3, "third", 36))
        .expect("terminal commit");
    let forged_terminal_ack = store
        .acknowledge_response(CumulativeAck {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: StreamGeneration::new(),
            expected_last_acked_seq: Some(2),
            through_seq: 3,
            at_ms: timestamp(37),
        })
        .expect_err("terminal ACK remains generation-bound");
    assert!(matches!(
        forged_terminal_ack,
        StorageError::Conflict {
            entity: "stream identity"
        }
    ));
    let terminal_ack = store
        .acknowledge_response(CumulativeAck {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: started.stream_generation.clone(),
            expected_last_acked_seq: Some(2),
            through_seq: 3,
            at_ms: timestamp(37),
        })
        .expect("persist cumulative terminal ACK without changing terminal state");
    assert_eq!(terminal_ack.status, RequestStatus::Completed);
    assert_eq!(terminal_ack.last_acked_seq, Some(3));
    let terminal_delivery = store
        .record_response_delivery(delivery(3, 4, 38))
        .expect_err("terminal request rejects new delivery");
    assert!(matches!(
        terminal_delivery,
        StorageError::InvalidState { .. }
    ));

    let state = store
        .get_request_state(&started.request_state_id)
        .expect("journal state");
    assert_eq!(state.owner_label, started.owner_label);
    assert_eq!(state.stream_generation, started.stream_generation);
    assert_eq!(state.last_delivered_seq, 3);
    assert_eq!(state.last_durable_seq, 3);
    assert_eq!(state.last_acked_seq, Some(3));
    assert_eq!(state.status, RequestStatus::Completed);
}

#[test]
fn stream_journal_clamps_wall_clock_regressions_without_blocking_progress() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 100);

    let first_delivery = store
        .record_response_delivery(lorepia_storage::DeliveryCheckpoint {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: started.stream_generation.clone(),
            expected_last_delivered_seq: 0,
            through_seq: 1,
            at_ms: timestamp(90),
        })
        .expect("delivery survives wall clock regression");
    assert_eq!(first_delivery.updated_at_ms, timestamp(100));
    store
        .checkpoint_response(checkpoint(&started, 0, 1, "one", 80))
        .expect("durability survives wall clock regression");
    let first_ack = store
        .acknowledge_response(CumulativeAck {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: started.stream_generation.clone(),
            expected_last_acked_seq: None,
            through_seq: 1,
            at_ms: timestamp(70),
        })
        .expect("ACK survives wall clock regression");
    assert_eq!(first_ack.updated_at_ms, timestamp(100));

    let future_delivery = store
        .record_response_delivery(lorepia_storage::DeliveryCheckpoint {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: started.stream_generation.clone(),
            expected_last_delivered_seq: 1,
            through_seq: 2,
            at_ms: timestamp(1_000),
        })
        .expect("future wall clock jump is retained");
    assert_eq!(future_delivery.updated_at_ms, timestamp(1_000));
    let terminal = store
        .complete_turn(checkpoint(&started, 1, 2, "two", 500))
        .expect("terminal timestamp clamps to prior future jump");
    assert_eq!(terminal.updated_at_ms, timestamp(1_000));
    let terminal_ack = store
        .acknowledge_response(CumulativeAck {
            request_state_id: started.request_state_id.clone(),
            owner_label: started.owner_label.clone(),
            stream_generation: started.stream_generation.clone(),
            expected_last_acked_seq: Some(1),
            through_seq: 2,
            at_ms: timestamp(400),
        })
        .expect("terminal ACK clamps independently of wall clock");
    assert_eq!(terminal_ack.updated_at_ms, timestamp(1_000));

    let state = store
        .get_request_state(&started.request_state_id)
        .expect("clamped state");
    assert_eq!(state.updated_at_ms, timestamp(1_000));
    assert_eq!(state.finished_at_ms, Some(timestamp(1_000)));
    assert_eq!(state.last_acked_seq, Some(2));
}
