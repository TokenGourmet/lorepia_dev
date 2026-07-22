mod common;

use std::sync::{Arc, Barrier};

use lorepia_storage::{
    CumulativeAck, DeliveryCheckpoint, MessageStatus, RequestFailureCode, RequestStatus,
};

use common::{begin_turn, checkpoint, create_chat, database, deliver_through, timestamp};

#[test]
fn concurrent_same_sequence_checkpoint_has_exactly_one_winner() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);
    deliver_through(&store, &started, 0, 1, 35);
    let barrier = Arc::new(Barrier::new(3));

    let first_store = store.clone();
    let first_started = started.clone();
    let first_barrier = Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.checkpoint_response(checkpoint(&first_started, 0, 1, "alpha", 40))
    });
    let second_store = store.clone();
    let second_started = started.clone();
    let second_barrier = Arc::clone(&barrier);
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_store.checkpoint_response(checkpoint(&second_started, 0, 1, "beta", 40))
    });
    barrier.wait();

    let results = [
        first.join().expect("first thread"),
        second.join().expect("second thread"),
    ];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

    let state = store
        .get_request_state(&started.request_state_id)
        .expect("request state");
    assert_eq!(state.last_durable_seq, 1);
    let messages = store
        .load_messages(&chat.id, None, 10)
        .expect("load messages");
    assert!(matches!(
        messages.messages[1].text.as_str(),
        "alpha" | "beta"
    ));
}

#[test]
fn reopening_marks_running_request_interrupted_once_and_keeps_partial_text() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);
    deliver_through(&store, &started, 0, 2, 35);
    store
        .checkpoint_response(checkpoint(&started, 0, 2, "survived", 40))
        .expect("checkpoint");
    drop(store);

    let recovered = lorepia_storage::Store::open_at(&path, timestamp(200)).expect("recover store");
    assert_eq!(recovered.startup_report().recovered_request_count, 1);
    let state = recovered
        .get_request_state(&started.request_state_id)
        .expect("recovered state");
    assert_eq!(state.status, RequestStatus::Interrupted);
    assert_eq!(state.failure_code, Some(RequestFailureCode::AppRestarted));
    assert_eq!(state.last_durable_seq, 2);
    assert_eq!(state.finished_at_ms, Some(timestamp(200)));
    let messages = recovered
        .load_messages(&chat.id, None, 10)
        .expect("recovered messages");
    assert_eq!(messages.messages[1].text, "survived");
    assert_eq!(messages.messages[1].status, MessageStatus::Partial);
    drop(recovered);

    let reopened_again =
        lorepia_storage::Store::open_at(&path, timestamp(300)).expect("reopen again");
    assert_eq!(reopened_again.startup_report().recovered_request_count, 0);
    let unchanged = reopened_again
        .get_request_state(&started.request_state_id)
        .expect("unchanged state");
    assert_eq!(unchanged.finished_at_ms, Some(timestamp(200)));
    assert_eq!(unchanged.updated_at_ms, timestamp(200));
}

#[test]
fn independent_store_clones_observe_committed_wal_updates() {
    let (_directory, path) = database();
    let writer = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open writer");
    let reader = writer.clone();
    let chat = create_chat(&writer, 20);

    assert_eq!(
        reader
            .list_chats(10, None)
            .expect("initial read")
            .chats
            .len(),
        1
    );
    let started = begin_turn(&writer, &chat, "hello", 30);
    deliver_through(&writer, &started, 0, 3, 35);
    writer
        .checkpoint_response(checkpoint(&started, 0, 3, "visible", 40))
        .expect("writer checkpoint");

    let messages = reader
        .load_messages(&chat.id, None, 10)
        .expect("reader observes commit");
    assert_eq!(messages.messages[1].text, "visible");
    assert_eq!(
        reader
            .get_request_state(&started.request_state_id)
            .expect("reader request state")
            .last_durable_seq,
        3
    );
}

#[test]
fn second_store_is_rejected_without_interrupting_the_live_owner() {
    let (_directory, path) = database();
    let owner = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open owner");
    let chat = create_chat(&owner, 20);
    let started = begin_turn(&owner, &chat, "hello", 30);
    deliver_through(&owner, &started, 0, 1, 35);

    let error = lorepia_storage::Store::open_at(&path, timestamp(100))
        .expect_err("second store must not acquire the database lease");
    assert!(matches!(
        error,
        lorepia_storage::StorageError::Conflict {
            entity: "database lease"
        }
    ));
    let state = owner
        .get_request_state(&started.request_state_id)
        .expect("owner request remains available");
    assert_eq!(state.status, RequestStatus::Running);
    owner
        .checkpoint_response(checkpoint(&started, 0, 1, "still live", 110))
        .expect("owner can still checkpoint");
}

#[test]
fn concurrent_delivery_and_ack_compare_and_swap_have_one_winner() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);
    let barrier = Arc::new(Barrier::new(3));

    let make_delivery = || DeliveryCheckpoint {
        request_state_id: started.request_state_id.clone(),
        owner_label: started.owner_label.clone(),
        stream_generation: started.stream_generation.clone(),
        expected_last_delivered_seq: 0,
        through_seq: 1,
        at_ms: timestamp(40),
    };
    let first_store = store.clone();
    let first_barrier = Arc::clone(&barrier);
    let first_delivery = make_delivery();
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.record_response_delivery(first_delivery)
    });
    let second_store = store.clone();
    let second_barrier = Arc::clone(&barrier);
    let second_delivery = make_delivery();
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_store.record_response_delivery(second_delivery)
    });
    barrier.wait();
    let delivery_results = [
        first.join().expect("first delivery thread"),
        second.join().expect("second delivery thread"),
    ];
    assert_eq!(
        delivery_results
            .iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        delivery_results
            .iter()
            .filter(|result| result.is_err())
            .count(),
        1
    );

    deliver_through(&store, &started, 1, 2, 41);
    store
        .checkpoint_response(checkpoint(&started, 0, 2, "durable", 42))
        .expect("durable checkpoint");

    let barrier = Arc::new(Barrier::new(3));
    let make_ack = || CumulativeAck {
        request_state_id: started.request_state_id.clone(),
        owner_label: started.owner_label.clone(),
        stream_generation: started.stream_generation.clone(),
        expected_last_acked_seq: None,
        through_seq: 2,
        at_ms: timestamp(43),
    };
    let first_store = store.clone();
    let first_barrier = Arc::clone(&barrier);
    let first_ack = make_ack();
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.acknowledge_response(first_ack)
    });
    let second_store = store.clone();
    let second_barrier = Arc::clone(&barrier);
    let second_ack = make_ack();
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_store.acknowledge_response(second_ack)
    });
    barrier.wait();
    let ack_results = [
        first.join().expect("first ACK thread"),
        second.join().expect("second ACK thread"),
    ];
    assert_eq!(
        ack_results.iter().filter(|result| result.is_ok()).count(),
        1
    );
    assert_eq!(
        ack_results.iter().filter(|result| result.is_err()).count(),
        1
    );
    let state = store
        .get_request_state(&started.request_state_id)
        .expect("request state");
    assert_eq!(state.last_delivered_seq, 2);
    assert_eq!(state.last_durable_seq, 2);
    assert_eq!(state.last_acked_seq, Some(2));
}
